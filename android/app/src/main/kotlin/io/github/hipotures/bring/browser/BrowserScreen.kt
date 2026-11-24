package io.github.hipotures.bring.browser

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import coil3.compose.AsyncImage
import io.github.hipotures.bring.ffi.Entry
import io.github.hipotures.bring.ffi.EntryState
import io.github.hipotures.bring.formatBytes
import io.github.hipotures.bring.formatMtime
import io.github.hipotures.bring.media.MediaKind
import io.github.hipotures.bring.media.RingThumb
import io.github.hipotures.bring.media.extensionOf
import io.github.hipotures.bring.media.kindOf
import io.github.hipotures.bring.ui.BadgeChip
import io.github.hipotures.bring.ui.FolderGlyph
import io.github.hipotures.bring.ui.RowDivider
import io.github.hipotures.bring.ui.Segmented
import io.github.hipotures.bring.ui.badgesFor
import themechanics.compose.Tm
import themechanics.compose.TmDimens
import java.io.File

/** Which entries the list shows. */
enum class Filter(val label: String, val state: EntryState?) {
    ALL("All", null),
    LOCAL("Local", EntryState.LOCAL_ONLY),
    SYNCED("Synced", EntryState.SYNCED),
    REMOTE("Remote", EntryState.REMOTE_ONLY),
}

/** Newest first, then by path for a stable order. */
private val newestFirst = compareByDescending<Entry> { it.mtimeNs }.thenBy { it.ringPath }

@Composable
fun BrowserScreen(
    entries: List<Entry>,
    dir: String,
    onDir: (String) -> Unit,
    onOpen: (Entry) -> Unit,
    header: @Composable () -> Unit,
    /** Ring path of the file being uploaded right now, if any. */
    uploading: String? = null,
) {
    var filter by rememberSaveable { mutableStateOf(Filter.ALL) }
    // Counts cover this folder and its subfolders, like the list below.
    val here = remember(entries, dir) {
        if (dir.isEmpty()) entries else entries.filter { it.ringPath.startsWith("$dir/") }
    }
    val counts = remember(here) { here.groupingBy { it.state }.eachCount() }
    val shown = remember(entries, filter) {
        if (filter.state == null) entries else entries.filter { it.state == filter.state }
    }
    val rows = remember(shown, dir) {
        listDir(shown, dir, { it.ringPath }, { it.size.toLong() }, newestFirst)
    }

    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = TmDimens.space6)) {
        item { Box(Modifier.padding(horizontal = TmDimens.panelPad, vertical = TmDimens.space2)) { header() } }
        item { Breadcrumbs(dir, onDir) }
        item {
            Segmented(
                options = Filter.entries,
                selected = filter,
                onSelect = { filter = it },
                label = { it.label },
                sub = { f -> "${if (f.state == null) here.size else counts[f.state] ?: 0}" },
                modifier = Modifier.padding(horizontal = TmDimens.panelPad, vertical = TmDimens.space2),
            )
            HorizontalDivider(thickness = TmDimens.bw2, color = Tm.colors.line2)
        }
        if (rows.isEmpty()) {
            item {
                Text(
                    when {
                        entries.isEmpty() -> "Nothing here yet. Files appear as they are synced."
                        filter.state != null -> "No ${filter.label.lowercase()} files in this folder."
                        else -> "No files match."
                    },
                    style = MaterialTheme.typography.bodyMedium,
                    color = Tm.colors.text2,
                    modifier = Modifier.padding(horizontal = TmDimens.regionPad, vertical = TmDimens.space6),
                )
            }
        }
        items(rows, key = { row ->
            when (row) {
                is DirRow.Folder -> "d:${row.path}"
                is DirRow.File -> "f:${row.item.id}"
            }
        }) { row ->
            when (row) {
                is DirRow.Folder -> FolderRow(row) { onDir(row.path) }
                is DirRow.File -> FileRow(row.item, row.name, row.item.ringPath == uploading) { onOpen(row.item) }
            }
            RowDivider()
        }
    }
}

@Composable
private fun Breadcrumbs(dir: String, onDir: (String) -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .horizontalScroll(rememberScrollState())
            .padding(horizontal = TmDimens.space1),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Crumb("Ring", current = dir.isEmpty()) { onDir("") }
        crumbs(dir).forEach { (name, path) ->
            Text("/", style = Tm.text.code, color = Tm.colors.text3)
            Crumb(name, current = path == dir) { onDir(path) }
        }
    }
}

@Composable
private fun Crumb(text: String, current: Boolean, onClick: () -> Unit) {
    Text(
        text,
        style = MaterialTheme.typography.bodyMedium,
        fontWeight = if (current) FontWeight.SemiBold else FontWeight.Normal,
        color = if (current) Tm.colors.text else Tm.colors.link,
        modifier = Modifier
            .clickable(enabled = !current, onClick = onClick)
            .padding(vertical = 14.dp, horizontal = TmDimens.space2),
    )
}

/** Record row on `surface`: 56 dp media square, name (wraps to two lines), mono meta. */
@Composable
private fun RecordRow(onClick: () -> Unit, leading: @Composable () -> Unit, content: @Composable () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .background(Tm.colors.surface)
            .clickable(onClick = onClick)
            .padding(horizontal = TmDimens.regionPad, vertical = TmDimens.space2),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        leading()
        Spacer(Modifier.width(TmDimens.space3))
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(TmDimens.space0_5)) { content() }
    }
}

@Composable
private fun FolderRow(row: DirRow.Folder, onClick: () -> Unit) {
    RecordRow(
        onClick,
        leading = {
            MediaSquare {
                Icon(FolderGlyph, null, Modifier.size(TmDimens.iconLg), tint = Tm.colors.text2)
            }
        },
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(TmDimens.space0_5)) {
                Name(row.name)
                Text("${row.files} files · ${formatBytes(row.bytes)}", style = Tm.text.numeric, color = Tm.colors.text2)
            }
            Icon(
                Icons.AutoMirrored.Outlined.KeyboardArrowRight,
                null,
                Modifier.size(TmDimens.iconLg),
                tint = Tm.colors.text3,
            )
        }
    }
}

@Composable
private fun FileRow(entry: Entry, name: String, uploading: Boolean, onClick: () -> Unit) {
    RecordRow(onClick, leading = { Thumb(entry, name) }) {
        Name(name)
        Text(
            "${formatBytes(entry.size)} · ${formatMtime(entry.mtimeNs)}",
            style = Tm.text.numeric,
            color = Tm.colors.text2,
            maxLines = 1,
        )
        FlowRow(
            Modifier.padding(top = TmDimens.space0_5),
            horizontalArrangement = Arrangement.spacedBy(TmDimens.space1),
            verticalArrangement = Arrangement.spacedBy(TmDimens.space0_5),
        ) {
            badgesFor(entry.state, entry.uploadPending, entry.lastError, entry.suppressed, uploading).forEach { BadgeChip(it) }
        }
    }
}

/** Long names wrap to a second line instead of being cut off sideways. */
@Composable
private fun Name(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.bodyMedium,
        fontWeight = FontWeight.Medium,
        color = Tm.colors.text,
        maxLines = 1,
        overflow = TextOverflow.Ellipsis,
    )
}

/** 56 dp square on `sunken` with a `line3` frame: thumbnails and placeholders. */
@Composable
private fun MediaSquare(content: @Composable BoxScope.() -> Unit) {
    Box(
        Modifier
            .size(56.dp)
            .border(TmDimens.bw3, Tm.colors.line3, RectangleShape)
            .background(Tm.colors.sunken),
        contentAlignment = Alignment.Center,
    ) { content() }
}

/** The Ring's thumbnail, the local image, or the extension on `sunken`. */
@Composable
fun Thumb(entry: Entry, name: String) {
    val kind = remember(entry.mime, name) { kindOf(entry.mime, name) }
    val model: Any? = remember(entry.thumb, entry.localPath, kind) {
        when {
            entry.thumb != null -> RingThumb(entry.thumb!!)
            entry.localPath != null && kind == MediaKind.IMAGE -> File(entry.localPath!!)
            else -> null
        }
    }
    MediaSquare {
        // The label stays visible underneath while loading or on failure.
        Text(extensionOf(name).take(4).uppercase().ifEmpty { "FILE" }, style = Tm.text.badge, color = Tm.colors.text2)
        if (model != null) {
            AsyncImage(model, null, Modifier.fillMaxSize(), contentScale = ContentScale.Crop)
        }
        if (kind == MediaKind.VIDEO) {
            Text(
                "▶",
                style = Tm.text.badge,
                color = Tm.colors.accentText,
                modifier = Modifier
                    .align(Alignment.BottomStart)
                    .background(Tm.colors.accent)
                    .padding(horizontal = TmDimens.space1),
            )
        }
    }
}
