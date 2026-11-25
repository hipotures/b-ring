package io.github.hipotures.bring.pairing

import android.media.MediaRecorder
import io.github.hipotures.bring.ffi.SoundLimits
import io.github.hipotures.bring.ffi.SoundListenConfig

/** Which bands "Listen" searches (the Ring picks one with `--sound-band`). */
enum class SoundBands(val label: String) {
    BOTH("Low + High"),
    LOW("Low"),
    HIGH("High"),
    CUSTOM("Custom"),
}

/** How eagerly a preamble starts a decode attempt; the CRC still guards the result. */
enum class SoundDetection(val label: String) {
    SENSITIVE("Sensitive"),
    DEFAULT("Default"),
    STRICT("Strict"),
    ;

    fun threshold(limits: SoundLimits): Float = when (this) {
        SENSITIVE -> 0.30f
        DEFAULT -> limits.defaultThreshold
        STRICT -> 0.60f
    }.coerceIn(limits.minThreshold, limits.maxThreshold)
}

/** `AudioRecord` source; [AUTO] is `UNPROCESSED` where supported, else `VOICE_RECOGNITION`. */
enum class MicSource(val label: String, val audioSource: Int?) {
    AUTO("Auto", null),
    UNPROCESSED("Raw", MediaRecorder.AudioSource.UNPROCESSED),
    VOICE_RECOGNITION("Voice", MediaRecorder.AudioSource.VOICE_RECOGNITION),
    MIC("Mic", MediaRecorder.AudioSource.MIC),
    CAMCORDER("Camera", MediaRecorder.AudioSource.CAMCORDER),
}

/**
 * Advanced settings of the sound channel. The defaults suit both
 * `bring pair --sound` and `--sound-band high`; the rest is for tuning.
 */
data class SoundSettings(
    val bands: SoundBands = SoundBands.BOTH,
    /** Lowest tone of the custom band, Hz. */
    val customHz: Int = 12_000,
    val detection: SoundDetection = SoundDetection.DEFAULT,
    val source: MicSource = MicSource.AUTO,
    /** Switch off echo cancellation, noise suppression and AGC (they smear tones). */
    val rawProcessing: Boolean = true,
    /** Per-band detection scores on the pairing screen. */
    val diagnostics: Boolean = false,
) {
    val isDefault: Boolean get() = this == SoundSettings()

    /** Lowest tone of each band to search, Hz. */
    fun bandsHz(limits: SoundLimits): List<UInt> = when (bands) {
        SoundBands.BOTH -> listOf(limits.lowBandHz, limits.highBandHz)
        SoundBands.LOW -> listOf(limits.lowBandHz)
        SoundBands.HIGH -> listOf(limits.highBandHz)
        SoundBands.CUSTOM -> listOf(clampHz(customHz, limits).toUInt())
    }

    fun listenConfig(limits: SoundLimits) = SoundListenConfig(bandsHz(limits), detection.threshold(limits))

    companion object {
        fun clampHz(hz: Int, limits: SoundLimits): Int =
            hz.coerceIn(limits.minBandHz.toInt(), limits.maxBandHz.toInt())

        /** "18000–20953 Hz" for a band starting at [lowHz]. */
        fun bandRange(lowHz: UInt, limits: SoundLimits): String = "$lowHz–${lowHz + limits.bandSpanHz} Hz"
    }
}

/** One band's detection score for the diagnostics view. */
data class BandScore(val lowHz: UInt, val score: Float, val best: Float)
