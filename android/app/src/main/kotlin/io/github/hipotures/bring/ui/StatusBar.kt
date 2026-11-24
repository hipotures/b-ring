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
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.List
import androidx.compose.material.icons.outlined.Refresh
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.hipotures.bring.SyncUi
import io.github.hipotures.bring.formatBytes
import io.github.hipotures.bring.formatWhen
import io.github.hipotures.bring.sync.RouteStatus
import io.github.hipotures.bring.sync.SyncRound
import io.github.hipotures.bring.sync.roundTitle
import themechanics.compose.Tm
import themechanics.compose.TmDimens

/** The route as a status plate: connected is a quiet dot, offline warns. */
@Composable
fun RoutePlate(route: RouteStatus, small: Boolean = true) {
    val mark = when (route) {
        RouteStatus.LAN, RouteStatus.WAN -> PlateMark.ACTIVE
        RouteStatus.OFFLINE -> PlateMark.WARNING
        RouteStatus.UNKNOWN -> PlateMark.PENDING
    }
    StatusPlate(route.label, mark, small = small)
}

/**
 * Level 1 region: `chrome` band with a 2 dp `line1` rule below. Title,
 * route plate and 48 dp icon actions, then what sync is doing: progress
 * while a round runs, otherwise the last round's result as a status plate.
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
    lastChecked: Long = 0L,
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
            RoutePlate(route)
            if (onSync != null) ActionIcon(Icons.Outlined.Refresh, "Sync now", onSync, enabled = !sync.running)
            if (onActivity != null) ActionIcon(Icons.AutoMirrored.Outlined.List, "Sync activity", onActivity)
            if (onSettings != null) ActionIcon(Icons.Outlined.Settings, "Settings", onSettings)
            if (onSync == null && onActivity == null && onSettings == null) Spacer(Modifier.width(TmDimens.space3))
        }
        when {
            sync.running -> SyncBlock(sync)
            onSync == null -> {}
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

/** [✓ Uploaded 12 files (34 MiB)]  Today 19:24 — tappable to the history. */
@Composable
private fun LastRound(r: SyncRound, onClick: (() -> Unit)?) =
    ResultRow(roundTitle(r), if (r.error != null) PlateMark.ERROR else PlateMark.COMPLETE, r.endedAt, onClick)

/** [✓ Up to date]  Today 19:40 — the last check found nothing new. */
@Composable
private fun LastCheck(at: Long, onClick: (() -> Unit)?) = ResultRow("Up to date", PlateMark.COMPLETE, at, onClick)

@Composable
private fun ResultRow(text: String, mark: PlateMark, at: Long, onClick: (() -> Unit)?) {
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
        Text(formatWhen(at), style = Tm.text.numeric, color = Tm.colors.text2, maxLines = 1)
    }
}

/**
 * A running round. The first line (file count, bytes) is the stable one;
 * the current file name below changes at most once a second.
 */
@Composable
private fun SyncBlock(sync: SyncUi) {
    val c = Tm.colors
    val path = sync.path
    Column(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = TmDimens.regionPad, vertical = TmDimens.space2),
        verticalArrangement = Arrangement.spacedBy(TmDimens.space1_5),
    ) {
        if (path != null) {
            Row(horizontalArrangement = Arrangement.spacedBy(TmDimens.space2), verticalAlignment = Alignment.CenterVertically) {
                Text("Uploading", style = MaterialTheme.typography.bodyMedium, color = c.text)
                Text(
                    "${sync.files} ${if (sync.files == 1) "file" else "files"} · ${formatBytes(sync.bytes)}",
                    style = Tm.text.numeric,
                    color = c.text,
                )
            }
        } else {
            Text("Checking the Ring and local folders…", style = MaterialTheme.typography.bodyMedium, color = c.text)
        }
        // A determinate bar only for files big enough to watch; for small
        // ones it would just flicker.
        val bar = Modifier.fillMaxWidth().height(4.dp)
        if (path != null && sync.total >= BIG_FILE) {
            LinearProgressIndicator(
                progress = { (sync.sent.toFloat() / sync.total).coerceIn(0f, 1f) },
                modifier = bar,
                color = c.accent,
                trackColor = c.line3,
                strokeCap = StrokeCap.Butt,
                gapSize = 0.dp,
                drawStopIndicator = {},
            )
        } else {
            BusyBar()
        }
        if (path != null) {
            Text(
                path.substringAfterLast('/') +
                    if (sync.total >= BIG_FILE) " · ${formatBytes(sync.sent)} / ${formatBytes(sync.total)}" else "",
                style = Tm.text.code,
                color = c.text2,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

private const val BIG_FILE = 4L * 1024 * 1024
