package io.github.hipotures.bring.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.runtime.Composable
import themechanics.compose.TheMechanicsTheme
import themechanics.compose.TmThemeName

/**
 * The user's theme choice. Colors, type, shapes and geometry all come from
 * The Mechanics (third_party/the-mechanics, package themechanics.compose);
 * this app defines no palette of its own.
 */
enum class ThemeChoice(val label: String) {
    SYSTEM("System"),
    LIGHT("Light"),
    DARK("Dark"),
    COFFEE("Coffee"),
}

/** Light and Dark follow the system unless forced; Coffee is explicit. */
@Composable
fun ThemeChoice.resolve(): TmThemeName = when (this) {
    ThemeChoice.SYSTEM -> if (isSystemInDarkTheme()) TmThemeName.Dark else TmThemeName.Light
    ThemeChoice.LIGHT -> TmThemeName.Light
    ThemeChoice.DARK -> TmThemeName.Dark
    ThemeChoice.COFFEE -> TmThemeName.Coffee
}

@Composable
fun BringTheme(choice: ThemeChoice, content: @Composable () -> Unit) {
    TheMechanicsTheme(theme = choice.resolve(), content = content)
}
