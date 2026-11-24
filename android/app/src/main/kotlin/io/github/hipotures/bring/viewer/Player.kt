package io.github.hipotures.bring.viewer

import android.content.Context
import themechanics.compose.TmDimens
import androidx.media3.exoplayer.ExoPlaybackException
import androidx.media3.common.Player
import androidx.media3.common.PlaybackException
import androidx.compose.ui.Alignment
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.background
import android.net.Uri
import androidx.annotation.OptIn
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.net.toUri
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import androidx.media3.common.MediaItem
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.DataSource
import androidx.media3.datasource.DefaultDataSource
import androidx.media3.datasource.okhttp.OkHttpDataSource
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import androidx.media3.ui.DefaultTimeBar
import androidx.media3.ui.PlayerView
import io.github.hipotures.bring.bring
import io.github.hipotures.bring.describe
import io.github.hipotures.bring.ffi.Entry
import io.github.hipotures.bring.net.PinnedTls
import io.github.hipotures.bring.ui.ErrorLine
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import themechanics.compose.Tm
import java.io.File

/**
 * Media3 playback with scrubbing. A local copy plays from disk; a Ring file
 * streams over OkHttp pinned to the Ring's certificate on LAN (§3.1), with
 * the bearer token sent as a default request header.
 */
@OptIn(UnstableApi::class)
@Composable
fun MediaPlayer(entry: Entry, local: File?, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    var player by remember { mutableStateOf<ExoPlayer?>(null) }
    var error by remember { mutableStateOf<String?>(null) }

    LaunchedEffect(entry.id, local) {
        try {
            val (factory, uri) = withContext(Dispatchers.IO) { source(context, entry, local) }
            player = ExoPlayer.Builder(context)
                .setMediaSourceFactory(DefaultMediaSourceFactory(factory))
                .build()
                .apply {
                    addListener(object : Player.Listener {
                        override fun onPlayerError(e: PlaybackException) {
                            error = playbackError(e)
                        }
                    })
                    setMediaItem(MediaItem.fromUri(uri))
                    prepare()
                    playWhenReady = true
                }
        } catch (e: Exception) {
            error = e.describe()
        }
    }
    DisposableEffect(player) {
        val p = player
        onDispose { p?.release() }
    }
    LifecycleEventEffect(Lifecycle.Event.ON_STOP) { player?.pause() }

    // Media3's controller is a View: only the time bar takes the theme's colors.
    val c = Tm.colors
    AndroidView(
        factory = {
            PlayerView(it).apply {
                setShowNextButton(false)
                setShowPreviousButton(false)
                findViewById<DefaultTimeBar>(androidx.media3.ui.R.id.exo_progress)?.apply {
                    setPlayedColor(c.accent.toArgb())
                    setScrubberColor(c.accent.toArgb())
                    setBufferedColor(c.line2.toArgb())
                    setUnplayedColor(c.line3.toArgb())
                }
            }
        },
        update = { it.player = player },
        modifier = modifier.fillMaxSize(),
    )
    // On top of the player: a failure must not look like "nothing happens".
    error?.let {
        Box(Modifier.fillMaxSize().padding(TmDimens.regionPad), contentAlignment = Alignment.Center) {
            Box(Modifier.background(c.surface).padding(TmDimens.space3)) { ErrorLine(it) }
        }
    }
}

/** What went wrong, in words; names the video format when the phone cannot decode it. */
@OptIn(UnstableApi::class)
private fun playbackError(e: PlaybackException): String {
    val f = (e as? ExoPlaybackException)?.takeIf { it.type == ExoPlaybackException.TYPE_RENDERER }?.rendererFormat
    val what = f?.let {
        listOfNotNull(codecName(it.codecs, it.sampleMimeType), if (it.width > 0) "${it.width}×${it.height}" else null)
            .joinToString(", ")
    }
    return when (e.errorCode) {
        // A hardware decoder may accept a format it cannot decode (e.g. 10-bit
        // H.264) and fail on the first frames: that is DECODING_FAILED.
        PlaybackException.ERROR_CODE_DECODING_FORMAT_UNSUPPORTED,
        PlaybackException.ERROR_CODE_DECODING_FORMAT_EXCEEDS_CAPABILITIES,
        PlaybackException.ERROR_CODE_DECODER_INIT_FAILED,
        PlaybackException.ERROR_CODE_DECODER_QUERY_FAILED,
        PlaybackException.ERROR_CODE_DECODING_FAILED,
        -> "This phone cannot decode this video" + (what?.let { " ($it)" } ?: "") + "."
        else -> "Cannot play: ${e.errorCodeName}" + (e.cause?.message?.let { " — $it" } ?: "")
    }
}

/** RFC 6381 codec string to a name people know; H.264 profiles by profile_idc. */
private fun codecName(codecs: String?, mime: String?): String? {
    val c = codecs ?: return mime
    if (c.startsWith("avc1.") && c.length >= 7) {
        val profile = when (c.substring(5, 7).uppercase()) {
            "42" -> "Baseline"
            "4D" -> "Main"
            "64" -> "High"
            "6E" -> "High 10, 10-bit"
            "7A" -> "High 4:2:2"
            "F4" -> "High 4:4:4"
            else -> null
        }
        return "H.264" + (profile?.let { " $it" } ?: "")
    }
    return c
}

@OptIn(UnstableApi::class)
private fun source(context: Context, entry: Entry, local: File?): Pair<DataSource.Factory, Uri> {
    if (local != null) return DefaultDataSource.Factory(context) to Uri.fromFile(local)
    val fileId = entry.remoteFileId ?: error("this file is not on the Ring")
    val ms = context.bring.moon.mediaSource(fileId)
    val http = OkHttpDataSource.Factory(PinnedTls.client(ms.tlsCertSha256))
        .setDefaultRequestProperties(mapOf("Authorization" to ms.authorization))
    return http to ms.url.toUri()
}
