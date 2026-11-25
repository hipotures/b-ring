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

    private val _demoMode = MutableStateFlow(sp.getBoolean(DEMO_MODE, false))
    /** Shows synthetic sample Rings, folders and activity without touching real data. */
    val demoMode: StateFlow<Boolean> = _demoMode.asStateFlow()

    fun setDemoMode(enabled: Boolean) {
        sp.edit { putBoolean(DEMO_MODE, enabled) }
        _demoMode.value = enabled
    }

    /** User-chosen fallback offload threshold, or null for the engine default. */
    var offloadThresholdBytes: ULong?
        get() = if (sp.contains(OFFLOAD)) sp.getLong(OFFLOAD, 0).toULong() else null
        set(v) = sp.edit { if (v == null) remove(OFFLOAD) else putLong(OFFLOAD, v.toLong()) }

    /** Last selected Ring in the UI (hex ring_id). */
    var selectedRingId: String?
        get() = sp.getString(SELECTED_RING, null)
        set(v) = sp.edit { if (v == null) remove(SELECTED_RING) else putString(SELECTED_RING, v) }

    /** Last destination folder chosen in the Share Sheet. */
    var lastShareFolder: String?
        get() = sp.getString(LAST_SHARE_FOLDER, null)
        set(v) = sp.edit { if (v.isNullOrBlank()) remove(LAST_SHARE_FOLDER) else putString(LAST_SHARE_FOLDER, v) }

    private companion object {
        const val THEME = "theme"
        const val LEGACY_DARK = "dark_theme" // before The Mechanics
        const val OFFLOAD = "offload_threshold"
        const val SELECTED_RING = "selected_ring_id"
        const val LAST_SHARE_FOLDER = "last_share_folder"
        const val DEMO_MODE = "demo_mode"
    }
}
