package io.github.hipotures.bring.activity

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
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
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.hipotures.bring.formatDuration
import io.github.hipotures.bring.formatWhen
import io.github.hipotures.bring.sync.SyncRound
import io.github.hipotures.bring.sync.roundDetail
import io.github.hipotures.bring.sync.roundTitle
import io.github.hipotures.bring.ui.BringTheme
import io.github.hipotures.bring.ui.Hairline
import io.github.hipotures.bring.ui.MetaStyle
import io.github.hipotures.bring.ui.Tag

/** Sync history: one row per round, newest first; tap a row for its files. */
@Composable
fun ActivityScreen(rounds: List<SyncRound>, lastChecked: Long) {
    var open by rememberSaveable { mutableStateOf<Long?>(null) }
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 24.dp)) {
        item {
            Text(
                if (lastChecked > 0) "Last check: ${formatWhen(lastChecked)}" else "No sync yet.",
                style = MaterialTheme.typography.bodyMedium,
                color = BringTheme.colors.dim,
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
            )
            HorizontalDivider(thickness = Hairline, color = MaterialTheme.colorScheme.outline)
        }
        if (rounds.isEmpty()) {
            item {
                Text(
                    "Rounds that upload or receive something show up here. Background checks that find nothing new are not listed.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = BringTheme.colors.dim,
                    modifier = Modifier.padding(16.dp),
                )
            }
        }
        items(rounds, key = { it.endedAt }) { r ->
            RoundRow(r, expanded = open == r.endedAt) { open = if (open == r.endedAt) null else r.endedAt }
            HorizontalDivider(thickness = Hairline, color = MaterialTheme.colorScheme.outlineVariant)
        }
    }
}

@Composable
private fun RoundRow(r: SyncRound, expanded: Boolean, onToggle: () -> Unit) {
    val c = BringTheme.colors
    Column(
        Modifier
            .fillMaxWidth()
            .clickable(enabled = r.files.isNotEmpty(), onClick = onToggle)
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(
                roundTitle(r),
                style = MaterialTheme.typography.bodyLarge,
                color = if (r.error != null) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface,
                modifier = Modifier.weight(1f),
            )
            if (r.route.isNotEmpty()) Tag(r.route, if (r.route == "LAN") c.synced else c.remote)
        }
        Text(
            roundDetail(r),
            style = MaterialTheme.typography.bodyMedium,
            color = if (r.error != null) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Text(
            "${formatWhen(r.endedAt)} · ${if (r.manual) "manual" else "background"} · ${formatDuration(r.durationMs)}" +
                if (r.files.isNotEmpty()) " · ${if (expanded) "hide" else "show"} files" else "",
            style = MetaStyle,
            color = c.dim,
        )
        if (expanded) {
            Column(Modifier.padding(top = 6.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                r.files.asReversed().forEach {
                    Text(it, style = MetaStyle, maxLines = 1, overflow = TextOverflow.MiddleEllipsis)
                }
                if (r.uploaded > r.files.size) {
                    Text("… and ${r.uploaded - r.files.size} more", style = MetaStyle, color = c.dim)
                }
            }
        }
    }
}
