package com.brentph505.zenime

import java.io.BufferedInputStream
import java.io.BufferedOutputStream
import java.io.ByteArrayOutputStream
import java.io.InputStream
import java.net.InetAddress
import java.net.HttpURLConnection
import java.net.ServerSocket
import java.net.Socket
import java.net.URI
import java.net.URL
import java.net.URLDecoder
import java.net.URLEncoder
import java.security.SecureRandom
import java.util.concurrent.Executors
import java.util.regex.Pattern

/**
 * App-local HLS proxy. It binds only to loopback, injects upstream Referer/Origin
 * headers, forwards Range requests and rewrites nested playlists/keys/segments
 * so ExoPlayer can request every HLS resource through the same header-aware path.
 *
 * This is not a public/open proxy. Requests must carry the random per-activity token.
 */
internal object LocalHlsProxy {
    private const val USER_AGENT =
        "Mozilla/5.0 (Linux; Android 10) AppleWebKit/537.36 Chrome/124.0 Mobile Safari/537.36"
    private const val MAX_PLAYLIST_BYTES = 12 * 1024 * 1024
    private var server: ServerSocket? = null
    private var worker: Thread? = null
    private var secret: String = ""
    private val clients = Executors.newFixedThreadPool(4) { task ->
        Thread(task, "zenime-hls-request").apply { isDaemon = true }
    }

    @Synchronized
    fun start(): Boolean {
        if (server?.isClosed == false) return true
        return try {
            val random = ByteArray(24)
            SecureRandom().nextBytes(random)
            secret = random.joinToString("") { "%02x".format(it) }
            val socket = ServerSocket(0, 24, InetAddress.getByName("127.0.0.1"))
            server = socket
            worker = Thread({
                while (!socket.isClosed) {
                    try {
                        val client = socket.accept()
                        clients.execute { handle(client) }
                    } catch (_: Exception) {
                        if (!socket.isClosed) continue
                    }
                }
            }, "zenime-hls-proxy").apply {
                isDaemon = true
                start()
            }
            true
        } catch (_: Exception) {
            stop()
            false
        }
    }

    @Synchronized
    fun proxyUrl(target: String, referer: String): String {
        val port = server?.localPort ?: return target
        if (!isSafeTarget(target)) return target
        return "http://127.0.0.1:" + port + "/p?token=" + enc(secret) +
            "&url=" + enc(target) + "&referer=" + enc(referer)
    }

    @Synchronized
    fun stop() {
        try { server?.close() } catch (_: Exception) {}
        server = null
        worker = null
        secret = ""
    }

    private fun enc(value: String): String = URLEncoder.encode(value, "UTF-8")

    private fun decode(value: String): String = URLDecoder.decode(value, "UTF-8")

    private fun isSafeTarget(raw: String): Boolean {
        return try {
            val uri = URI(raw)
            val scheme = uri.scheme?.lowercase() ?: return false
            if (scheme != "https" && scheme != "http") return false
            val host = uri.host?.lowercase() ?: return false
            if (host == "localhost" || host.endsWith(".localhost") || host.endsWith(".local")) return false
            val addresses = InetAddress.getAllByName(host)
            addresses.isNotEmpty() && addresses.none {
                it.isAnyLocalAddress || it.isLoopbackAddress || it.isLinkLocalAddress ||
                    it.isSiteLocalAddress || it.isMulticastAddress
            }
        } catch (_: Exception) {
            false
        }
    }

    private fun queryMap(target: String): Map<String, String> {
        val result = mutableMapOf<String, String>()
        val query = target.substringAfter('?', "")
        for (part in query.split('&')) {
            if (part.isEmpty()) continue
            val key = part.substringBefore('=')
            val value = part.substringAfter('=', "")
            try { result[decode(key)] = decode(value) } catch (_: Exception) {}
        }
        return result
    }

    private fun handle(socket: Socket) {
        socket.use { client ->
            client.soTimeout = 20_000
            val input = BufferedInputStream(client.getInputStream())
            val output = BufferedOutputStream(client.getOutputStream())
            val requestLine = readLine(input) ?: return
            val pieces = requestLine.split(' ')
            if (pieces.size < 2) {
                writeResponse(output, 400, "text/plain", "Bad request".toByteArray())
                return
            }
            val method = pieces[0].uppercase()
            if (method != "GET" && method != "HEAD") {
                writeResponse(output, 405, "text/plain", "Method not allowed".toByteArray())
                return
            }
            val headers = mutableMapOf<String, String>()
            while (true) {
                val line = readLine(input) ?: break
                if (line.isEmpty()) break
                val colon = line.indexOf(':')
                if (colon > 0) headers[line.substring(0, colon).trim().lowercase()] = line.substring(colon + 1).trim()
            }
            val params = queryMap(pieces[1])
            if (params["token"].isNullOrEmpty() || params["token"] != secret) {
                writeResponse(output, 403, "text/plain", "Forbidden".toByteArray())
                return
            }
            val target = params["url"].orEmpty()
            val referer = params["referer"].orEmpty()
            if (!isSafeTarget(target)) {
                writeResponse(output, 400, "text/plain", "Unsupported upstream URL".toByteArray())
                return
            }
            var upstream: HttpURLConnection? = null
            try {
                upstream = URL(target).openConnection() as HttpURLConnection
                upstream.requestMethod = method
                upstream.instanceFollowRedirects = true
                upstream.connectTimeout = 12_000
                upstream.readTimeout = 25_000
                upstream.setRequestProperty("User-Agent", USER_AGENT)
                upstream.setRequestProperty("Accept", "*/*")
                upstream.setRequestProperty("Accept-Encoding", "identity")
                if (referer.isNotBlank()) {
                    upstream.setRequestProperty("Referer", referer)
                    try {
                        val origin = URI(referer)
                        if (!origin.scheme.isNullOrBlank() && !origin.host.isNullOrBlank()) {
                            upstream.setRequestProperty("Origin", origin.scheme + "://" + origin.host +
                                if (origin.port > 0) ":" + origin.port else "")
                        }
                    } catch (_: Exception) {}
                }
                headers["range"]?.let { upstream.setRequestProperty("Range", it) }
                val status = upstream.responseCode
                val contentType = upstream.contentType ?: "application/octet-stream"
                val isPlaylist = target.substringBefore('?').lowercase().endsWith(".m3u8") ||
                    contentType.contains("mpegurl", ignoreCase = true)
                val stream = if (status >= 400) upstream.errorStream else upstream.inputStream
                if (isPlaylist) {
                    val bytes = if (method == "HEAD" || stream == null) ByteArray(0) else readAll(stream, MAX_PLAYLIST_BYTES)
                    val body = if (method == "HEAD") ByteArray(0) else rewritePlaylist(String(bytes, Charsets.UTF_8), target, referer)
                    writeResponse(output, status, contentType, body)
                } else {
                    // Stream segments/keys progressively instead of buffering large video chunks in RAM.
                    val phrase = when (status) {
                        200 -> "OK"; 206 -> "Partial Content"; 400 -> "Bad Request"
                        403 -> "Forbidden"; 404 -> "Not Found"; 416 -> "Range Not Satisfiable"
                        else -> if (status >= 500) "Bad Gateway" else "Upstream"
                    }
                    val responseHeaders = StringBuilder()
                    responseHeaders.append("HTTP/1.1 ").append(status).append(' ').append(phrase).append("\r\n")
                    responseHeaders.append("Content-Type: ").append(contentType).append("\r\n")
                    upstream.getHeaderField("Content-Length")?.let { responseHeaders.append("Content-Length: ").append(it).append("\r\n") }
                    upstream.getHeaderField("Content-Range")?.let { responseHeaders.append("Content-Range: ").append(it).append("\r\n") }
                    upstream.getHeaderField("Accept-Ranges")?.let { responseHeaders.append("Accept-Ranges: ").append(it).append("\r\n") }
                    responseHeaders.append("Connection: close\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\n\r\n")
                    output.write(responseHeaders.toString().toByteArray(Charsets.US_ASCII))
                    output.flush()
                    if (method != "HEAD" && stream != null) {
                        stream.use { source ->
                            val buffer = ByteArray(32 * 1024)
                            while (true) {
                                val count = source.read(buffer)
                                if (count < 0) break
                                output.write(buffer, 0, count)
                                output.flush()
                            }
                        }
                    }
                }
            } catch (_: Exception) {
                try { writeResponse(output, 502, "text/plain", "Upstream media request failed".toByteArray()) } catch (_: Exception) {}
            } finally {
                upstream?.disconnect()
            }
        }
    }

    private fun rewritePlaylist(text: String, playlistUrl: String, referer: String): ByteArray {
        val uriPattern = Pattern.compile("URI=\"([^\"]+)\"")
        val matcher = uriPattern.matcher(text)
        val attrRewrite = StringBuffer()
        while (matcher.find()) {
            val original = matcher.group(1) ?: continue
            val changed = proxyResolved(original, playlistUrl, referer)
            matcher.appendReplacement(attrRewrite, "URI=\"" + MatcherQuote.quote(changed) + "\"")
        }
        matcher.appendTail(attrRewrite)
        val rewritten = attrRewrite.toString().lineSequence().joinToString("\n") { line ->
            val trimmed = line.trim()
            if (trimmed.isEmpty() || trimmed.startsWith("#") || trimmed.startsWith("data:")) line
            else {
                val resolved = proxyResolved(trimmed, playlistUrl, referer)
                if (line.startsWith(" ") || line.startsWith("\t")) line.takeWhile { it == ' ' || it == '\t' } + resolved else resolved
            }
        }
        return rewritten.toByteArray(Charsets.UTF_8)
    }

    private fun proxyResolved(raw: String, parent: String, referer: String): String {
        return try {
            if (raw.startsWith("data:", ignoreCase = true) || raw.startsWith("skd:", ignoreCase = true)) return raw
            val resolved = URI(parent).resolve(raw).toString()
            proxyUrl(resolved, referer)
        } catch (_: Exception) {
            raw
        }
    }

    private fun readAll(stream: InputStream, maximum: Int): ByteArray {
        stream.use {
            val out = ByteArrayOutputStream()
            val buffer = ByteArray(16 * 1024)
            while (true) {
                val count = it.read(buffer)
                if (count < 0) break
                if (out.size() + count > maximum) throw IllegalStateException("Playlist/response too large")
                out.write(buffer, 0, count)
            }
            return out.toByteArray()
        }
    }

    private fun readLine(input: InputStream): String? {
        val bytes = ByteArrayOutputStream()
        while (true) {
            val n = input.read()
            if (n < 0) return if (bytes.size() == 0) null else bytes.toString("UTF-8")
            if (n == 10) break
            if (n != 13) bytes.write(n)
            if (bytes.size() > 16 * 1024) return null
        }
        return bytes.toString("UTF-8")
    }

    private fun writeResponse(
        output: BufferedOutputStream,
        status: Int,
        contentType: String,
        body: ByteArray,
        contentRange: String? = null
    ) {
        val statusText = when (status) {
            200 -> "OK"
            206 -> "Partial Content"
            400 -> "Bad Request"
            403 -> "Forbidden"
            405 -> "Method Not Allowed"
            416 -> "Range Not Satisfiable"
            else -> if (status >= 500) "Bad Gateway" else "Upstream"
        }
        val headers = StringBuilder()
        headers.append("HTTP/1.1 ").append(status).append(' ').append(statusText).append("\r\n")
        headers.append("Content-Type: ").append(contentType).append("\r\n")
        headers.append("Content-Length: ").append(body.size).append("\r\n")
        headers.append("Connection: close\r\n")
        headers.append("Cache-Control: no-store\r\n")
        if (!contentRange.isNullOrBlank()) headers.append("Content-Range: ").append(contentRange).append("\r\n")
        headers.append("Access-Control-Allow-Origin: *\r\n\r\n")
        output.write(headers.toString().toByteArray(Charsets.US_ASCII))
        output.write(body)
        output.flush()
    }

    private object MatcherQuote {
        fun quote(value: String): String {
            return value.replace("\\", "\\\\").replace("$", "\\$").replace("\"", "\\\"")
        }
    }
}
