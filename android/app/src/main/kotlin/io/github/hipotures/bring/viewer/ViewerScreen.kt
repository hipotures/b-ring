package io.github.hipotures.bring.viewer

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.webkit.MimeTypeMap
import androidx.compose.foundation.background
import androidx.compose.material3.Icon
import androidx.compose.material.icons.outlined.KeyboardArrowUp
import androidx.compose.material.icons.outlined.KeyboardArrowDown
import androidx.compose.material.icons.Icons
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.activity.compose.BackHandler
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.material3.Text
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.HorizontalDivider
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
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
import androidx.compose.ui.graphics.StrokeCap
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
import io.github.hipotures.bring.ui.SharpButton
import io.github.hipotures.bring.ui.badgesFor
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import themechanics.compose.Tm
import themechanics.compose.TmDimens
import java.io.File

/** One file: media fills the screen, a compact info bar below. */
@Composable
fun ViewerScreen(entryId: Long, route: RouteStatus, uploading: String? = null) {
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
        error?.let { Box(Modifier.padding(TmDimens.panelPad)) { ErrorLine(it) } }
        return
    }
    val name = e.ringPath.substringAfterLast('/')
    val kind = remember(e) { kindOf(e.mime, name) }
    val local = remember(e) { e.localPath?.let(::File)?.takeIf { it.isFile } }
    val playable = local != null || e.remoteFileId != null

    // Media fills the screen; for other files the details are the content.
    val media = playable && kind != MediaKind.OTHER
    var details by rememberSaveable(e.ringPath) { mutableStateOf(!media) }
    // Back closes the details first, then the file.
    BackHandler(enabled = media && details) { details = false }
    Column(Modifier.fillMaxSize()) {
        when {
            kind == MediaKind.IMAGE && playable -> Box(Modifier.weight(1f).fillMaxWidth().background(Color.Black)) {
                ZoomableImage(local ?: RingImage(e.remoteFileId!!, e.blake3, e.size), onDoubleTap = { details = !details })
            }
            (kind == MediaKind.VIDEO || kind == MediaKind.AUDIO) && playable ->
                Box(Modifier.weight(1f).fillMaxWidth().background(Color.Black)) { MediaPlayer(e, local) }
            else -> {}
        }
        InfoBar(e, name, kind, local, route, media, details, { details = it }, e.ringPath == uploading, if (media) Modifier else Modifier.weight(1f))
    }
}

@Composable
/** Pinch to zoom; double tap shows or hides the details (and resets the zoom). */
private fun ZoomableImage(model: Any, onDoubleTap: () -> Unit) {
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
                    detectTapGestures(onDoubleTap = {
                        scale = 1f
                        offset = Offset.Zero
                        onDoubleTap()
                    })
                }
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
        if (loading) CircularProgressIndicator(color = Tm.colors.accent, trackColor = Tm.colors.line3, strokeCap = StrokeCap.Butt)
    }
}

/**
 * Name (one line, cut at the end) and "Open with…"; state, size and date;
 * full paths and hash, selectable and copyable, only when asked for.
 */
@Composable
private fun InfoBar(
    e: Entry,
    name: String,
    kind: MediaKind,
    local: File?,
    route: RouteStatus,
    media: Boolean,
    open: Boolean,
    onOpen: (Boolean) -> Unit,
    uploading: Boolean,
    modifier: Modifier,
) {
    val c = Tm.colors
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var message by remember { mutableStateOf<String?>(null) }
    Column(modifier.fillMaxWidth().background(c.surface)) {
        HorizontalDivider(thickness = TmDimens.bw2, color = c.line2)
        Row(
            Modifier
                .fillMaxWidth()
                .heightIn(min = 48.dp)
                .padding(start = TmDimens.regionPad, end = TmDimens.space3, top = TmDimens.space1),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(TmDimens.space2),
        ) {
            Text(
                name,
                style = MaterialTheme.typography.titleSmall,
                color = c.text,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f),
            )
            if (local != null) SharpButton("Open with…", onClick = { message = openLocal(context, local, e.mime) })
            badgesFor(e.state, e.uploadPending, e.lastError, e.suppressed, uploading).forEach { BadgeChip(it) }
        }
        Row(
            Modifier
                .fillMaxWidth()
                // The whole line toggles the details; the chevron shows which way.
                .clickable(enabled = media) { onOpen(!open) }
                .heightIn(min = 48.dp)
                .padding(start = TmDimens.regionPad, end = if (media) 0.dp else TmDimens.space3),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(TmDimens.space2),
        ) {
            Text(
                "${formatBytes(e.size)} · ${formatMtime(e.mtimeNs)}",
                style = Tm.text.numeric,
                color = c.text2,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f),
            )
            if (media) {
                Box(Modifier.size(48.dp), contentAlignment = Alignment.Center) {
                    Icon(
                        if (open) Icons.Outlined.KeyboardArrowDown else Icons.Outlined.KeyboardArrowUp,
                        contentDescription = if (open) "Hide details" else "Show details",
                        tint = c.link,
                        modifier = Modifier.size(TmDimens.iconLg),
                    )
                }
            }
        }
        if (open) {
            HorizontalDivider(thickness = TmDimens.bw3, color = c.line3)
            SelectionContainer(
                Modifier
                    .then(if (media) Modifier.heightIn(max = 320.dp) else Modifier.weight(1f))
                    .verticalScroll(rememberScrollState()),
            ) {
                Column(
                    Modifier.padding(horizontal = TmDimens.regionPad, vertical = TmDimens.space3),
                    verticalArrangement = Arrangement.spacedBy(TmDimens.space2),
                ) {
                    KeyValue("On the Ring", e.ringPath, copy = true)
                    KeyValue("On this device", e.localPath ?: "— (on the Ring only)", copy = e.localPath != null)
                    KeyValue("Size", "${formatBytes(e.size)} (${e.size} B)")
                    KeyValue("Modified", formatMtime(e.mtimeNs))
                    KeyValue("Type", e.mime ?: extensionOf(name).ifEmpty { "?" })
                    e.blake3?.let { KeyValue("BLAKE3", it, copy = true) }
                    e.lastError?.let { KeyValue("Error", it) }
                    if (e.suppressed) KeyValue("Note", "Held: removed on the Ring, never uploaded again")
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
            }
        }
        message?.let { Box(Modifier.padding(horizontal = TmDimens.regionPad, vertical = TmDimens.space2)) { ErrorLine(it) } }
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
