package com.brentph505.zenime

import android.app.Activity
import android.graphics.Color
import android.net.Uri
import android.os.Bundle
import android.view.Gravity
import android.view.View
import android.view.WindowManager
import android.webkit.CookieManager
import android.webkit.SafeBrowsingResponse
import android.webkit.SslErrorHandler
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebResourceError
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import android.webkit.SslError
import android.widget.FrameLayout
import android.widget.TextView
import android.widget.Toast

/**
 * Native equivalent of Zenime Web's embedded-player iframe.
 * Provider embeds need a browser engine for their JavaScript and player scripts;
 * no application-hosted page or public iframe proxy is required.
 */
class IframePlayerActivity : Activity() {
    private var webView: WebView? = null
    private var customView: View? = null
    private var customViewCallback: WebChromeClient.CustomViewCallback? = null
    private lateinit var root: FrameLayout
    private lateinit var toolbar: View

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        window.statusBarColor = Color.BLACK
        window.navigationBarColor = Color.BLACK

        val rawUrl = intent.getStringExtra("embed_url").orEmpty()
        val title = intent.getStringExtra("media_title") ?: "Embedded player"
        val referer = intent.getStringExtra("media_referer").orEmpty()
        val prefs = getSharedPreferences("zenime_native", MODE_PRIVATE)
        val autoPlay = prefs.getString("auto_play", "true") != "false"
        val autoSkip = prefs.getString("skip_intro", "false") == "true"
        val embedUrl = buildProviderEmbedUrl(rawUrl, autoPlay, autoSkip)

        if (!isHttpUrl(embedUrl)) {
            Toast.makeText(this, "This provider returned an invalid embed URL.", Toast.LENGTH_LONG).show()
            finish()
            return
        }

        root = FrameLayout(this).apply { setBackgroundColor(Color.BLACK) }
        toolbar = TextView(this).apply {
            text = "‹   " + title
            setTextColor(Color.WHITE)
            textSize = 14f
            gravity = Gravity.CENTER_VERTICAL
            setPadding(dp(16), 0, dp(16), 0)
            setBackgroundColor(0xDD101016.toInt())
            maxLines = 1
            ellipsize = android.text.TextUtils.TruncateAt.END
            setOnClickListener { finish() }
        }
        root.addView(toolbar, FrameLayout.LayoutParams(
            FrameLayout.LayoutParams.MATCH_PARENT, dp(52), Gravity.TOP
        ))

        CookieManager.getInstance().setAcceptCookie(true)
        val browser = WebView(this)
        webView = browser
        browser.setBackgroundColor(Color.BLACK)
        browser.settings.apply {
            javaScriptEnabled = true
            domStorageEnabled = true
            databaseEnabled = true
            mediaPlaybackRequiresUserGesture = !autoPlay
            javaScriptCanOpenWindowsAutomatically = false
            setSupportMultipleWindows(false)
            allowFileAccess = false
            allowContentAccess = false
            loadsImagesAutomatically = true
            mixedContentMode = WebSettings.MIXED_CONTENT_NEVER_ALLOW
            if (android.os.Build.VERSION.SDK_INT >= 26) safeBrowsingEnabled = true
        }
        CookieManager.getInstance().setAcceptThirdPartyCookies(browser, true)
        browser.webViewClient = object : WebViewClient() {
            override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                val scheme = request.url.scheme?.lowercase()
                return scheme != "http" && scheme != "https"
            }
            override fun onReceivedSslError(view: WebView, handler: SslErrorHandler, error: SslError) {
                handler.cancel()
            }
            override fun onReceivedError(view: WebView, request: WebResourceRequest, error: WebResourceError) {
                if (request.isForMainFrame) {
                    Toast.makeText(this@IframePlayerActivity, "The provider embed could not load.", Toast.LENGTH_LONG).show()
                }
            }
        }
        browser.webChromeClient = object : WebChromeClient() {
            override fun onShowCustomView(view: View, callback: CustomViewCallback) {
                if (customView != null) {
                    callback.onCustomViewHidden()
                    return
                }
                customView = view
                customViewCallback = callback
                root.removeView(browser)
                root.removeView(toolbar)
                root.addView(view, FrameLayout.LayoutParams(
                    FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT
                ))
                hideSystemBars()
            }

            override fun onHideCustomView() {
                val view = customView ?: return
                root.removeView(view)
                customView = null
                customViewCallback?.onCustomViewHidden()
                customViewCallback = null
                root.addView(browser, FrameLayout.LayoutParams(
                    FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT
                ).apply { topMargin = dp(52) })
                root.addView(toolbar, FrameLayout.LayoutParams(
                    FrameLayout.LayoutParams.MATCH_PARENT, dp(52), Gravity.TOP
                ))
            }
        }
        root.addView(browser, FrameLayout.LayoutParams(
            FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT
        ).apply { topMargin = dp(52) })
        setContentView(root)

        val headers = if (referer.isNotBlank()) mapOf("Referer" to referer) else emptyMap()
        browser.loadUrl(embedUrl, headers)
        hideSystemBars()
    }

    private fun buildProviderEmbedUrl(raw: String, autoPlay: Boolean, autoSkip: Boolean): String {
        return try {
            val uri = Uri.parse(raw)
            val host = uri.host.orEmpty().lowercase()
            val builder = uri.buildUpon().clearQuery()
            for (pair in uri.queryParameterNames) {
                if (pair !in listOf("autoplay", "autoPlay", "skI", "skO")) {
                    for (value in uri.getQueryParameters(pair)) builder.appendQueryParameter(pair, value)
                }
            }
            if (host.contains("flixcloud")) {
                builder.appendQueryParameter("autoPlay", if (autoPlay) "true" else "false")
                builder.appendQueryParameter("skI", if (autoSkip) "true" else "false")
                builder.appendQueryParameter("skO", if (autoSkip) "true" else "false")
            } else if (host.contains("zanix") || host.contains("4animo")) {
                builder.appendQueryParameter("autoPlay", if (autoPlay) "1" else "0")
                builder.appendQueryParameter("skI", if (autoSkip) "1" else "0")
                builder.appendQueryParameter("skO", if (autoSkip) "1" else "0")
            } else if (autoPlay) {
                builder.appendQueryParameter("autoplay", "1")
            }
            builder.build().toString()
        } catch (_: Exception) {
            raw
        }
    }

    private fun isHttpUrl(url: String): Boolean =
        try { val uri = Uri.parse(url); (uri.scheme == "https" || uri.scheme == "http") && !uri.host.isNullOrBlank() }
        catch (_: Exception) { false }

    private fun hideSystemBars() {
        @Suppress("DEPRECATION")
        window.decorView.systemUiVisibility =
            (View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
                or View.SYSTEM_UI_FLAG_FULLSCREEN
                or View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                or View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                or View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
                or View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN)
    }

    private fun dp(value: Int): Int = (value * resources.displayMetrics.density + 0.5f).toInt()

    @Deprecated("Deprecated in Java")
    override fun onBackPressed() {
        val view = webView
        if (customView != null) {
            webChromeClientHideFullScreen()
        } else if (view?.canGoBack() == true) {
            view.goBack()
        } else {
            finish()
        }
    }

    private fun webChromeClientHideFullScreen() {
        customViewCallback?.onCustomViewHidden()
        customViewCallback = null
        customView?.let { root.removeView(it) }
        customView = null
        webView?.let { view ->
            root.addView(view, FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT
            ).apply { topMargin = dp(52) })
        }
        root.addView(toolbar, FrameLayout.LayoutParams(
            FrameLayout.LayoutParams.MATCH_PARENT, dp(52), Gravity.TOP
        ))
    }

    override fun onDestroy() {
        webView?.apply {
            stopLoading()
            loadUrl("about:blank")
            clearHistory()
            removeAllViews()
            destroy()
        }
        webView = null
        super.onDestroy()
    }
}
