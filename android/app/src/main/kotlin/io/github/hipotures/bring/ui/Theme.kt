package io.github.hipotures.bring.ui

import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/** Technical minimalism: sharp 2dp corners, 1dp rules, dense type. */
val Sharp = RoundedCornerShape(2.dp)
val Hairline = 1.dp
val Mono = FontFamily.Monospace

/** Colours outside Material's scheme: file-state badges and status text. */
@Immutable
data class BringColors(
    val local: Color,
    val synced: Color,
    val remote: Color,
    /** Text drawn on a filled badge. */
    val onBadge: Color,
    val warn: Color,
    val dim: Color,
)

private val DarkBring = BringColors(
    local = Color(0xFFF5A524),
    synced = Color(0xFF2FD07F),
    remote = Color(0xFF5B9BFF),
    onBadge = Color(0xFF05070A),
    warn = Color(0xFFFF6B5E),
    dim = Color(0xFF9AA3AE),
)

private val LightBring = BringColors(
    local = Color(0xFF9A5B00),
    synced = Color(0xFF0E7A45),
    remote = Color(0xFF1F55C8),
    onBadge = Color(0xFFFFFFFF),
    warn = Color(0xFFC62828),
    dim = Color(0xFF4A525C),
)

private val DarkScheme: ColorScheme = darkColorScheme(
    primary = Color(0xFF3FB6FF),
    onPrimary = Color(0xFF001522),
    secondary = Color(0xFF9AA3AE),
    onSecondary = Color(0xFF05070A),
    background = Color(0xFF0B0D10),
    onBackground = Color(0xFFE8EBF0),
    surface = Color(0xFF0B0D10),
    onSurface = Color(0xFFE8EBF0),
    surfaceVariant = Color(0xFF151A20),
    onSurfaceVariant = Color(0xFFB8C0CA),
    surfaceContainer = Color(0xFF12161B),
    surfaceContainerHigh = Color(0xFF181D24),
    outline = Color(0xFF3A434E),
    outlineVariant = Color(0xFF262C34),
    error = Color(0xFFFF6B5E),
    onError = Color(0xFF1A0000),
)

private val LightScheme: ColorScheme = lightColorScheme(
    primary = Color(0xFF005FA3),
    onPrimary = Color(0xFFFFFFFF),
    secondary = Color(0xFF4A525C),
    onSecondary = Color(0xFFFFFFFF),
    background = Color(0xFFF4F6F8),
    onBackground = Color(0xFF0B0D10),
    surface = Color(0xFFF4F6F8),
    onSurface = Color(0xFF0B0D10),
    surfaceVariant = Color(0xFFFFFFFF),
    onSurfaceVariant = Color(0xFF2E353D),
    surfaceContainer = Color(0xFFFFFFFF),
    surfaceContainerHigh = Color(0xFFECEFF3),
    outline = Color(0xFF8A939E),
    outlineVariant = Color(0xFFC9D0D8),
    error = Color(0xFFC62828),
    onError = Color(0xFFFFFFFF),
)

private val SharpShapes = Shapes(
    extraSmall = Sharp,
    small = Sharp,
    medium = Sharp,
    large = Sharp,
    extraLarge = Sharp,
)

private val DenseType = Typography().run {
    copy(
        titleLarge = titleLarge.copy(fontSize = 18.sp, fontWeight = FontWeight.SemiBold),
        titleMedium = titleMedium.copy(fontSize = 15.sp, fontWeight = FontWeight.SemiBold),
        bodyLarge = bodyLarge.copy(fontSize = 15.sp),
        bodyMedium = bodyMedium.copy(fontSize = 14.sp),
        labelLarge = labelLarge.copy(fontFamily = Mono, fontSize = 13.sp, letterSpacing = 0.5.sp),
        labelMedium = labelMedium.copy(fontFamily = Mono, fontSize = 12.sp),
        labelSmall = labelSmall.copy(fontFamily = Mono, fontSize = 11.sp),
    )
}

/** Monospace metadata line (sizes, dates, hashes). */
val MetaStyle = TextStyle(fontFamily = Mono, fontSize = 11.sp, letterSpacing = 0.sp)

val LocalBringColors = staticCompositionLocalOf { DarkBring }

@Composable
fun BringTheme(dark: Boolean, content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalBringColors provides if (dark) DarkBring else LightBring) {
        MaterialTheme(
            colorScheme = if (dark) DarkScheme else LightScheme,
            shapes = SharpShapes,
            typography = DenseType,
            content = content,
        )
    }
}

object BringTheme {
    val colors: BringColors
        @Composable get() = LocalBringColors.current
}
