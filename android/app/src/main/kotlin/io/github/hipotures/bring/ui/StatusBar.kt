package io.github.hipotures.bring.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
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
import androidx.compose.ui.unit.sp
import io.github.hipotures.bring.SyncUi
import io.github.hipotures.bring.formatBytes
import io.github.hipotures.bring.sync.RouteStatus

@Composable
fun routeColor(route: RouteStatus): Color = when (route) {
    RouteStatus.LAN -> BringTheme.colors.synced
    RouteStatus.WAN -> BringTheme.colors.remote
    RouteStatus.OFFLINE -> BringTheme.colors.warn
    RouteStatus.UNKNOWN -> BringTheme.colors.dim
}

/** Title row with the route tag, sync control and an optional back arrow. */
@Composable
fun TopStrip(
    title: String,
    route: RouteStatus,
    sync: SyncUi,
    onBack: (() -> Unit)? = null,
    onSync: (() -> Unit)? = null,
    onSettings: (() -> Unit)? = null,
) {
    Column(Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surfaceContainer)) {
        Row(
            Modifier.fillMaxWidth().height(48.dp).padding(horizontal = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            if (onBack != null) {
                IconButton(onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") }
            } else {
                Text(
                    "B·RING",
                    fontFamily = Mono,
                    fontWeight = FontWeight.Bold,
                    color = MaterialTheme.colorScheme.primary,
                    modifier = Modifier.padding(start = 8.dp),
                )
            }
            Text(
                title,
                style = MaterialTheme.typography.titleMedium,
                maxLines = 1,
                overflow = TextOverflow.StartEllipsis,
                modifier = Modifier.weight(1f),
            )
            Tag(route.label, routeColor(route))
            if (onSync != null) {
                IconButton(onSync, enabled = !sync.running) {
                    Icon(Icons.Filled.Refresh, "Sync now", Modifier.size(22.dp))
                }
            }
            if (onSettings != null) {
                IconButton(onSettings) { Icon(Icons.Filled.Settings, "Settings", Modifier.size(22.dp)) }
            }
        }
        if (sync.running) SyncBlock(sync)
        HorizontalDivider(thickness = Hairline, color = MaterialTheme.colorScheme.outline)
    }
}

/**
 * What a running round is doing. The first line (file count, bytes) is the
 * stable one; the current file name below changes at most once a second.
 */
@Composable
private fun SyncBlock(sync: SyncUi) {
    val path = sync.path
    Column(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 12.dp, vertical = 6.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Tag(if (path != null) "UPLOAD" else "SYNC", MaterialTheme.colorScheme.primary)
            Text(
                if (path != null) "${sync.files} files · ${formatBytes(sync.bytes)} sent" else "Checking the Ring and local folders…",
                style = MaterialTheme.typography.bodyMedium,
                maxLines = 1,
            )
        }
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
                style = MetaStyle.copy(fontSize = 12.sp),
                color = BringTheme.colors.dim,
                maxLines = 1,
                overflow = TextOverflow.MiddleEllipsis,
            )
        }
    }
}

private const val BIG_FILE = 4L * 1024 * 1024
