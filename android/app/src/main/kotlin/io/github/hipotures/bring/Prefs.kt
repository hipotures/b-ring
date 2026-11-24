package io.github.hipotures.bring

import android.content.Context
import androidx.core.content.edit
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** Small UI preferences. Sync state lives in the engine's database. */
class Prefs(context: Context) {
    private val sp = context.getSharedPreferences("prefs", Context.MODE_PRIVATE)

    private val _dark = MutableStateFlow(sp.getBoolean(DARK, true))
    /** Dark (high contrast) by default. */
    val darkTheme: StateFlow<Boolean> = _dark.asStateFlow()

    fun setDarkTheme(dark: Boolean) {
        sp.edit { putBoolean(DARK, dark) }
        _dark.value = dark
    }

    /** User-chosen offload threshold, or null for the engine default. */
    var offloadThresholdBytes: ULong?
        get() = if (sp.contains(OFFLOAD)) sp.getLong(OFFLOAD, 0).toULong() else null
        set(v) = sp.edit { if (v == null) remove(OFFLOAD) else putLong(OFFLOAD, v.toLong()) }

    private companion object {
        const val DARK = "dark_theme"
        const val OFFLOAD = "offload_threshold"
    }
}
