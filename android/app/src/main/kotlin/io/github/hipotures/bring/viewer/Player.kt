package io.github.hipotures.bring.viewer

import android.content.Context
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
import androidx.media3.ui.PlayerView
import io.github.hipotures.bring.bring
import io.github.hipotures.bring.describe
import io.github.hipotures.bring.ffi.Entry
import io.github.hipotures.bring.net.PinnedTls
import io.github.hipotures.bring.ui.ErrorLine
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
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

    error?.let { ErrorLine(it) }
    AndroidView(
        factory = { PlayerView(it).apply { setShowNextButton(false); setShowPreviousButton(false) } },
        update = { it.player = player },
        modifier = modifier.fillMaxSize(),
    )
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
