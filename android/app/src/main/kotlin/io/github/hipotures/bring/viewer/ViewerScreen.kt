package io.github.hipotures.bring.viewer

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.webkit.MimeTypeMap
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.core.content.FileProvider
import androidx.core.net.toUri
import coil3.compose.AsyncImage
import io.github.hipotures.bring.bring
import io.github.hipotures.bring.describe
import io.github.hipotures.bring.ffi.Entry
import io.github.hipotures.bring.formatBytes
import io.github.hipotures.bring.formatMtime
import io.github.hipotures.bring.media.MediaKind
import io.github.hipotures.bring.media.RingImage
import io.github.hipotures.bring.media.extensionOf
import io.github.hipotures.bring.media.kindOf
import io.github.hipotures.bring.sync.RouteStatus
import io.github.hipotures.bring.ui.BadgeChip
import io.github.hipotures.bring.ui.ErrorLine
import io.github.hipotures.bring.ui.KeyValue
import io.github.hipotures.bring.ui.Panel
import io.github.hipotures.bring.ui.SharpButton
import io.github.hipotures.bring.ui.badgesFor
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File

/** One file: media on top, details and actions below. */
@Composable
fun ViewerScreen(entryId: Long, route: RouteStatus) {
    val context = LocalContext.current
    var entry by remember { mutableStateOf<Entry?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(entryId) {
        try {
            entry = withContext(Dispatchers.IO) { context.bring.moon.entry(entryId) }
        } catch (e: Exception) {
            error = e.describe()
        }
    }
    val e = entry
    if (e == null) {
        error?.let { Box(Modifier.padding(12.dp)) { ErrorLine(it) } }
        return
    }
    val name = e.ringPath.substringAfterLast('/')
    val kind = remember(e) { kindOf(e.mime, name) }
    val local = remember(e) { e.localPath?.let(::File)?.takeIf { it.isFile } }
    val playable = local != null || e.remoteFileId != null

    Column(Modifier.fillMaxSize()) {
        when {
            kind == MediaKind.IMAGE && playable -> Box(Modifier.weight(1f).fillMaxWidth().background(Color.Black)) {
                ZoomableImage(local ?: RingImage(e.remoteFileId!!, e.blake3, e.size))
            }
            (kind == MediaKind.VIDEO || kind == MediaKind.AUDIO) && playable ->
                Box(Modifier.weight(1f).fillMaxWidth().background(Color.Black)) { MediaPlayer(e, local) }
            else -> {}
        }
        Details(e, name, kind, local, route, Modifier.then(if (kind == MediaKind.OTHER || !playable) Modifier.weight(1f) else Modifier))
    }
}

@Composable
private fun ZoomableImage(model: Any) {
    var scale by remember { mutableFloatStateOf(1f) }
    var offset by remember { mutableStateOf(Offset.Zero) }
    var loading by remember { mutableStateOf(true) }
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        AsyncImage(
            model = model,
            contentDescription = null,
            contentScale = ContentScale.Fit,
            onState = { loading = it is coil3.compose.AsyncImagePainter.State.Loading },
            modifier = Modifier
                .fillMaxSize()
                .pointerInput(Unit) {
                    detectTransformGestures { _, pan, zoom, _ ->
                        scale = (scale * zoom).coerceIn(1f, 8f)
                        offset = if (scale == 1f) Offset.Zero else offset + pan
                    }
                }
                .graphicsLayer {
                    scaleX = scale
                    scaleY = scale
                    translationX = offset.x
                    translationY = offset.y
                },
        )
        if (loading) CircularProgressIndicator()
    }
}

@Composable
private fun Details(
    e: Entry,
    name: String,
    kind: MediaKind,
    local: File?,
    route: RouteStatus,
    modifier: Modifier,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var message by remember { mutableStateOf<String?>(null) }
    Column(modifier.verticalScroll(rememberScrollState()).padding(12.dp)) {
        Panel(title = name) {
            FlowRow(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                badgesFor(e.state, e.uploadPending, e.lastError, e.suppressed).forEach { BadgeChip(it) }
            }
            KeyValue("ring path", e.ringPath)
            KeyValue("local", e.localPath ?: "— (on the Ring only)")
            KeyValue("size", "${formatBytes(e.size)} (${e.size} B)")
            KeyValue("modified", formatMtime(e.mtimeNs))
            KeyValue("type", e.mime ?: extensionOf(name).ifEmpty { "?" })
            e.blake3?.let { KeyValue("blake3", it.take(16) + "…") }
            e.lastError?.let { KeyValue("error", it) }
            if (e.suppressed) KeyValue("note", "Held: removed on the Ring, never uploaded again")
            FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                if (local != null) {
                    SharpButton("Open with…", onClick = { message = openLocal(context, local, e.mime) })
                }
                // External players cannot pin the LAN certificate (§3.1).
                val fileId = e.remoteFileId
                if (fileId != null && route == RouteStatus.WAN && kind != MediaKind.IMAGE) {
                    SharpButton("External player", onClick = {
                        scope.launch {
                            message = try {
                                val url = withContext(Dispatchers.IO) { context.bring.moon.streamUrl(fileId) }
                                openUrl(context, url, e.mime)
                            } catch (ex: Exception) {
                                ex.describe()
                            }
                        }
                    })
                }
            }
            message?.let { ErrorLine(it) }
        }
    }
}

private fun mimeFor(mime: String?, file: File): String =
    mime ?: MimeTypeMap.getSingleton().getMimeTypeFromExtension(file.extension.lowercase()) ?: "*/*"

/** ACTION_VIEW through the FileProvider; returns an error line or null. */
private fun openLocal(context: Context, file: File, mime: String?): String? {
    val uri = FileProvider.getUriForFile(context, "${context.packageName}.files", file)
    val intent = Intent(Intent.ACTION_VIEW)
        .setDataAndType(uri, mimeFor(mime, file))
        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    return launch(context, Intent.createChooser(intent, null))
}

private fun openUrl(context: Context, url: String, mime: String?): String? =
    launch(context, Intent(Intent.ACTION_VIEW).setDataAndType(url.toUri(), mime ?: "video/*"))

private fun launch(context: Context, intent: Intent): String? = try {
    context.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    null
} catch (_: ActivityNotFoundException) {
    "No app can open this file"
}
