package io.github.hipotures.bring.activity

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.remember
import io.github.hipotures.bring.ffi.QueuedSummary
import io.github.hipotures.bring.ffi.RingInfo
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
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.DateRange
import androidx.compose.material3.DatePicker
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.SelectableDates
import androidx.compose.material3.rememberDatePickerState
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.ZoneOffset
import java.time.format.DateTimeFormatter
import androidx.compose.material.icons.outlined.KeyboardArrowDown
import androidx.compose.material.icons.outlined.KeyboardArrowUp
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableIntStateOf
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

/** Rounds shown at first, and added each time the list is scrolled to its end. */
private const val PAGE = 50

private val DAY_LABEL = DateTimeFormatter.ofPattern("d MMM yyyy")

private fun SyncRound.localDay(): LocalDate = Instant.ofEpochMilli(endedAt).atZone(ZoneId.systemDefault()).toLocalDate()

/** Sync history: one record per round, newest first; tap a record for its files. */
@Composable
fun ActivityScreen(
    rounds: List<SyncRound>,
    lastChecked: Long,
    rings: List<RingInfo>,
    queued: QueuedSummary?,
    activeRingName: String,
) {
    val c = Tm.colors
    var open by rememberSaveable { mutableStateOf<Long?>(null) }
    // The chosen day as an epoch day (local time zone); null = every day.
    var day by rememberSaveable { mutableStateOf<Long?>(null) }
    var picking by rememberSaveable { mutableStateOf(false) }
    var limit by rememberSaveable(day) { mutableIntStateOf(PAGE) }
    val days = remember(rounds) { rounds.mapTo(HashSet()) { it.localDay() } }
    val shown = remember(rounds, day) { day?.let { d -> rounds.filter { it.localDay().toEpochDay() == d } } ?: rounds }
    val totals = remember(rings, queued) { TransferTotals.of(rings, queued) }
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = TmDimens.space6)) {
        item {
            TransferSummary(totals, activeRingName)
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
                    modifier = Modifier.weight(1f),
                )
                val chosen = day
                if (chosen != null) {
                    Text(
                        LocalDate.ofEpochDay(chosen).format(DAY_LABEL),
                        style = Tm.text.numeric,
                        color = c.link,
                        modifier = Modifier.clickable { picking = !picking },
                    )
                    Icon(
                        Icons.Outlined.Close,
                        "Show all days",
                        Modifier.clickable { day = null; picking = false }.size(TmDimens.iconLg),
                        tint = c.link,
                    )
                } else if (days.isNotEmpty()) {
                    Icon(
                        Icons.Outlined.DateRange,
                        "Pick a day",
                        Modifier.clickable { picking = !picking }.size(TmDimens.iconLg),
                        tint = c.link,
                    )
                }
            }
            if (picking && days.isNotEmpty()) {
                RoundCalendar(days, day) { day = it; picking = false }
            }
            HorizontalDivider(thickness = TmDimens.bw2, color = c.line2)
        }
        if (shown.isEmpty() && day != null) {
            item {
                Text("Nothing was synced on this day.", style = MaterialTheme.typography.bodyMedium, color = c.text2, modifier = Modifier.padding(TmDimens.regionPad))
            }
        } else if (rounds.isEmpty()) {
            item {
                Text(
                    "Rounds that upload or receive something show up here. Checks that find nothing new are not listed; see Last check above.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = c.text2,
                    modifier = Modifier.padding(TmDimens.regionPad),
                )
            }
        }
        items(shown.take(limit), key = { it.endedAt }) { r ->
            RoundRow(r, expanded = open == r.endedAt) { open = if (open == r.endedAt) null else r.endedAt }
            RowDivider()
        }
        // Reached only by scrolling to the end of what is shown: pull in the next page.
        if (shown.size > limit) {
            item(key = "more") { LaunchedEffect(limit) { limit += PAGE } }
        }
    }
}

@Composable
private fun RoundRow(r: SyncRound, expanded: Boolean, onToggle: () -> Unit) {
    val c = Tm.colors
    val detail = roundDetail(r)
    val shown = r.files.take(SyncRound.MAX_FILES)
    Column(
        Modifier
            .fillMaxWidth()
            .background(c.surface)
            .clickable(onClick = onToggle)
            .padding(horizontal = TmDimens.regionPad, vertical = TmDimens.space3),
        verticalArrangement = Arrangement.spacedBy(TmDimens.space1),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(TmDimens.space2)) {
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(TmDimens.space1)) {
                Text(
                    roundTitle(r),
                    style = MaterialTheme.typography.bodyMedium,
                    fontWeight = FontWeight.SemiBold,
                    color = c.text,
                )
                if (detail.isNotEmpty()) {
                    Text(detail, style = MaterialTheme.typography.bodyMedium, color = if (r.error != null) c.danger else c.text2)
                }
                Text(
                    // Wrap only between parts, never inside "Today 19:24".
                    listOfNotNull(r.ringName.ifEmpty { null }, formatWhen(r.endedAt))
                        .joinToString(" · ") { it.replace(' ', '\u00A0') },
                    style = Tm.text.numeric,
                    color = c.text2,
                )
            }
            if (r.error != null) Tag("Failed", TmTone.Danger)
            Icon(
                if (expanded) Icons.Outlined.KeyboardArrowUp else Icons.Outlined.KeyboardArrowDown,
                if (expanded) "Hide details" else "Show details",
                Modifier.size(TmDimens.iconLg),
                tint = c.link,
            )
        }
        if (expanded) {
            Column(Modifier.padding(top = TmDimens.space1_5), verticalArrangement = Arrangement.spacedBy(TmDimens.space0_5)) {
                Text(
                    listOfNotNull(
                        r.route.ifEmpty { null },
                        if (r.manual) "manual" else "background",
                        formatDuration(r.durationMs),
                    ).joinToString(" · ") { it.replace(' ', '\u00A0') },
                    style = Tm.text.numeric,
                    color = c.text2,
                )
                FileNames(shown, r.uploaded)
                val onRing = r.receivedFiles.take(SyncRound.MAX_FILES)
                if (onRing.isNotEmpty()) {
                    if (shown.isNotEmpty()) Text("New on the Ring", style = Tm.text.eyebrow, color = c.text2, modifier = Modifier.padding(top = TmDimens.space1_5))
                    FileNames(onRing, r.received)
                }
            }
        }
    }
}

/** Global cumulative remote transfer across all paired Rings, plus the active Ring's queued count. */
private data class TransferTotals(
    val files: Long,
    val bytes: Long,
    val queuedFiles: Int,
    val queuedBytes: Long,
) {
    companion object {
        fun of(rings: List<RingInfo>, queued: QueuedSummary?): TransferTotals {
            var files = 0L
            var bytes = 0L
            for (r in rings) {
                files += r.transferredFiles.toLong()
                bytes += r.transferredBytes.toLong()
            }
            val qFiles = queued?.files?.toInt() ?: 0
            val qBytes = queued?.bytes?.toLong() ?: 0L
            return TransferTotals(files, bytes, qFiles, qBytes)
        }
    }
}

/**
 * Global cumulative transfer to remote Rings: total uploaded file count and
 * total transferred size, plus a queued line when uploads are waiting. The
 * totals span all Rings while the queue is the active Ring's, so both are
 * labelled with their scope.
 */
@Composable
private fun TransferSummary(t: TransferTotals, activeRingName: String) {
    val c = Tm.colors
    Column(
        Modifier.padding(horizontal = TmDimens.panelPad, vertical = TmDimens.space3),
        verticalArrangement = Arrangement.spacedBy(TmDimens.space2),
    ) {
        Text("ALL RINGS", style = Tm.text.eyebrow, color = c.text2)
        Row(
            Modifier
                .fillMaxWidth()
                .height(IntrinsicSize.Min)
                .border(TmDimens.bw2, c.line2)
                .background(c.surface),
        ) {
            Column(
                Modifier.weight(1f).padding(horizontal = TmDimens.space3, vertical = TmDimens.space2),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Text("UPLOADED FILES", style = Tm.text.eyebrow, color = c.text2, maxLines = 1)
                Text("${t.files}", style = Tm.text.numeric, color = c.text, maxLines = 1)
            }
            Box(Modifier.width(TmDimens.bw1).fillMaxHeight().background(c.line3))
            Column(
                Modifier.weight(1f).padding(horizontal = TmDimens.space3, vertical = TmDimens.space2),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Text("TOTAL SIZE", style = Tm.text.eyebrow, color = c.text2, maxLines = 1)
                Text(formatBytes(t.bytes), style = Tm.text.numeric, color = c.text, maxLines = 1)
            }
        }
        if (t.queuedFiles > 0) {
            Row(horizontalArrangement = Arrangement.spacedBy(TmDimens.space2), verticalAlignment = Alignment.CenterVertically) {
                Tag("Queued", TmTone.Pending)
                Text(
                    listOfNotNull(
                        "${t.queuedFiles} ${if (t.queuedFiles == 1) "file" else "files"}",
                        formatBytes(t.queuedBytes),
                        activeRingName.ifEmpty { null },
                    ).joinToString(" · "),
                    style = Tm.text.numeric,
                    color = c.text,
                )
            }
        }
    }
}

/** The names of [shown] files (the folder dimmer, on its own line) and "… and N more" out of [total]. */
@Composable
private fun FileNames(shown: List<String>, total: Int) {
    val c = Tm.colors
    shown.forEach {
        // The file name is what matters: let it wrap, and give the folder its own dimmer line.
        Column(Modifier.padding(top = TmDimens.space1)) {
            Text(it.substringAfterLast('/'), style = Tm.text.code, color = c.text)
            val dir = it.substringBeforeLast('/', "")
            if (dir.isNotEmpty()) {
                Text(dir, style = Tm.text.code, color = c.text2, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
        }
    }
    if (total > shown.size && shown.isNotEmpty()) {
        Text("… and ${total - shown.size} more", style = Tm.text.code, color = c.text2)
    }
}

/** A month calendar of [days] (the days that have rounds); every other day is greyed out and cannot be picked. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun RoundCalendar(days: Set<LocalDate>, selected: Long?, onPick: (Long) -> Unit) {
    val first = days.min()
    val last = days.max()
    val selectable = remember(days) {
        object : SelectableDates {
            // The picker speaks UTC midnights; `days` are local dates.
            override fun isSelectableDate(utcTimeMillis: Long) =
                Instant.ofEpochMilli(utcTimeMillis).atZone(ZoneOffset.UTC).toLocalDate() in days

            override fun isSelectableYear(year: Int) = year in first.year..last.year
        }
    }
    fun utc(d: LocalDate) = d.atStartOfDay(ZoneOffset.UTC).toInstant().toEpochMilli()
    val state = rememberDatePickerState(
        initialSelectedDateMillis = selected?.let { utc(LocalDate.ofEpochDay(it)) },
        initialDisplayedMonthMillis = utc(selected?.let(LocalDate::ofEpochDay) ?: last),
        yearRange = first.year..last.year,
        selectableDates = selectable,
    )
    DatePicker(state, title = null, headline = null, showModeToggle = false)
    LaunchedEffect(state.selectedDateMillis) {
        val picked = state.selectedDateMillis ?: return@LaunchedEffect
        val d = Instant.ofEpochMilli(picked).atZone(ZoneOffset.UTC).toLocalDate().toEpochDay()
        if (d != selected) onPick(d)
    }
}
