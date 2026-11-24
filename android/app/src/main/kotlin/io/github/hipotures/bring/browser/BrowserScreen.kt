package io.github.hipotures.bring.browser

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.height
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
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.VerticalDivider
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
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
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
import io.github.hipotures.bring.ui.BringTheme
import io.github.hipotures.bring.ui.FolderGlyph
import io.github.hipotures.bring.ui.Hairline
import io.github.hipotures.bring.ui.MetaStyle
import io.github.hipotures.bring.ui.Sharp
import io.github.hipotures.bring.ui.badgesFor
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
) {
    var filter by rememberSaveable { mutableStateOf(Filter.ALL) }
    val counts = remember(entries) { entries.groupingBy { it.state }.eachCount() }
    val shown = remember(entries, filter) {
        if (filter.state == null) entries else entries.filter { it.state == filter.state }
    }
    val rows = remember(shown, dir) {
        listDir(shown, dir, { it.ringPath }, { it.size.toLong() }, newestFirst)
    }

    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 24.dp)) {
        item { Box(Modifier.padding(horizontal = 12.dp, vertical = 8.dp)) { header() } }
        item { Breadcrumbs(dir, onDir) }
        item {
            FilterBar(filter, onFilter = { filter = it }) { f ->
                if (f.state == null) entries.size else counts[f.state] ?: 0
            }
            HorizontalDivider(thickness = Hairline, color = MaterialTheme.colorScheme.outline)
        }
        if (rows.isEmpty()) {
            item {
                Text(
                    if (entries.isEmpty()) "Nothing here yet. Files appear as they are synced." else "No files match.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = BringTheme.colors.dim,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 24.dp),
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
                is DirRow.File -> FileRow(row.item, row.name) { onOpen(row.item) }
            }
            HorizontalDivider(thickness = Hairline, color = MaterialTheme.colorScheme.outlineVariant)
        }
    }
}

@Composable
private fun Breadcrumbs(dir: String, onDir: (String) -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .horizontalScroll(rememberScrollState())
            .padding(horizontal = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Crumb("Ring", current = dir.isEmpty()) { onDir("") }
        crumbs(dir).forEach { (name, path) ->
            Text("›", style = MaterialTheme.typography.bodyLarge, color = BringTheme.colors.dim)
            Crumb(name, current = path == dir) { onDir(path) }
        }
    }
}

@Composable
private fun Crumb(text: String, current: Boolean, onClick: () -> Unit) {
    Text(
        text,
        style = MaterialTheme.typography.bodyLarge,
        fontWeight = if (current) FontWeight.SemiBold else FontWeight.Normal,
        color = if (current) MaterialTheme.colorScheme.onBackground else MaterialTheme.colorScheme.primary,
        modifier = Modifier
            .clickable(enabled = !current, onClick = onClick)
            .padding(vertical = 12.dp, horizontal = 8.dp),
    )
}

@Composable
private fun FolderRow(row: DirRow.Folder, onClick: () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(
            Modifier
                .size(56.dp)
                .border(Hairline, MaterialTheme.colorScheme.outline, Sharp)
                .background(MaterialTheme.colorScheme.surfaceVariant, Sharp),
            contentAlignment = Alignment.Center,
        ) {
            Icon(FolderGlyph, null, Modifier.size(28.dp), tint = MaterialTheme.colorScheme.primary)
        }
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(row.name, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(
                "${row.files} files · ${formatBytes(row.bytes)}",
                style = MetaStyle,
                color = BringTheme.colors.dim,
            )
        }
        Text("›", fontSize = 22.sp, color = BringTheme.colors.dim)
    }
}

@Composable
private fun FileRow(entry: Entry, name: String, onClick: () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Thumb(entry, name)
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
            Text(
                name,
                style = MaterialTheme.typography.bodyLarge,
                maxLines = 1,
                overflow = TextOverflow.MiddleEllipsis,
            )
            Text(
                "${formatBytes(entry.size)} · ${formatMtime(entry.mtimeNs)}",
                style = MetaStyle,
                color = BringTheme.colors.dim,
                maxLines = 1,
            )
            FlowRow(horizontalArrangement = Arrangement.spacedBy(4.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                badgesFor(entry.state, entry.uploadPending, entry.lastError, entry.suppressed).forEach { BadgeChip(it) }
            }
        }
    }
}

/** 56dp square: the Ring's thumbnail, the local image, or the extension. */
@Composable
fun Thumb(entry: Entry, name: String, size: Int = 56) {
    val kind = remember(entry.mime, name) { kindOf(entry.mime, name) }
    val model: Any? = remember(entry.thumb, entry.localPath, kind) {
        when {
            entry.thumb != null -> RingThumb(entry.thumb!!)
            entry.localPath != null && kind == MediaKind.IMAGE -> File(entry.localPath!!)
            else -> null
        }
    }
    Box(
        Modifier
            .size(size.dp)
            .border(Hairline, MaterialTheme.colorScheme.outline, Sharp)
            .background(MaterialTheme.colorScheme.surfaceVariant, Sharp),
        contentAlignment = Alignment.Center,
    ) {
        // The label stays visible underneath while loading or on failure.
        Text(
            extensionOf(name).take(4).uppercase().ifEmpty { "FILE" },
            style = MetaStyle.copy(fontSize = 12.sp, fontWeight = FontWeight.Bold),
            color = BringTheme.colors.dim,
        )
        if (model != null) {
            AsyncImage(model, null, Modifier.fillMaxSize(), contentScale = ContentScale.Crop)
        }
        if (kind == MediaKind.VIDEO) {
            Text(
                "▶",
                fontSize = 12.sp,
                color = BringTheme.colors.onBadge,
                modifier = Modifier
                    .align(Alignment.BottomStart)
                    .background(MaterialTheme.colorScheme.primary)
                    .padding(horizontal = 3.dp),
            )
        }
    }
}

/** Full-width segmented control: one square cell per filter, label over count. */
@Composable
private fun FilterBar(selected: Filter, onFilter: (Filter) -> Unit, count: (Filter) -> Int) {
    Row(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 12.dp, vertical = 8.dp)
            .height(IntrinsicSize.Min)
            .border(Hairline, MaterialTheme.colorScheme.outline, Sharp),
    ) {
        Filter.entries.forEachIndexed { i, f ->
            val on = f == selected
            if (i > 0) VerticalDivider(thickness = Hairline, color = MaterialTheme.colorScheme.outline)
            Column(
                Modifier
                    .weight(1f)
                    .background(if (on) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceContainer)
                    .clickable { onFilter(f) }
                    .padding(vertical = 8.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                val fg = if (on) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurface
                Text(f.label, style = MaterialTheme.typography.labelLarge, color = fg, maxLines = 1)
                Text("${count(f)}", style = MetaStyle, color = if (on) fg else BringTheme.colors.dim, maxLines = 1)
            }
        }
    }
}
