package io.github.hipotures.bring.browser

import io.github.hipotures.bring.ui.SharpButton
import io.github.hipotures.bring.ui.ConfirmDialog
import io.github.hipotures.bring.ui.ActionIcon
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Check
import androidx.compose.foundation.combinedClickable
import androidx.activity.compose.BackHandler
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
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.outlined.Add
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
import io.github.hipotures.bring.media.LocalVideoThumb
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
    /** Delete the local copies of these synced files (they stay on the Ring). */
    onFreeLocal: (List<Entry>) -> Unit = {},
    /** Custom name of the active Ring, shown as the breadcrumb root. */
    ringName: String = "Ring",
    /** Watched Ring folder paths (`ringFolder`) so empty watched folders stay visible. */
    watchedFolders: List<String> = emptyList(),
    /** Quick-add a watched folder from the root breadcrumbs bar. */
    onAddFolder: (() -> Unit)? = null,
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
    val rows = remember(shown, dir, filter, watchedFolders) {
        listDir(
            shown,
            dir,
            { it.ringPath },
            { it.size.toLong() },
            newestFirst,
            extraFolders = if (filter == Filter.ALL) watchedFolders else emptyList(),
        )
    }
    // Long-press starts selecting; a tap then toggles instead of opening.
    var selected by remember(dir) { mutableStateOf(emptySet<Long>()) }
    val picked = remember(here, selected) { here.filter { it.id in selected } }
    val selecting = picked.isNotEmpty()
    var confirm by remember { mutableStateOf(false) }
    val haptics = LocalHapticFeedback.current
    BackHandler(enabled = selecting) { selected = emptySet() }
    fun toggle(e: Entry) {
        selected = if (e.id in selected) selected - e.id else selected + e.id
    }

    Box(Modifier.fillMaxSize()) {
    LazyColumn(
        Modifier.fillMaxSize(),
        contentPadding = PaddingValues(bottom = if (selecting) 160.dp else TmDimens.space6),
    ) {
        item { header() }
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
        item {
            Breadcrumbs(dir, onDir, rootLabel = ringName, onAddFolder = onAddFolder)
            RowDivider()
        }
        if (rows.isEmpty()) {
            item {
                Text(
                    when {
                        filter.state != null -> "No ${filter.label.lowercase()} files in this folder."
                        dir.isNotEmpty() -> "This folder is empty. Files added here will sync automatically."
                        entries.isEmpty() -> "Nothing here yet. Files appear as they are synced."
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
                is DirRow.File -> FileRow(
                    row.item,
                    row.name,
                    uploading = row.item.ringPath == uploading,
                    selected = row.item.id in selected,
                    onClick = { if (selecting) toggle(row.item) else onOpen(row.item) },
                    onLongClick = {
                        haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                        toggle(row.item)
                    },
                )
            }
            RowDivider()
        }
    }
    if (selecting) {
        SelectionBar(
            picked = picked,
            onClear = { selected = emptySet() },
            onAll = { selected = rows.filterIsInstance<DirRow.File<Entry>>().map { it.item.id }.toSet() },
            onFree = { confirm = true },
            modifier = Modifier.align(Alignment.BottomCenter),
        )
    }
    }

    if (confirm) {
        val synced = picked.filter { it.isFreeable() }
        ConfirmDialog(
            title = "Remove ${files(synced.size)} from this phone?",
            text = "${formatBytes(synced.sumOf { it.size })} freed. " +
                (if (synced.size == 1) "The file stays on the Ring and shows" else "The files stay on the Ring and show") +
                " as Remote: you can still open and stream ${if (synced.size == 1) "it" else "them"} here.",
            action = "Remove from phone",
            onConfirm = {
                confirm = false
                onFreeLocal(synced)
                selected = emptySet()
            },
            onDismiss = { confirm = false },
        )
    }
}

/** Only a synced copy (verified on the Ring, nothing pending) may go. */
private fun Entry.isFreeable() = state == EntryState.SYNCED && !uploadPending && localPath != null

private fun files(n: Int) = "$n ${if (n == 1) "file" else "files"}"

/**
 * Bottom bar while selecting, on `chrome` with a `line1` rule: count and
 * size, select all, close; the action names how many files it applies to.
 */
@Composable
private fun SelectionBar(
    picked: List<Entry>,
    onClear: () -> Unit,
    onAll: () -> Unit,
    onFree: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val c = Tm.colors
    val freeable = picked.count { it.isFreeable() }
    Column(modifier.fillMaxWidth().background(c.chrome)) {
        HorizontalDivider(thickness = TmDimens.bw1, color = c.line1)
        Column(
            Modifier.padding(horizontal = TmDimens.regionPad, vertical = TmDimens.space2),
            verticalArrangement = Arrangement.spacedBy(TmDimens.space1),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                ActionIcon(Icons.Outlined.Close, "Clear selection", onClear)
                Text(
                    "${picked.size} selected · ${formatBytes(picked.sumOf { it.size })}",
                    style = Tm.text.numeric,
                    color = c.text,
                    maxLines = 1,
                    modifier = Modifier.weight(1f),
                )
                SharpButton("Select all", onClick = onAll)
            }
            SharpButton(
                if (freeable == 0) "Only synced files can be removed" else "Remove from phone ($freeable)",
                onClick = onFree,
                enabled = freeable > 0,
                danger = true,
                primary = true,
                modifier = Modifier.fillMaxWidth(),
            )
            if (freeable in 1 until picked.size) {
                Text(
                    "${picked.size - freeable} skipped: only synced files have a copy here to remove.",
                    style = MaterialTheme.typography.bodySmall,
                    color = c.text2,
                )
            }
        }
    }
}

/** The Mechanics `.tm-breadcrumbs` bar on the file-list area, separate from the filter stats. */
@Composable
private fun Breadcrumbs(
    dir: String,
    onDir: (String) -> Unit,
    rootLabel: String = "Ring",
    onAddFolder: (() -> Unit)? = null,
) {
    val c = Tm.colors
    Row(
        Modifier
            .fillMaxWidth()
            .background(c.head2)
            .padding(start = TmDimens.space2, end = TmDimens.space1),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Row(
            Modifier
                .weight(1f)
                .horizontalScroll(rememberScrollState()),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (dir.isNotEmpty()) {
                ActionIcon(Icons.AutoMirrored.Outlined.ArrowBack, "Up", { onDir(parentOf(dir)) })
            }
            Crumb(rootLabel, current = dir.isEmpty()) { onDir("") }
            crumbs(dir).forEach { (name, path) ->
                Text("/", style = Tm.text.code, color = c.line2)
                Crumb(name, current = path == dir) { onDir(path) }
            }
        }
        if (dir.isEmpty() && onAddFolder != null) {
            ActionIcon(Icons.Outlined.Add, "Watch folder", onAddFolder)
        }
    }
}

@Composable
private fun Crumb(text: String, current: Boolean, onClick: () -> Unit) {
    Text(
        text,
        style = Tm.text.code,
        fontWeight = if (current) FontWeight.SemiBold else FontWeight.Normal,
        color = if (current) Tm.colors.text else Tm.colors.text2,
        modifier = Modifier
            .clickable(enabled = !current, onClick = onClick)
            .padding(vertical = 12.dp, horizontal = TmDimens.space2),
    )
}

/** Record row on `surface`: 56 dp media square, name (wraps to two lines), mono meta. */
@Composable
private fun RecordRow(
    onClick: () -> Unit,
    leading: @Composable () -> Unit,
    onLongClick: (() -> Unit)? = null,
    selected: Boolean = false,
    content: @Composable () -> Unit,
) {
    Row(
        Modifier
            .fillMaxWidth()
            .background(if (selected) Tm.colors.selectBg else Tm.colors.surface)
            .combinedClickable(onClick = onClick, onLongClick = onLongClick)
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
                Text(
                    if (row.files == 0) "0 files" else "${files(row.files)} · ${formatBytes(row.bytes)}",
                    style = Tm.text.numeric,
                    color = Tm.colors.text2,
                )
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
private fun FileRow(
    entry: Entry,
    name: String,
    uploading: Boolean,
    selected: Boolean,
    onClick: () -> Unit,
    onLongClick: () -> Unit,
) {
    RecordRow(
        onClick,
        onLongClick = onLongClick,
        selected = selected,
        leading = {
            Box {
                Thumb(entry, name)
                if (selected) {
                    Box(
                        Modifier
                            .align(Alignment.TopEnd)
                            .size(22.dp)
                            .background(Tm.colors.selectLine),
                        contentAlignment = Alignment.Center,
                    ) {
                        Icon(Icons.Outlined.Check, "Selected", Modifier.size(18.dp), tint = Tm.colors.surface)
                    }
                }
            }
        },
    ) {
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
    val model: Any? = remember(entry.ringId, entry.thumb, entry.localPath, entry.mtimeNs, kind) {
        when {
            entry.thumb != null -> RingThumb(entry.ringId, entry.thumb!!)
            entry.localPath != null && kind == MediaKind.IMAGE -> File(entry.localPath!!)
            entry.localPath != null && kind == MediaKind.VIDEO -> LocalVideoThumb(entry.localPath!!, entry.mtimeNs)
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
