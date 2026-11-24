package io.github.hipotures.bring.activity

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.remember
import io.github.hipotures.bring.ffi.Entry
import io.github.hipotures.bring.ffi.EntryState
import io.github.hipotures.bring.formatBytes
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import io.github.hipotures.bring.formatDuration
import io.github.hipotures.bring.formatWhen
import io.github.hipotures.bring.sync.SyncRound
import io.github.hipotures.bring.sync.roundDetail
import io.github.hipotures.bring.sync.roundTitle
import io.github.hipotures.bring.ui.RowDivider
import io.github.hipotures.bring.ui.Tag
import themechanics.compose.Tm
import themechanics.compose.TmDimens
import themechanics.compose.TmTone

/** Sync history: one record per round, newest first; tap a record for its files. */
@Composable
fun ActivityScreen(rounds: List<SyncRound>, lastChecked: Long, entries: List<Entry>) {
    val c = Tm.colors
    var open by rememberSaveable { mutableStateOf<Long?>(null) }
    val stats = remember(entries) { Stats.of(entries) }
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = TmDimens.space6)) {
        item {
            StatsTable(stats)
            HorizontalDivider(thickness = TmDimens.bw2, color = c.line2)
        }
        item {
            Row(
                Modifier.padding(horizontal = TmDimens.regionPad, vertical = TmDimens.space3),
                horizontalArrangement = Arrangement.spacedBy(TmDimens.space2),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text("LAST CHECK", style = Tm.text.eyebrow, color = c.text2)
                Text(
                    if (lastChecked > 0) formatWhen(lastChecked) else "never",
                    style = Tm.text.numeric,
                    color = c.text,
                )
            }
            HorizontalDivider(thickness = TmDimens.bw2, color = c.line2)
        }
        if (rounds.isEmpty()) {
            item {
                Text(
                    "Rounds that upload or receive something show up here. Background checks that find nothing new are not listed.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = c.text2,
                    modifier = Modifier.padding(TmDimens.regionPad),
                )
            }
        }
        items(rounds, key = { it.endedAt }) { r ->
            RoundRow(r, expanded = open == r.endedAt) { open = if (open == r.endedAt) null else r.endedAt }
            RowDivider()
        }
    }
}

@Composable
private fun RoundRow(r: SyncRound, expanded: Boolean, onToggle: () -> Unit) {
    val c = Tm.colors
    Column(
        Modifier
            .fillMaxWidth()
            .background(c.surface)
            .clickable(enabled = r.files.isNotEmpty(), onClick = onToggle)
            .padding(horizontal = TmDimens.regionPad, vertical = TmDimens.space3),
        verticalArrangement = Arrangement.spacedBy(TmDimens.space1),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(TmDimens.space2)) {
            Text(
                roundTitle(r),
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.SemiBold,
                color = c.text,
                modifier = Modifier.weight(1f),
            )
            if (r.files.isNotEmpty()) {
                Text(if (expanded) "Hide files" else "Show files", style = MaterialTheme.typography.labelMedium, color = c.link)
            }
            if (r.error != null) Tag("Failed", TmTone.Danger)
        }
        Text(roundDetail(r), style = MaterialTheme.typography.bodyMedium, color = if (r.error != null) c.danger else c.text2)
        Text(
                listOfNotNull(
                    formatWhen(r.endedAt),
                    r.route.ifEmpty { null },
                    if (r.manual) "manual" else "background",
                    formatDuration(r.durationMs),
                    // Wrap only between parts, never inside "0.5 s" or "Today 19:24".
                ).joinToString(" · ") { it.replace(' ', '\u00A0') },
                style = Tm.text.numeric,
                color = c.text2,
            )
        if (expanded) {
            Column(Modifier.padding(top = TmDimens.space1_5), verticalArrangement = Arrangement.spacedBy(TmDimens.space0_5)) {
                r.files.asReversed().forEach {
                    Text(it, style = Tm.text.code, color = c.text, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                if (r.uploaded > r.files.size) {
                    Text("… and ${r.uploaded - r.files.size} more", style = Tm.text.code, color = c.text2)
                }
            }
        }
    }
}

/** Count and bytes of one group of entries. */
private data class Tally(val files: Int = 0, val bytes: Long = 0) {
    operator fun plus(e: Entry) = Tally(files + 1, bytes + e.size.toLong())
}

/** Every file this device knows of: here, on the Ring, or both. */
private data class Stats(val all: Tally, val local: Tally, val synced: Tally, val remote: Tally, val queued: Tally) {
    companion object {
        fun of(entries: List<Entry>): Stats {
            var all = Tally(); var local = Tally(); var synced = Tally(); var remote = Tally(); var queued = Tally()
            for (e in entries) {
                all += e
                when (e.state) {
                    EntryState.LOCAL_ONLY -> local += e
                    EntryState.SYNCED -> synced += e
                    EntryState.REMOTE_ONLY -> remote += e
                }
                if (e.uploadPending) queued += e
            }
            return Stats(all, local, synced, remote, queued)
        }
    }
}

/**
 * All / Local / Synced / Remote, same cuts as the file list's filter:
 * file count over total size. A line below when uploads are queued.
 */
@Composable
private fun StatsTable(s: Stats) {
    val c = Tm.colors
    Column(
        Modifier.padding(horizontal = TmDimens.panelPad, vertical = TmDimens.space3),
        verticalArrangement = Arrangement.spacedBy(TmDimens.space2),
    ) {
        Row(
            Modifier
                .fillMaxWidth()
                .height(IntrinsicSize.Min)
                .border(TmDimens.bw2, c.line2)
                .background(c.surface),
        ) {
            listOf("All" to s.all, "Local" to s.local, "Synced" to s.synced, "Remote" to s.remote)
                .forEachIndexed { i, (label, t) ->
                    if (i > 0) Box(Modifier.width(TmDimens.bw1).fillMaxHeight().background(c.line3))
                    Column(
                        Modifier.weight(1f).padding(horizontal = TmDimens.space2, vertical = TmDimens.space2),
                        horizontalAlignment = Alignment.CenterHorizontally,
                    ) {
                        Text(label.uppercase(), style = Tm.text.eyebrow, color = c.text2, maxLines = 1)
                        Text("${t.files}", style = Tm.text.numeric, color = c.text, maxLines = 1)
                        Text(formatBytes(t.bytes), style = Tm.text.numeric, color = c.text2, maxLines = 1)
                    }
                }
        }
        if (s.queued.files > 0) {
            Row(horizontalArrangement = Arrangement.spacedBy(TmDimens.space2), verticalAlignment = Alignment.CenterVertically) {
                Tag("Queued", TmTone.Pending)
                Text(
                    "${s.queued.files} ${if (s.queued.files == 1) "file" else "files"} · ${formatBytes(s.queued.bytes)}",
                    style = Tm.text.numeric,
                    color = c.text,
                )
            }
        }
    }
}
