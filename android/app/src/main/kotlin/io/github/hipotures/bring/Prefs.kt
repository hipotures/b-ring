package io.github.hipotures.bring

import android.content.Context
import androidx.core.content.edit
import io.github.hipotures.bring.ui.ThemeChoice
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** Small UI preferences. Sync state lives in the engine's database. */
class Prefs(context: Context) {
    private val sp = context.getSharedPreferences("prefs", Context.MODE_PRIVATE)

    private val _theme = MutableStateFlow(
        ThemeChoice.entries.firstOrNull { it.name == sp.getString(THEME, null) } ?: ThemeChoice.SYSTEM,
    )
    /** Follows the system (Light/Dark) by default. */
    val theme: StateFlow<ThemeChoice> = _theme.asStateFlow()

    fun setTheme(choice: ThemeChoice) {
        sp.edit {
            putString(THEME, choice.name)
            remove(LEGACY_DARK)
        }
        _theme.value = choice
    }

    /** User-chosen offload threshold, or null for the engine default. */
    var offloadThresholdBytes: ULong?
        get() = if (sp.contains(OFFLOAD)) sp.getLong(OFFLOAD, 0).toULong() else null
        set(v) = sp.edit { if (v == null) remove(OFFLOAD) else putLong(OFFLOAD, v.toLong()) }

    private companion object {
        const val THEME = "theme"
        const val LEGACY_DARK = "dark_theme" // before The Mechanics
        const val OFFLOAD = "offload_threshold"
    }
}
