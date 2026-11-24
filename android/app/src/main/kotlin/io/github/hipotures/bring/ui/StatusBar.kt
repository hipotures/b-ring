package io.github.hipotures.bring.ui

import androidx.compose.foundation.background
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.graphics.vector.path
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.Color
import androidx.compose.runtime.getValue
import androidx.compose.foundation.layout.Box
import androidx.compose.animation.core.tween
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.animateColor
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
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
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
 * Level 1 region: `chrome` band with a 2 dp `line1` rule below. Title, a
 * blinking upload mark while a file is on its way to the Ring, route plate
 * and 48 dp icon actions. On the home screen a second line holds the last
 * round's result. Sync never changes the strip's height, so nothing below
 * it jumps.
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
            if (sync.running && sync.path != null) UploadMark(sync, onActivity)
            RoutePlate(route)
            if (onSync != null) ActionIcon(Icons.Outlined.Refresh, "Sync now", onSync, enabled = !sync.running)
            if (onActivity != null) ActionIcon(Icons.AutoMirrored.Outlined.List, "Sync activity", onActivity)
            if (onSettings != null) ActionIcon(Icons.Outlined.Settings, "Settings", onSettings)
            if (onSync == null && onActivity == null && onSettings == null) Spacer(Modifier.width(TmDimens.space3))
        }
        when {
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
 * Square with an up arrow (the shape of Android's upload icon), pulsing
 * between `text` and `info` while a file uploads. Tapping it opens the
 * history where the strip has a history action.
 */
@Composable
private fun UploadMark(sync: SyncUi, onClick: (() -> Unit)?) {
    val c = Tm.colors
    val tint by rememberInfiniteTransition(label = "upload").animateColor(
        initialValue = c.text,
        targetValue = c.info,
        animationSpec = infiniteRepeatable(tween(500), RepeatMode.Reverse),
        label = "tint",
    )
    val what = "Uploading ${sync.files} ${if (sync.files == 1) "file" else "files"}, ${formatBytes(sync.bytes)}"
    Box(
        Modifier
            .size(48.dp)
            .clickable(enabled = onClick != null) { onClick?.invoke() }
            .semantics { contentDescription = what },
        contentAlignment = Alignment.Center,
    ) {
        Icon(UploadGlyph, null, Modifier.size(TmDimens.iconLg), tint = tint)
    }
}

private val UploadGlyph: ImageVector by lazy {
    ImageVector.Builder("upload", 24.dp, 24.dp, 24f, 24f).apply {
        path(stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineJoin = StrokeJoin.Miter) {
            moveTo(4f, 4f); lineTo(20f, 4f); lineTo(20f, 20f); lineTo(4f, 20f); close()
        }
        path(stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineJoin = StrokeJoin.Miter) {
            moveTo(12f, 17f); lineTo(12f, 8f)
            moveTo(8f, 12f); lineTo(12f, 8f); lineTo(16f, 12f)
        }
    }.build()
}
