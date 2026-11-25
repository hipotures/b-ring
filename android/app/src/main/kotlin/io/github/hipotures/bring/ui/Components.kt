package io.github.hipotures.bring.ui

import androidx.compose.material3.Icon
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material.icons.outlined.KeyboardArrowUp
import androidx.compose.material.icons.outlined.KeyboardArrowDown
import androidx.compose.material.icons.Icons
import android.content.ClipData
import android.content.ClipboardManager
import android.os.Build
import android.widget.Toast
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsFocusedAsState
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.VerticalDivider
import androidx.compose.material3.minimumInteractiveComponentSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.path
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import io.github.hipotures.bring.ffi.EntryState
import themechanics.compose.Tm
import themechanics.compose.TmDimens
import themechanics.compose.TmTone
import themechanics.compose.tmHardShadow
import themechanics.compose.tone

/** Controls and badges: 2 dp radius. Regions and panels are square. */
val ControlShape = RoundedCornerShape(TmDimens.radiusControl)

/** Level 3 separator: rows inside a list or panel. */
@Composable
fun RowDivider() = HorizontalDivider(thickness = TmDimens.bw3, color = Tm.colors.line3)

/**
 * Level 2 panel: 1 dp `line2` frame, `head2` header band with a 1 dp rule,
 * `surface` body. [trailing] holds a small status plate or a quiet action.
 * With [onToggle] the header folds the body away (chevron at the end).
 */
@Composable
fun Panel(
    modifier: Modifier = Modifier,
    title: String? = null,
    trailing: @Composable RowScope.() -> Unit = {},
    expanded: Boolean = true,
    onToggle: (() -> Unit)? = null,
    content: @Composable ColumnScope.() -> Unit,
) {
    val c = Tm.colors
    Column(
        modifier
            .fillMaxWidth()
            .border(TmDimens.bw2, c.line2, RectangleShape)
            .background(c.surface),
    ) {
        if (title != null) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .background(c.head2)
                    .clickable(enabled = onToggle != null) { onToggle?.invoke() }
                    .heightIn(min = TmDimens.toolbarH)
                    .padding(horizontal = TmDimens.panelPad, vertical = TmDimens.space1),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(TmDimens.space2),
            ) {
                Text(title, style = MaterialTheme.typography.titleMedium, color = c.text, modifier = Modifier.weight(1f))
                trailing()
                if (onToggle != null) {
                    Icon(
                        if (expanded) Icons.Outlined.KeyboardArrowUp else Icons.Outlined.KeyboardArrowDown,
                        if (expanded) "Collapse" else "Expand",
                        Modifier.size(TmDimens.iconLg),
                        tint = c.text2,
                    )
                }
            }
            if (expanded) HorizontalDivider(thickness = TmDimens.bw2, color = c.line2)
        }
        if (expanded) {
            Column(
                Modifier.padding(TmDimens.panelPad),
                verticalArrangement = Arrangement.spacedBy(TmDimens.gapPanel),
                content = content,
            )
        }
    }
}

/**
 * Level 3 group nested in a panel: 1 dp `line3` frame and a `sunken` band
 * with an eyebrow label, never stronger than the panel header above it.
 */
@Composable
fun Group(
    modifier: Modifier = Modifier,
    title: String? = null,
    content: @Composable ColumnScope.() -> Unit,
) {
    val c = Tm.colors
    Column(modifier.fillMaxWidth().border(TmDimens.bw3, c.line3, RectangleShape)) {
        if (title != null) {
            Text(
                title.uppercase(),
                style = Tm.text.eyebrow,
                color = c.text2,
                modifier = Modifier
                    .fillMaxWidth()
                    .background(c.sunken)
                    .padding(horizontal = TmDimens.cellPadX, vertical = TmDimens.space1_5),
            )
            HorizontalDivider(thickness = TmDimens.bw3, color = c.line3)
        }
        Column(content = content)
    }
}

/**
 * A control: 2 dp `ink` frame, 2 dp radius, `surface` fill; [primary] is
 * filled with `accent`. Visually 40 dp tall, 48 dp touch target. Focus is
 * the hard ink "lift", never a ring.
 */
@Composable
fun SharpButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    primary: Boolean = false,
    danger: Boolean = false,
) {
    val c = Tm.colors
    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    val focused by interaction.collectIsFocusedAsState()
    val (fill, frame, fg) = when {
        !enabled -> Triple(c.sunken, c.lineDisabled, c.textDisabled)
        primary && danger -> Triple(c.danger, c.ink, c.textInverse)
        primary -> Triple(if (pressed) c.accentPress else c.accent, c.ink, c.accentText)
        else -> Triple(if (pressed) c.press else c.surface, c.ink, if (danger) c.danger else c.text)
    }
    Box(
        modifier
            .minimumInteractiveComponentSize()
            .then(if (focused && enabled) Modifier.tmHardShadow(c.focus, TmDimens.focusLift) else Modifier)
            .border(TmDimens.bwControl, frame, ControlShape)
            .background(fill, ControlShape)
            .clickable(interaction, indication = null, enabled = enabled, onClick = onClick)
            .heightIn(min = TmDimens.controlLg)
            .padding(horizontal = TmDimens.controlPadXLg),
        contentAlignment = Alignment.Center,
    ) {
        Text(text, style = MaterialTheme.typography.labelLarge, color = fg, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

/**
 * One-of-N choice: 2 dp ink frame, 1 dp ink dividers, the chosen cell in
 * the selection fill. Weight never changes, so nothing shifts. [sub] adds a
 * mono second line (counts).
 */
@Composable
fun <T> Segmented(
    options: List<T>,
    selected: T,
    onSelect: (T) -> Unit,
    label: (T) -> String,
    modifier: Modifier = Modifier,
    sub: ((T) -> String)? = null,
) {
    val c = Tm.colors
    Row(
        modifier
            .fillMaxWidth()
            .height(IntrinsicSize.Min)
            .border(TmDimens.bwControl, c.ink, ControlShape)
            .padding(TmDimens.bwControl),
    ) {
        options.forEachIndexed { i, o ->
            if (i > 0) VerticalDivider(thickness = TmDimens.bw2, color = c.ink)
            Column(
                Modifier
                    .weight(1f)
                    .background(if (o == selected) c.selectBg else c.surface)
                    .clickable { onSelect(o) }
                    .heightIn(min = TmDimens.controlLg)
                    .padding(horizontal = TmDimens.space1, vertical = TmDimens.space1_5),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.Center,
            ) {
                Text(label(o), style = MaterialTheme.typography.labelLarge, color = c.text, maxLines = 1)
                if (sub != null) Text(sub(o), style = Tm.text.numeric, color = c.text2, maxLines = 1)
            }
        }
    }
}

/** `key   value`: the label in `labelMedium`, the value in mono. */
@Composable
fun KeyValue(
    key: String,
    value: String,
    valueColor: Color = Color.Unspecified,
    copy: Boolean = false,
    onEdit: (() -> Unit)? = null,
) {
    val context = LocalContext.current
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(
            key,
            style = MaterialTheme.typography.labelMedium,
            color = Tm.colors.text2,
            modifier = Modifier.widthIn(min = 104.dp).padding(end = TmDimens.space3),
        )
        Text(
            value,
            style = Tm.text.code,
            color = if (valueColor == Color.Unspecified) Tm.colors.text else valueColor,
            modifier = Modifier.weight(1f),
        )
        if (onEdit != null) {
            Icon(
                Icons.Outlined.Edit,
                "Edit $key",
                Modifier
                    .clickable(onClick = onEdit)
                    .padding(start = TmDimens.space2, top = TmDimens.space1, bottom = TmDimens.space1)
                    .size(18.dp),
                tint = Tm.colors.link,
            )
        }
        if (copy) {
            Text(
                "Copy",
                style = MaterialTheme.typography.labelMedium,
                color = Tm.colors.link,
                modifier = Modifier
                    .clickable {
                        context.getSystemService(ClipboardManager::class.java)
                            ?.setPrimaryClip(ClipData.newPlainText(key, value))
                        // Android 13+ confirms copies itself.
                        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
                            Toast.makeText(context, "Copied", Toast.LENGTH_SHORT).show()
                        }
                    }
                    .padding(start = TmDimens.space3, top = 1.dp, bottom = TmDimens.space2),
            )
        }
    }
}

// ---------------------------------------------------------------- badges

/** What a file row shows, in order; see [badgesFor]. */
enum class Badge(val label: String, val tone: TmTone) {
    LOCAL("Local", TmTone.Neutral),
    SYNCED("Synced", TmTone.Success),
    REMOTE("Remote", TmTone.Info),
    UPLOAD("Queued", TmTone.Pending),
    SENDING("Uploading", TmTone.Accent),
    ERROR("Failed", TmTone.Danger),
    HELD("Held", TmTone.Warning),
}

/**
 * The state badge first, then flags: queued for upload (or uploading right
 * now), last upload failed, or held back (`suppressed`: never uploaded again, §5.1).
 */
fun badgesFor(
    state: EntryState,
    pending: Boolean,
    error: String?,
    suppressed: Boolean,
    uploading: Boolean = false,
): List<Badge> =
    buildList {
        add(
            when (state) {
                EntryState.LOCAL_ONLY -> Badge.LOCAL
                EntryState.SYNCED -> Badge.SYNCED
                EntryState.REMOTE_ONLY -> Badge.REMOTE
            },
        )
        if (pending) add(if (uploading) Badge.SENDING else Badge.UPLOAD)
        if (error != null) add(Badge.ERROR)
        if (suppressed) add(Badge.HELD)
    }

@Composable
fun BadgeChip(badge: Badge) = Tag(badge.label, badge.tone)

/**
 * Inline status (rows, lines of text): tone `soft` fill, 1 dp tone `line`
 * frame, tone `fg` mono uppercase text and a marker shape, so state is
 * never shown by color alone.
 */
@Composable
fun Tag(text: String, tone: TmTone) {
    val t = Tm.colors.tone(tone)
    Row(
        Modifier
            .border(TmDimens.bw2, t.line, ControlShape)
            .background(t.soft, ControlShape)
            .padding(horizontal = TmDimens.space1_5, vertical = TmDimens.space0_5),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        ToneMark(tone, t.fg)
        Spacer(Modifier.width(TmDimens.space1))
        Text(text.uppercase(), style = Tm.text.badge, color = t.fg, maxLines = 1)
    }
}

/** Badge marker, about cap height: ■ neutral ✓ success ▲ warning ✕ danger ● info ○ pending ⬚ offline. */
@Composable
private fun ToneMark(tone: TmTone, color: Color, size: Dp = 9.dp) {
    Canvas(Modifier.size(size)) {
        val w = this.size.width
        val sw = w * 0.2f
        when (tone) {
            TmTone.Neutral, TmTone.Accent -> drawRect(color, Offset(w * 0.1f, w * 0.1f), Size(w * 0.8f, w * 0.8f))
            TmTone.Success -> drawPath(
                Path().apply { moveTo(w * 0.08f, w * 0.52f); lineTo(w * 0.38f, w * 0.82f); lineTo(w * 0.94f, w * 0.18f) },
                color, style = Stroke(sw, cap = StrokeCap.Square, join = StrokeJoin.Miter),
            )
            TmTone.Warning -> drawPath(
                Path().apply { moveTo(w / 2, 0f); lineTo(w, w); lineTo(0f, w); close() }, color,
            )
            TmTone.Danger -> {
                drawLine(color, Offset(w * 0.12f, w * 0.12f), Offset(w * 0.88f, w * 0.88f), sw, StrokeCap.Square)
                drawLine(color, Offset(w * 0.88f, w * 0.12f), Offset(w * 0.12f, w * 0.88f), sw, StrokeCap.Square)
            }
            TmTone.Info -> drawCircle(color, w / 2)
            TmTone.Pending -> drawCircle(color, w / 2 - sw / 2, style = Stroke(sw))
            TmTone.Offline -> drawRect(
                color, Offset(sw / 2, sw / 2), Size(w - sw, w - sw),
                style = Stroke(sw, pathEffect = PathEffect.dashPathEffect(floatArrayOf(sw * 1.5f, sw))),
            )
        }
    }
}

// --------------------------------------------------------- status plates

/** Marker of a status plate: ✓ complete, dot active, triangle warning, ! error, ring pending. */
enum class PlateMark { COMPLETE, ACTIVE, WARNING, ERROR, PENDING }

/**
 * Standalone status (top bar, panel head): control height, `ink` frame,
 * `surface` fill; the state is carried by the drawn [mark].
 */
@Composable
fun StatusPlate(
    text: String,
    mark: PlateMark,
    modifier: Modifier = Modifier,
    small: Boolean = false,
    onClick: (() -> Unit)? = null,
) {
    val c = Tm.colors
    Row(
        modifier
            .then(if (onClick != null) Modifier.minimumInteractiveComponentSize() else Modifier)
            .border(TmDimens.bwControl, c.ink, ControlShape)
            .background(c.surface, ControlShape)
            .then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier)
            .heightIn(min = if (small) TmDimens.controlSm else TmDimens.controlMd)
            .padding(horizontal = if (small) TmDimens.controlPadXSm else TmDimens.controlPadXMd),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        PlateMarkIcon(mark, if (small) 14.dp else TmDimens.iconMd)
        Spacer(Modifier.width(TmDimens.space2))
        Text(
            text,
            style = if (small) MaterialTheme.typography.labelMedium else MaterialTheme.typography.labelLarge,
            color = c.text,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
fun PlateMarkIcon(mark: PlateMark, size: Dp) {
    val c = Tm.colors
    val ground = c.surface
    Canvas(Modifier.size(size)) {
        val w = this.size.width
        val r = w / 2
        val sw = w * 0.12f
        when (mark) {
            PlateMark.COMPLETE -> {
                drawCircle(c.success, r)
                drawPath(
                    Path().apply { moveTo(w * 0.27f, w * 0.52f); lineTo(w * 0.44f, w * 0.68f); lineTo(w * 0.74f, w * 0.34f) },
                    ground, style = Stroke(w * 0.14f, cap = StrokeCap.Square, join = StrokeJoin.Miter),
                )
            }
            PlateMark.ACTIVE -> {
                drawCircle(c.info, r - sw / 2, style = Stroke(sw))
                drawCircle(c.info, r * 0.45f)
            }
            PlateMark.WARNING -> {
                drawCircle(c.warning, r - sw / 2, style = Stroke(sw))
                drawPath(
                    Path().apply { moveTo(r, w * 0.24f); lineTo(w * 0.76f, w * 0.7f); lineTo(w * 0.24f, w * 0.7f); close() },
                    c.warning,
                )
            }
            PlateMark.ERROR -> {
                drawCircle(c.danger, r)
                drawExclamation(ground, w)
            }
            PlateMark.PENDING -> drawCircle(c.text3, r - sw / 2, style = Stroke(sw))
        }
    }
}

/** "!" drawn as shapes, with a clear gap between stem and dot. */
private fun DrawScope.drawExclamation(color: Color, w: Float) {
    val sw = w * 0.15f
    drawRoundRect(color, Offset(w / 2 - sw / 2, w * 0.2f), Size(sw, w * 0.36f), CornerRadius(sw / 4))
    drawRect(color, Offset(w / 2 - sw / 2, w * 0.66f), Size(sw, sw))
}

/** An error message: danger text after a solid "!" mark. */
@Composable
fun ErrorLine(text: String) {
    Row(verticalAlignment = Alignment.Top) {
        Box(Modifier.padding(top = 3.dp)) { PlateMarkIcon(PlateMark.ERROR, 14.dp) }
        Spacer(Modifier.width(TmDimens.space2))
        Text(text, color = Tm.colors.danger, style = MaterialTheme.typography.bodyMedium)
    }
}

/** A confirmation message: normal text after a green "✓" mark. */
@Composable
fun InfoLine(text: String) {
    Row(verticalAlignment = Alignment.Top) {
        Box(Modifier.padding(top = 3.dp)) { PlateMarkIcon(PlateMark.COMPLETE, 14.dp) }
        Spacer(Modifier.width(TmDimens.space2))
        Text(text, color = Tm.colors.text, style = MaterialTheme.typography.bodyMedium)
    }
}

/** Outlined sliders glyph for per-Ring configuration (icons-core has no Tune). */
val SlidersGlyph: ImageVector by lazy {
    ImageVector.Builder("sliders", 24.dp, 24.dp, 24f, 24f).apply {
        path(stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Square) {
            // Top slider (knob on left)
            moveTo(3f, 6f); lineTo(7f, 6f)
            moveTo(11f, 6f); lineTo(21f, 6f)
            moveTo(9f, 4f); lineTo(9f, 8f)
            // Middle slider (knob on right)
            moveTo(3f, 12f); lineTo(14f, 12f)
            moveTo(18f, 12f); lineTo(21f, 12f)
            moveTo(16f, 10f); lineTo(16f, 14f)
            // Bottom slider (knob in center)
            moveTo(3f, 18f); lineTo(9f, 18f)
            moveTo(13f, 18f); lineTo(21f, 18f)
            moveTo(11f, 16f); lineTo(11f, 20f)
        }
    }.build()
}

/** A square-cornered folder outline in the Outlined icon style (icons-core has no folder). */
val FolderGlyph: ImageVector by lazy {
    ImageVector.Builder("folder", 24.dp, 24.dp, 24f, 24f).apply {
        path(stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineJoin = StrokeJoin.Miter) {
            moveTo(3f, 5f)
            lineTo(10f, 5f)
            lineTo(12f, 7.5f)
            lineTo(21f, 7.5f)
            lineTo(21f, 19f)
            lineTo(3f, 19f)
            close()
        }
    }.build()
}

/** [FolderGlyph] with a plus inside: "add a watched folder", distinct from the bare Add Ring `+`. */
val NewFolderGlyph: ImageVector by lazy {
    ImageVector.Builder("new_folder", 24.dp, 24.dp, 24f, 24f).apply {
        path(stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineJoin = StrokeJoin.Miter) {
            moveTo(3f, 5f)
            lineTo(10f, 5f)
            lineTo(12f, 7.5f)
            lineTo(21f, 7.5f)
            lineTo(21f, 19f)
            lineTo(3f, 19f)
            close()
            // Plus, centred in the folder body
            moveTo(9f, 13.25f); lineTo(15f, 13.25f)
            moveTo(12f, 10.25f); lineTo(12f, 16.25f)
        }
    }.build()
}

/** Text fields: `inset` well, `lineInput` frame, `ink` when focused (no accent ring). */
@Composable
fun tmFieldColors() = Tm.colors.let { c ->
    OutlinedTextFieldDefaults.colors(
        focusedContainerColor = c.inset,
        unfocusedContainerColor = c.inset,
        focusedBorderColor = c.ink,
        unfocusedBorderColor = c.lineInput,
        focusedLabelColor = c.text,
        unfocusedLabelColor = c.text2,
        cursorColor = c.text,
    )
}

/** Indeterminate progress: `accent` on a `line3` track, square ends, no gap. */
@Composable
fun BusyBar(modifier: Modifier = Modifier) {
    LinearProgressIndicator(
        modifier.fillMaxWidth().height(4.dp),
        color = Tm.colors.accent,
        trackColor = Tm.colors.line3,
        strokeCap = StrokeCap.Butt,
        gapSize = 0.dp,
    )
}

/**
 * Overlay dialog: 2 dp `line1` frame and a hard offset shadow, no soft
 * elevation; the action is a danger button.
 */
@Composable
fun ConfirmDialog(
    title: String,
    text: String,
    action: String,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
) {
    androidx.compose.material3.AlertDialog(
        onDismissRequest = onDismiss,
        // The padding keeps the offset shadow inside the dialog window.
        modifier = Modifier
            .padding(end = TmDimens.shadowOverlay, bottom = TmDimens.shadowOverlay)
            .tmHardShadow(Tm.colors.shadowOverlay, TmDimens.shadowOverlay)
            .border(TmDimens.bw1, Tm.colors.line1, MaterialTheme.shapes.extraLarge),
        containerColor = Tm.colors.surface,
        tonalElevation = 0.dp,
        title = { Text(title) },
        text = { Text(text) },
        confirmButton = { SharpButton(action, danger = true, primary = true, onClick = onConfirm) },
        dismissButton = { SharpButton("Cancel", onClick = onDismiss) },
    )
}
