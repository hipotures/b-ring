package io.github.hipotures.bring.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.List
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Refresh
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.hipotures.bring.SyncUi
import io.github.hipotures.bring.ffi.RingInfo
import io.github.hipotures.bring.formatBytes
import io.github.hipotures.bring.formatWhen
import io.github.hipotures.bring.sync.RouteStatus
import io.github.hipotures.bring.sync.SyncRound
import io.github.hipotures.bring.sync.roundTitle
import themechanics.compose.Tm
import themechanics.compose.TmDimens

/** Route → status mark. */
fun routeMark(route: RouteStatus): PlateMark = when (route) {
    RouteStatus.LAN, RouteStatus.WAN -> PlateMark.ACTIVE
    RouteStatus.OFFLINE -> PlateMark.WARNING
    RouteStatus.UNKNOWN -> PlateMark.PENDING
}

/** The route as a status plate: connected is a quiet dot, offline warns. */
@Composable
fun RoutePlate(route: RouteStatus, small: Boolean = true) {
    StatusPlate(route.label, routeMark(route), small = small)
}

/**
 * Horizontal switcher for paired Rings on Home: scrollable Ring chips on the
 * left, with optional per-Ring settings and "+" (add Ring) actions pinned at
 * the far right edge.
 */
@Composable
fun RingSwitcher(
    rings: List<RingInfo>,
    activeRingId: String,
    routes: Map<String, RouteStatus>,
    onSelect: (String) -> Unit,
    onRingSettings: (() -> Unit)? = null,
    onAddRing: (() -> Unit)? = null,
) {
    val c = Tm.colors
    Column(Modifier.fillMaxWidth().background(c.chrome)) {
        Row(
            Modifier
                .fillMaxWidth()
                .padding(horizontal = TmDimens.space3, vertical = TmDimens.space1_5),
            horizontalArrangement = Arrangement.spacedBy(TmDimens.space2),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Row(
                Modifier
                    .weight(1f)
                    .horizontalScroll(rememberScrollState()),
                horizontalArrangement = Arrangement.spacedBy(TmDimens.space2),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                rings.forEach { ring ->
                    val selected = ring.ringId == activeRingId
                    val r = routes[ring.ringId] ?: RouteStatus.UNKNOWN
                    Row(
                        Modifier
                            .border(
                                if (selected) TmDimens.bwControl else TmDimens.bw2,
                                if (selected) c.ink else c.line2,
                                ControlShape,
                            )
                            .background(if (selected) c.selectBg else c.surface, ControlShape)
                            .clickable { onSelect(ring.ringId) }
                            .heightIn(min = TmDimens.controlSm)
                            .padding(horizontal = TmDimens.controlPadXMd, vertical = TmDimens.space1),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(TmDimens.space1_5),
                    ) {
                        PlateMarkIcon(routeMark(r), 12.dp)
                        Text(
                            ring.name,
                            style = MaterialTheme.typography.labelLarge,
                            color = c.text,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
            }
            if (onRingSettings != null) {
                Row(
                    Modifier
                        .border(TmDimens.bw2, c.line2, ControlShape)
                        .background(c.surface, ControlShape)
                        .clickable(onClick = onRingSettings)
                        .heightIn(min = TmDimens.controlSm)
                        .padding(horizontal = TmDimens.space2, vertical = TmDimens.space1),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Icon(SlidersGlyph, "Ring settings", Modifier.size(18.dp), tint = c.text2)
                }
            }
            if (onAddRing != null) {
                Row(
                    Modifier
                        .border(TmDimens.bw2, c.line2, ControlShape)
                        .background(c.surface, ControlShape)
                        .clickable(onClick = onAddRing)
                        .heightIn(min = TmDimens.controlSm)
                        .padding(horizontal = TmDimens.space2, vertical = TmDimens.space1),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Icon(Icons.Outlined.Add, "Add Ring", Modifier.size(18.dp), tint = c.link)
                }
            }
        }
        HorizontalDivider(thickness = TmDimens.bw2, color = c.line2)
    }
}

/**
 * Level 1 region: `chrome` band with a 2 dp `line1` rule below. Title,
 * optional route plate and 48 dp icon actions. On the home screen a second
 * line holds the last round's result. Uploads show in the system status bar,
 * so the strip never changes height and nothing below it jumps.
 */
@Composable
fun TopStrip(
    title: String,
    route: RouteStatus? = null,
    sync: SyncUi,
    onBack: (() -> Unit)? = null,
    onSync: (() -> Unit)? = null,
    onActivity: (() -> Unit)? = null,
    onSettings: (() -> Unit)? = null,
    lastRound: SyncRound? = null,
    lastChecked: Long = 0L,
    queued: Int = 0,
    multiRing: Boolean = false,
) {
    val c = Tm.colors
    Column(Modifier.fillMaxWidth().background(c.chrome)) {
        Row(
            Modifier.fillMaxWidth().height(56.dp).padding(horizontal = TmDimens.space1),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (onBack != null) ActionIcon(Icons.AutoMirrored.Outlined.ArrowBack, "Back", onBack)
            Text(
                title,
                style = MaterialTheme.typography.titleLarge,
                color = c.text,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f).padding(start = if (onBack == null) TmDimens.space3 else TmDimens.space1),
            )
            if (route != null) RoutePlate(route)
            if (onSync != null) ActionIcon(Icons.Outlined.Refresh, "Sync now", onSync, enabled = !sync.running)
            if (onActivity != null) ActionIcon(Icons.AutoMirrored.Outlined.List, "Sync activity", onActivity)
            if (onSettings != null) ActionIcon(Icons.Outlined.Settings, "Settings", onSettings)
            if (onSync == null && onActivity == null && onSettings == null) Spacer(Modifier.width(TmDimens.space3))
        }
        when {
            onSync == null -> {}
            sync.running -> ActiveSync(sync, multiRing, onActivity)
            queued > 0 && lastRound?.error == null -> QueuedStatus(queued, lastChecked, onActivity)
            // A later check found nothing new: say that, not the old news.
            lastChecked > (lastRound?.endedAt ?: 0L) -> LastCheck(lastChecked, onActivity)
            lastRound != null -> LastRound(lastRound, onActivity)
        }
        HorizontalDivider(thickness = TmDimens.bw1, color = c.line1)
    }
}

/** Outlined icon, 20 dp, `text2`, in a 48 dp target. */
@Composable
fun ActionIcon(icon: ImageVector, description: String, onClick: () -> Unit, enabled: Boolean = true) {
    IconButton(onClick, enabled = enabled) {
        Icon(
            icon,
            description,
            Modifier.size(TmDimens.iconLg),
            tint = if (enabled) Tm.colors.text2 else Tm.colors.textDisabled,
        )
    }
}

/** [● Uploading]  42% · 1.2 GiB — same fixed-height row, so nothing below jumps. */
@Composable
private fun ActiveSync(sync: SyncUi, multiRing: Boolean, onClick: (() -> Unit)?) {
    val base = if (sync.path != null) "Uploading" else "Syncing"
    val label = if (multiRing && !sync.ringName.isNullOrBlank()) "$base · ${sync.ringName}" else base
    val detail = when {
        sync.path == null -> ""
        sync.total > 0 -> "${(sync.sent * 100 / sync.total).coerceIn(0, 100)}% · ${formatBytes(sync.bytes)}"
        sync.bytes > 0 -> formatBytes(sync.bytes)
        else -> ""
    }
    ResultRow(label, PlateMark.ACTIVE, detail, onClick)
}

/** [○ 2 queued]  Today 22:41 — files still waiting to upload. */
@Composable
private fun QueuedStatus(queued: Int, at: Long, onClick: (() -> Unit)?) =
    ResultRow("$queued queued", PlateMark.PENDING, if (at > 0) formatWhen(at) else "", onClick)

/** [✓ Uploaded 12 files (34 MiB)]  Today 19:24 — tappable to the history. */
@Composable
private fun LastRound(r: SyncRound, onClick: (() -> Unit)?) =
    ResultRow(roundTitle(r), if (r.error != null) PlateMark.ERROR else PlateMark.COMPLETE, formatWhen(r.endedAt), onClick)

/** [✓ Up to date]  Today 19:40 — the last check found nothing new. */
@Composable
private fun LastCheck(at: Long, onClick: (() -> Unit)?) = ResultRow("Up to date", PlateMark.COMPLETE, formatWhen(at), onClick)

@Composable
private fun ResultRow(text: String, mark: PlateMark, detail: String, onClick: (() -> Unit)?) {
    Row(
        Modifier
            .fillMaxWidth()
            .clickable(enabled = onClick != null) { onClick?.invoke() }
            .heightIn(min = 48.dp)
            .padding(horizontal = TmDimens.space3, vertical = TmDimens.space1),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(TmDimens.space3),
    ) {
        StatusPlate(text, mark, modifier = Modifier.weight(1f, fill = false))
        if (detail.isNotEmpty()) {
            Text(detail, style = Tm.text.numeric, color = Tm.colors.text2, maxLines = 1)
        }
    }
}
