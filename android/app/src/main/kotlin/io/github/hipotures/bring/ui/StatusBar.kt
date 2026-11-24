package io.github.hipotures.bring.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.List
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.hipotures.bring.SyncUi
import io.github.hipotures.bring.formatBytes
import io.github.hipotures.bring.formatWhen
import io.github.hipotures.bring.sync.RouteStatus
import io.github.hipotures.bring.sync.SyncRound
import io.github.hipotures.bring.sync.roundTitle

@Composable
fun routeColor(route: RouteStatus): Color = when (route) {
    RouteStatus.LAN -> BringTheme.colors.synced
    RouteStatus.WAN -> BringTheme.colors.remote
    RouteStatus.OFFLINE -> BringTheme.colors.warn
    RouteStatus.UNKNOWN -> BringTheme.colors.dim
}

/**
 * App bar: title, route, actions (48dp targets), then what sync is doing:
 * progress while a round runs, otherwise the last round's outcome.
 */
@Composable
fun TopStrip(
    title: String,
    route: RouteStatus,
    sync: SyncUi,
    onBack: (() -> Unit)? = null,
    onSync: (() -> Unit)? = null,
    onActivity: (() -> Unit)? = null,
    onSettings: (() -> Unit)? = null,
    lastRound: SyncRound? = null,
) {
    Column(Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surfaceContainer)) {
        Row(
            Modifier.fillMaxWidth().height(60.dp).padding(horizontal = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (onBack != null) {
                IconButton(onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") }
            }
            Text(
                title,
                style = MaterialTheme.typography.titleLarge,
                fontWeight = FontWeight.SemiBold,
                color = if (title == "B-Ring") MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurface,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f).padding(start = if (onBack == null) 12.dp else 4.dp),
            )
            Tag(route.label, routeColor(route))
            if (onSync != null) {
                IconButton(onSync, enabled = !sync.running) { Icon(Icons.Filled.Refresh, "Sync now") }
            }
            if (onActivity != null) {
                IconButton(onActivity) { Icon(Icons.AutoMirrored.Filled.List, "Sync activity") }
            }
            if (onSettings != null) {
                IconButton(onSettings) { Icon(Icons.Filled.Settings, "Settings") }
            }
        }
        when {
            sync.running -> SyncBlock(sync)
            lastRound != null && onSync != null -> LastRound(lastRound, onActivity)
        }
        HorizontalDivider(thickness = Hairline, color = MaterialTheme.colorScheme.outline)
    }
}

/** "Uploaded 12 files (34 MiB) · Today 19:24", tappable to the history. */
@Composable
private fun LastRound(r: SyncRound, onClick: (() -> Unit)?) {
    Row(
        Modifier
            .fillMaxWidth()
            .heightIn(min = 40.dp)
            .clickable(enabled = onClick != null) { onClick?.invoke() }
            .padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            "${roundTitle(r)} · ${formatWhen(r.endedAt)}",
            style = MaterialTheme.typography.bodyMedium,
            color = if (r.error != null) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

/**
 * A running round. The first line (file count, bytes) is the stable one;
 * the current file name below changes at most once a second.
 */
@Composable
private fun SyncBlock(sync: SyncUi) {
    val path = sync.path
    Column(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 8.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Text(
            if (path != null) "Uploading · ${sync.files} files · ${formatBytes(sync.bytes)}" else "Checking the Ring and local folders…",
            style = MaterialTheme.typography.bodyMedium,
            maxLines = 1,
        )
        // A determinate bar only for files big enough to watch; for small
        // ones it would just flicker.
        if (path != null && sync.total >= BIG_FILE) {
            LinearProgressIndicator(
                progress = { (sync.sent.toFloat() / sync.total).coerceIn(0f, 1f) },
                modifier = Modifier.fillMaxWidth().height(4.dp),
                gapSize = 0.dp,
                drawStopIndicator = {},
            )
        } else {
            LinearProgressIndicator(Modifier.fillMaxWidth().height(4.dp), gapSize = 0.dp)
        }
        if (path != null) {
            Text(
                path.substringAfterLast('/') +
                    if (sync.total >= BIG_FILE) " · ${formatBytes(sync.sent)} / ${formatBytes(sync.total)}" else "",
                style = MetaStyle,
                color = BringTheme.colors.dim,
                maxLines = 1,
                overflow = TextOverflow.MiddleEllipsis,
            )
        }
    }
}

private const val BIG_FILE = 4L * 1024 * 1024
