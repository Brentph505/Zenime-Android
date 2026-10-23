package com.brentph505.zenime

import android.app.NativeActivity
import android.content.Intent
import android.net.Uri
import android.os.Bundle

/** NativeActivity host for Slint, including deep-link delivery for future OAuth callbacks. */
class MainActivity : NativeActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        handleCallback(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleCallback(intent)
    }

    private fun handleCallback(intent: Intent?) {
        val uri: Uri = intent?.data ?: return
        if (uri.scheme != "zenime" || uri.host != "callback") return
        val fragment = uri.fragment.orEmpty()
        val values = fragment.split('&').mapNotNull { pair ->
            val pieces = pair.split('=', limit = 2)
            if (pieces.size == 2) pieces[0] to Uri.decode(pieces[1]) else null
        }.toMap()
        val token = values["access_token"] ?: return
        if (token.isNotBlank()) {
            getSharedPreferences("zenime_native", MODE_PRIVATE)
                .edit().putString("anilist_access_token", token).apply()
        }
    }
}
