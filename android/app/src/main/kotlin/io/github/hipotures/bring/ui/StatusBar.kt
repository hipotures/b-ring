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
        if (sync.running) {
            if (sync.total > 0) {
                LinearProgressIndicator(
                    progress = { (sync.sent.toFloat() / sync.total).coerceIn(0f, 1f) },
                    modifier = Modifier.fillMaxWidth().height(2.dp),
                )
                Text(
                    "↑ ${sync.path?.substringAfterLast('/')} · ${formatBytes(sync.sent)} / ${formatBytes(sync.total)}",
                    style = MetaStyle,
                    color = BringTheme.colors.dim,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.padding(horizontal = 10.dp, vertical = 2.dp),
                )
            } else {
                LinearProgressIndicator(Modifier.fillMaxWidth().height(2.dp))
            }
        }
        HorizontalDivider(thickness = Hairline, color = MaterialTheme.colorScheme.outline)
    }
}
