package com.brentph505.zenime

import android.app.Activity
import android.graphics.Color
import android.net.Uri
import android.os.Bundle
import android.view.Gravity
import android.view.View
import android.view.WindowManager
import android.widget.FrameLayout
import android.widget.TextView
import android.widget.Toast
import androidx.media3.common.MediaItem
import androidx.media3.common.MimeTypes
import androidx.media3.common.Player
import androidx.media3.datasource.DefaultHttpDataSource
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import androidx.media3.ui.AspectRatioFrameLayout
import androidx.media3.ui.PlayerView

class PlayerActivity : Activity() {
    private var player: ExoPlayer? = null
    private var playerView: PlayerView? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        window.statusBarColor = Color.BLACK
        window.navigationBarColor = Color.BLACK

        val mediaUrl = intent.getStringExtra("media_url")
        val mediaTitle = intent.getStringExtra("media_title") ?: "Now playing"
        val referer = intent.getStringExtra("media_referer").orEmpty()
        val nextMediaUrl = intent.getStringExtra("next_media_url").orEmpty()
        val prefs = getSharedPreferences("zenime_native", MODE_PRIVATE)
        val autoPlay = prefs.getString("auto_play", "true") != "false"
        val showSkipIntro = prefs.getString("skip_intro", "false") == "true"

        if (mediaUrl.isNullOrBlank() || !(mediaUrl.startsWith("https://") || mediaUrl.startsWith("http://"))) {
            Toast.makeText(this, "This episode has no playable direct stream.", Toast.LENGTH_LONG).show()
            finish()
            return
        }

        val useLocalProxy = LocalHlsProxy.start()
        val playbackUrl = if (useLocalProxy && mediaUrl.substringBefore('?').lowercase().endsWith(".m3u8")) {
            LocalHlsProxy.proxyUrl(mediaUrl, referer)
        } else mediaUrl

        val dataSourceFactory = DefaultHttpDataSource.Factory()
            .setUserAgent("Mozilla/5.0 (Linux; Android 10) AppleWebKit/537.36 Chrome/120 Mobile Safari/537.36")
            .setAllowCrossProtocolRedirects(true)
            .setDefaultRequestProperties(
                if (referer.isNotBlank()) mapOf("Referer" to referer) else emptyMap()
            )

        val mediaSourceFactory = DefaultMediaSourceFactory(dataSourceFactory)
        player = ExoPlayer.Builder(this)
            .setMediaSourceFactory(mediaSourceFactory)
            .build()

        val videoView = PlayerView(this).apply {
            player = this@PlayerActivity.player
            useController = true
            resizeMode = AspectRatioFrameLayout.RESIZE_MODE_FIT
            setShowBuffering(PlayerView.SHOW_BUFFERING_WHEN_PLAYING)
            setKeepContentOnPlayerReset(true)
        }
        playerView = videoView

        val root = FrameLayout(this).apply {
            setBackgroundColor(Color.BLACK)
            addView(
                videoView,
                FrameLayout.LayoutParams(
                    FrameLayout.LayoutParams.MATCH_PARENT,
                    FrameLayout.LayoutParams.MATCH_PARENT
                )
            )
        }

        val topBar = TextView(this).apply {
            text = "‹   " + mediaTitle
            setTextColor(Color.WHITE)
            textSize = 14f
            gravity = Gravity.CENTER_VERTICAL
            setPadding(dp(16), 0, dp(16), 0)
            setBackgroundColor(0x99000000.toInt())
            maxLines = 1
            ellipsize = android.text.TextUtils.TruncateAt.END
            setOnClickListener { finish() }
        }
        root.addView(
            topBar,
            FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT,
                dp(52),
                Gravity.TOP
            )
        )

        if (showSkipIntro) {
            val skipButton = TextView(this).apply {
                text = "Skip intro  ›"
                setTextColor(Color.WHITE)
                textSize = 13f
                gravity = Gravity.CENTER
                setPadding(dp(14), 0, dp(14), 0)
                setBackgroundColor(0xCC51418C.toInt())
                setOnClickListener {
                    val duration = this@PlayerActivity.player?.duration ?: 0L
                    val target = (this@PlayerActivity.player?.currentPosition ?: 0L) + 85_000L
                    this@PlayerActivity.player?.seekTo(if (duration > 0L) minOf(target, duration) else target)
                }
            }
            root.addView(
                skipButton,
                FrameLayout.LayoutParams(
                    FrameLayout.LayoutParams.WRAP_CONTENT,
                    dp(42),
                    Gravity.BOTTOM or Gravity.END
                ).apply { setMargins(0, 0, dp(16), dp(72)) }
            )
        }

        setContentView(root)

        val itemBuilder = MediaItem.Builder().setUri(Uri.parse(playbackUrl))
        val lower = mediaUrl.substringBefore('?').lowercase()
        if (lower.endsWith(".m3u8")) {
            itemBuilder.setMimeType(MimeTypes.APPLICATION_M3U8)
        } else if (lower.endsWith(".mpd")) {
            itemBuilder.setMimeType(MimeTypes.APPLICATION_MPD)
        }

        player?.apply {
            val items = mutableListOf(itemBuilder.build())
            if (nextMediaUrl.startsWith("https://") || nextMediaUrl.startsWith("http://")) {
                items.add(MediaItem.fromUri(nextMediaUrl))
            }
            setMediaItems(items)
            playWhenReady = autoPlay
            prepare()
            addListener(object : Player.Listener {
                override fun onPlayerError(error: androidx.media3.common.PlaybackException) {
                    Toast.makeText(
                        this@PlayerActivity,
                        "Playback failed for this source. Try another episode/provider.",
                        Toast.LENGTH_LONG
                    ).show()
                }
            })
        }

        hideSystemBars()
    }

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

    private fun dp(value: Int): Int =
        (value * resources.displayMetrics.density + 0.5f).toInt()

    override fun onStop() {
        super.onStop()
        player?.release()
        player = null
        LocalHlsProxy.stop()
        playerView?.player = null
        playerView = null
    }
}
