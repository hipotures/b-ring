package io.github.hipotures.bring

import android.content.Context
import androidx.core.content.edit
import io.github.hipotures.bring.pairing.MicSource
import io.github.hipotures.bring.pairing.SoundBands
import io.github.hipotures.bring.pairing.SoundDetection
import io.github.hipotures.bring.pairing.SoundSettings
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

    private val _sound = MutableStateFlow(loadSound())
    /** Advanced settings of "Listen" (sound pairing). */
    val sound: StateFlow<SoundSettings> = _sound.asStateFlow()

    fun setSound(s: SoundSettings) {
        sp.edit {
            putString(SOUND_BANDS, s.bands.name)
            putInt(SOUND_CUSTOM_HZ, s.customHz)
            putString(SOUND_DETECTION, s.detection.name)
            putString(SOUND_SOURCE, s.source.name)
            putBoolean(SOUND_RAW, s.rawProcessing)
            putBoolean(SOUND_DIAGNOSTICS, s.diagnostics)
        }
        _sound.value = s
    }

    private fun loadSound(): SoundSettings {
        val d = SoundSettings()
        return SoundSettings(
            bands = SoundBands.entries.firstOrNull { it.name == sp.getString(SOUND_BANDS, null) } ?: d.bands,
            customHz = sp.getInt(SOUND_CUSTOM_HZ, d.customHz),
            detection = SoundDetection.entries.firstOrNull { it.name == sp.getString(SOUND_DETECTION, null) }
                ?: d.detection,
            source = MicSource.entries.firstOrNull { it.name == sp.getString(SOUND_SOURCE, null) } ?: d.source,
            rawProcessing = sp.getBoolean(SOUND_RAW, d.rawProcessing),
            diagnostics = sp.getBoolean(SOUND_DIAGNOSTICS, d.diagnostics),
        )
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
        const val SOUND_BANDS = "sound_bands"
        const val SOUND_CUSTOM_HZ = "sound_custom_hz"
        const val SOUND_DETECTION = "sound_detection"
        const val SOUND_SOURCE = "sound_source"
        const val SOUND_RAW = "sound_raw_processing"
        const val SOUND_DIAGNOSTICS = "sound_diagnostics"
    }
}
