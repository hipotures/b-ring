package io.github.hipotures.bring.ui

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.KeyboardArrowUp
import androidx.compose.material.icons.filled.Phone
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.path
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.hipotures.bring.ffi.EntryState

/** A bordered, square section with an optional mono caption. */
@Composable
fun Panel(
    modifier: Modifier = Modifier,
    title: String? = null,
    trailing: @Composable RowScope.() -> Unit = {},
    content: @Composable ColumnScope.() -> Unit,
) {
    Column(
        modifier
            .fillMaxWidth()
            .border(Hairline, MaterialTheme.colorScheme.outline, Sharp)
            .background(MaterialTheme.colorScheme.surfaceContainer, Sharp),
    ) {
        if (title != null) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .background(MaterialTheme.colorScheme.surfaceContainerHigh)
                    .heightIn(min = 44.dp)
                    .padding(horizontal = 14.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    title,
                    style = MaterialTheme.typography.titleSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.weight(1f),
                )
                trailing()
            }
            HorizontalDivider(thickness = Hairline, color = MaterialTheme.colorScheme.outline)
        }
        Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            content()
        }
    }
}

/** Square-cornered button, 48dp tall; `primary` fills it, otherwise it is outlined. */
@Composable
fun SharpButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    primary: Boolean = false,
    danger: Boolean = false,
) {
    val padding = PaddingValues(horizontal = 18.dp, vertical = 10.dp)
    val label: @Composable RowScope.() -> Unit = {
        Text(text, style = MaterialTheme.typography.labelLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
    val accent = if (danger) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.primary
    if (primary) {
        Button(
            onClick,
            modifier.heightIn(min = 48.dp),
            enabled = enabled,
            shape = Sharp,
            contentPadding = padding,
            colors = ButtonDefaults.buttonColors(containerColor = accent),
            content = label,
        )
    } else {
        OutlinedButton(
            onClick,
            modifier.heightIn(min = 48.dp),
            enabled = enabled,
            shape = Sharp,
            contentPadding = padding,
            border = BorderStroke(
                Hairline,
                if (enabled) accent else MaterialTheme.colorScheme.outlineVariant,
            ),
            colors = ButtonDefaults.outlinedButtonColors(contentColor = accent),
            content = label,
        )
    }
}

/** `key   value` row; [code] sets the value in monospace (hashes, URLs). */
@Composable
fun KeyValue(key: String, value: String, valueColor: Color = Color.Unspecified, code: Boolean = false) {
    Row(Modifier.fillMaxWidth()) {
        Text(
            key,
            style = MaterialTheme.typography.bodyMedium,
            color = BringTheme.colors.dim,
            modifier = Modifier.widthIn(min = 96.dp).padding(end = 12.dp),
        )
        Text(
            value,
            style = if (code) CodeStyle else MaterialTheme.typography.bodyMedium,
            color = valueColor,
            modifier = Modifier.weight(1f),
        )
    }
}

/** What a file row shows, in order; see [badgesFor]. */
enum class Badge(val label: String) {
    LOCAL("LOCAL"),
    SYNCED("SYNCED"),
    REMOTE("REMOTE"),
    UPLOAD("QUEUED"),
    ERROR("ERROR"),
    HELD("HELD"),
}

/**
 * The state badge first, then flags: queued for upload, last upload failed,
 * or held back (`suppressed`: never uploaded again, §5.1).
 */
fun badgesFor(state: EntryState, pending: Boolean, error: String?, suppressed: Boolean): List<Badge> =
    buildList {
        add(
            when (state) {
                EntryState.LOCAL_ONLY -> Badge.LOCAL
                EntryState.SYNCED -> Badge.SYNCED
                EntryState.REMOTE_ONLY -> Badge.REMOTE
            },
        )
        if (pending) add(Badge.UPLOAD)
        if (error != null) add(Badge.ERROR)
        if (suppressed) add(Badge.HELD)
    }

/** A square "ring": the file lives on the Ring. */
private val RingGlyph: ImageVector by lazy {
    ImageVector.Builder("ring", 24.dp, 24.dp, 24f, 24f).apply {
        path(stroke = SolidColor(Color.Black), strokeLineWidth = 3.5f, strokeLineJoin = StrokeJoin.Miter) {
            moveTo(5f, 5f)
            lineTo(19f, 5f)
            lineTo(19f, 19f)
            lineTo(5f, 19f)
            close()
        }
    }.build()
}

/** A square-cornered folder outline (icons-core has no folder icon). */
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

/** A filled, square, high-contrast badge with an icon and a label. */
@Composable
fun BadgeChip(badge: Badge) {
    val c = BringTheme.colors
    val (fill, icon) = when (badge) {
        Badge.LOCAL -> c.local to Icons.Filled.Phone
        Badge.SYNCED -> c.synced to Icons.Filled.Check
        Badge.REMOTE -> c.remote to RingGlyph
        Badge.UPLOAD -> MaterialTheme.colorScheme.primary to Icons.Filled.KeyboardArrowUp
        Badge.ERROR -> c.warn to Icons.Filled.Warning
        Badge.HELD -> c.dim to null
    }
    Row(
        Modifier
            .background(fill, Sharp)
            .padding(horizontal = 6.dp, vertical = 2.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (icon != null) {
            Icon(icon, null, Modifier.size(14.dp), tint = c.onBadge)
            Spacer(Modifier.width(4.dp))
        }
        Text(
            badge.label,
            color = c.onBadge,
            style = TextStyle(fontSize = 12.sp, fontWeight = FontWeight.Bold, letterSpacing = 0.5.sp),
            maxLines = 1,
        )
    }
}

/** An outlined status tag, e.g. the route (LAN / WAN / OFFLINE). */
@Composable
fun Tag(text: String, color: Color) {
    Box(
        Modifier
            .border(Hairline, color, Sharp)
            .padding(horizontal = 8.dp, vertical = 3.dp),
    ) {
        Text(
            text,
            color = color,
            style = TextStyle(fontSize = 12.sp, fontWeight = FontWeight.Bold, letterSpacing = 0.5.sp),
            maxLines = 1,
            overflow = TextOverflow.Clip,
        )
    }
}

@Composable
fun ErrorLine(text: String) {
    Text(
        text,
        color = MaterialTheme.colorScheme.error,
        style = MaterialTheme.typography.bodyMedium,
    )
}
