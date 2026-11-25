package io.github.hipotures.bring.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
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
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.hipotures.bring.R
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
 * The Mechanics `.tm-tabs` switcher for paired Rings on Home.
 * Everything above this bar is global (app-wide); everything below belongs to
 * the selected Ring.
 */
@Composable
fun RingSwitcher(
    rings: List<RingInfo>,
    activeRingId: String,
    routes: Map<String, RouteStatus>,
    onSelect: (String) -> Unit,
) {
    val c = Tm.colors
    Column(Modifier.fillMaxWidth().background(c.chrome)) {
        Row(
            Modifier
                .fillMaxWidth()
                .horizontalScroll(rememberScrollState())
                .padding(horizontal = TmDimens.space2),
            verticalAlignment = Alignment.Bottom,
        ) {
            rings.forEach { ring ->
                val selected = ring.ringId == activeRingId
                val r = routes[ring.ringId] ?: RouteStatus.UNKNOWN
                Column(
                    Modifier
                        .width(IntrinsicSize.Max)
                        .clickable { onSelect(ring.ringId) },
                ) {
                    Row(
                        Modifier
                            .heightIn(min = 44.dp)
                            .padding(horizontal = TmDimens.space3),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(TmDimens.space1_5),
                    ) {
                        PlateMarkIcon(routeMark(r), 10.dp)
                        Text(
                            ring.name,
                            style = MaterialTheme.typography.labelLarge,
                            fontWeight = if (selected) FontWeight.SemiBold else FontWeight.Medium,
                            color = if (selected) c.text else c.text2,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                    Box(
                        Modifier
                            .fillMaxWidth()
                            .height(3.dp)
                            .background(if (selected) c.selectLine else Color.Transparent),
                    )
                }
            }
        }
        HorizontalDivider(thickness = TmDimens.bw1, color = c.line1)
    }
}

/** Transparent-background B-Ring emblem adapting to Light / Dark / Coffee themes. */
@Composable
private fun BringMark(modifier: Modifier = Modifier) {
    val ringColor = Tm.colors.ink
    val moonColor = Tm.colors.selectLine
    Canvas(modifier) {
        val s = size.minDimension / 48f
        val ring = Path().apply {
            moveTo(28f * s, 13f * s)
            lineTo(11f * s, 13f * s)
            lineTo(11f * s, 37f * s)
            lineTo(35f * s, 37f * s)
            lineTo(35f * s, 20f * s)
        }
        drawPath(
            path = ring,
            color = ringColor,
            style = Stroke(width = 4.5f * s, cap = StrokeCap.Square, join = StrokeJoin.Miter),
        )
        drawRect(
            color = moonColor,
            topLeft = Offset(32f * s, 6f * s),
            size = Size(10f * s, 10f * s),
        )
    }
}

/**
 * Per-Ring fixed-height (56 dp) two-line bar below [RingSwitcher]:
 * line 1 = status (quiet text when normal, loud [Tag] on active sync / error / offline),
 * line 2 = route + timestamp (or active filename), never disappearing during sync.
 * Right side holds local actions (Sync now, Ring settings) at fixed positions.
 */
@Composable
fun RingBar(
    route: RouteStatus,
    sync: SyncUi,
    onSync: () -> Unit,
    onRingSettings: () -> Unit,
    onActivity: () -> Unit,
    lastRound: SyncRound? = null,
    lastChecked: Long = 0L,
    queued: Int = 0,
    multiRing: Boolean = false,
) {
    val c = Tm.colors
    val whenText = when {
        lastChecked > (lastRound?.endedAt ?: 0L) -> formatWhen(lastChecked)
        lastRound != null -> formatWhen(lastRound.endedAt)
        else -> null
    }
    val subLine = when {
        sync.running -> listOfNotNull(
            route.label.takeIf { route != RouteStatus.UNKNOWN } ?: "Checking",
            sync.ringName?.takeIf { multiRing && it.isNotBlank() },
        ).joinToString(" · ")
        else -> listOfNotNull(
            route.label.takeIf { route != RouteStatus.UNKNOWN } ?: "Checking",
            whenText,
        ).joinToString(" · ")
    }
    Column(Modifier.fillMaxWidth().background(c.surface)) {
        Row(
            Modifier
                .fillMaxWidth()
                .height(56.dp)
                .padding(start = TmDimens.space3, end = TmDimens.space1),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(
                Modifier
                    .weight(1f)
                    .fillMaxHeight()
                    .clickable(onClick = onActivity)
                    .padding(end = TmDimens.space2),
                verticalArrangement = Arrangement.Center,
            ) {
                val (headline, headlineColor) = when {
                    sync.running -> {
                        val text = when {
                            sync.files > 0 -> {
                                val f = "${sync.files} ${if (sync.files == 1) "file" else "files"}"
                                if (sync.bytes > 0) "Uploading · $f · ${formatBytes(sync.bytes)}" else "Uploading · $f"
                            }
                            sync.bytes > 0 -> "Uploading · ${formatBytes(sync.bytes)}"
                            sync.path != null -> "Uploading…"
                            else -> "Checking files…"
                        }
                        text to c.text
                    }
                    route == RouteStatus.OFFLINE -> "Offline" to c.warning
                    lastRound?.error != null && lastChecked <= lastRound.endedAt -> {
                        "Failed · ${roundTitle(lastRound)}" to c.danger
                    }
                    queued > 0 -> "$queued queued" to c.text
                    lastChecked > (lastRound?.endedAt ?: 0L) -> "Up to date" to c.text
                    lastRound != null -> roundTitle(lastRound) to c.text
                    else -> "Ready" to c.text
                }
                Text(
                    headline,
                    style = MaterialTheme.typography.bodyMedium,
                    color = headlineColor,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    subLine,
                    style = Tm.text.numeric,
                    color = c.text2,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            ActionIcon(Icons.Outlined.Refresh, "Sync now", onSync, enabled = !sync.running)
            ActionIcon(SlidersGlyph, "Ring settings", onRingSettings)
        }
        HorizontalDivider(thickness = TmDimens.bw2, color = c.line2)
    }
}

/**
 * Level 1 region: `chrome` band with a divider below. Title, optional route
 * plate and 48 dp icon actions.
 */
@Composable
fun TopStrip(
    title: String,
    route: RouteStatus? = null,
    sync: SyncUi,
    showAppIcon: Boolean = false,
    onBack: (() -> Unit)? = null,
    onSync: (() -> Unit)? = null,
    onAddRing: (() -> Unit)? = null,
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
            if (showAppIcon && onBack == null) {
                BringMark(
                    modifier = Modifier
                        .padding(start = TmDimens.space3)
                        .size(26.dp),
                )
            }
            Text(
                title,
                style = MaterialTheme.typography.titleLarge,
                color = c.text,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f).padding(
                    start = when {
                        onBack != null -> TmDimens.space1
                        showAppIcon -> TmDimens.space2
                        else -> TmDimens.space3
                    },
                ),
            )
            if (route != null) RoutePlate(route)
            if (onSync != null) ActionIcon(Icons.Outlined.Refresh, "Sync now", onSync, enabled = !sync.running)
            if (onAddRing != null) ActionIcon(Icons.Outlined.Add, "Add Ring", onAddRing)
            if (onSettings != null) ActionIcon(Icons.Outlined.Settings, "Settings", onSettings)
            if (onSync == null && onAddRing == null && onSettings == null) Spacer(Modifier.width(TmDimens.space3))
        }
        when {
            onSync == null -> {}
            sync.running -> ActiveSync(sync, multiRing, onActivity)
            queued > 0 && lastRound?.error == null -> QueuedStatus(queued, lastChecked, onActivity)
            // A later check found nothing new: say that, not the old news.
            lastChecked > (lastRound?.endedAt ?: 0L) -> LastCheck(lastChecked, onActivity)
            lastRound != null -> LastRound(lastRound, onActivity)
        }
        HorizontalDivider(thickness = if (onAddRing != null) TmDimens.bw2 else TmDimens.bw1, color = if (onAddRing != null) c.line2 else c.line1)
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
        Box(Modifier.weight(1f)) {
            StatusPlate(text, mark)
        }
        if (detail.isNotEmpty()) {
            Text(detail, style = Tm.text.numeric, color = Tm.colors.text2, maxLines = 1)
        }
    }
}
