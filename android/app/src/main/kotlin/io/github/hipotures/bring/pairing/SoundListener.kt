package io.github.hipotures.bring.pairing

import android.Manifest
import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioRecord
import android.media.MediaRecorder
import android.media.audiofx.AcousticEchoCanceler
import android.media.audiofx.AutomaticGainControl
import android.media.audiofx.NoiseSuppressor
import io.github.hipotures.bring.ffi.SoundReceiver
import io.github.hipotures.bring.ffi.soundLimits
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import kotlin.coroutines.coroutineContext

/**
 * Listens for `bring pair --sound` (docs/ARCHITECTURE.md §3.6): microphone PCM
 * goes into the engine's [SoundReceiver] until a frame decodes.
 *
 * Voice processing smears tones, so the raw `UNPROCESSED` source is used
 * where the device has one (`VOICE_RECOGNITION` otherwise, which is meant to
 * skip it), and echo cancellation, noise suppression and AGC are switched off
 * on the session. [SoundSettings] can override all of that, and the bands.
 */
object SoundListener {
    const val SAMPLE_RATE = 48_000
    private const val CHUNK_SAMPLES = SAMPLE_RATE / 10
    /** Per 100 ms chunk: a peak fades to a tenth in ~2 s. */
    private const val SCORE_DECAY = 0.89f

    fun available(context: Context): Boolean =
        context.packageManager.hasSystemFeature(PackageManager.FEATURE_MICROPHONE)

    const val PERMISSION = Manifest.permission.RECORD_AUDIO

    /** The `AudioRecord` source for [choice] and its name for the UI. */
    private fun source(context: Context, choice: MicSource): Pair<Int, String> {
        choice.audioSource?.let { return it to choice.name.lowercase() }
        val am = context.getSystemService(AudioManager::class.java)
        val unprocessed = am?.getProperty(AudioManager.PROPERTY_SUPPORT_AUDIO_SOURCE_UNPROCESSED) == "true"
        return if (unprocessed) {
            MediaRecorder.AudioSource.UNPROCESSED to "unprocessed"
        } else {
            MediaRecorder.AudioSource.VOICE_RECOGNITION to "voice_recognition"
        }
    }

    /** What the microphone hears right now, for the level meter and diagnostics. */
    data class Progress(val level: Float, val source: String, val bands: List<BandScore>, val frames: Int)

    /**
     * Suspends until a pairing code is heard; cancel the coroutine to stop.
     * With [onCode] it never returns: every decoded frame goes there and
     * listening goes on (diagnostics).
     */
    @SuppressLint("MissingPermission") // The caller requests RECORD_AUDIO first.
    suspend fun listen(
        context: Context,
        settings: SoundSettings,
        onCode: ((String) -> Unit)? = null,
        onProgress: (Progress) -> Unit,
    ): String = withContext(Dispatchers.IO) {
        val receiver = SoundReceiver.withConfig(SAMPLE_RATE.toUInt(), settings.listenConfig(soundLimits()))
        val bandsHz = receiver.bandsHz()
        val shown = FloatArray(bandsHz.size)
        val best = FloatArray(bandsHz.size)
        val (audioSource, sourceName) = source(context, settings.source)
        val minBuf = AudioRecord.getMinBufferSize(
            SAMPLE_RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT,
        )
        if (minBuf <= 0) {
            receiver.close()
            error("This device cannot record 48 kHz audio")
        }
        val record = AudioRecord(
            audioSource, SAMPLE_RATE, AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_16BIT, maxOf(minBuf, CHUNK_SAMPLES * 2 * 4),
        )
        if (record.state != AudioRecord.STATE_INITIALIZED) {
            record.release()
            receiver.close()
            error("Microphone is not available (source: $sourceName)")
        }
        val effects = if (settings.rawProcessing) {
            listOfNotNull(
                if (AcousticEchoCanceler.isAvailable()) AcousticEchoCanceler.create(record.audioSessionId) else null,
                if (NoiseSuppressor.isAvailable()) NoiseSuppressor.create(record.audioSessionId) else null,
                if (AutomaticGainControl.isAvailable()) AutomaticGainControl.create(record.audioSessionId) else null,
            )
        } else {
            emptyList()
        }
        effects.forEach { runCatching { it.enabled = false } }
        try {
            record.startRecording()
            val buf = ByteArray(CHUNK_SAMPLES * 2)
            var code: String? = null
            var frames = 0
            while (code == null) {
                coroutineContext.ensureActive()
                val n = record.read(buf, 0, buf.size)
                if (n < 0) error("Microphone read failed ($n)")
                if (n == 0) continue
                code = receiver.pushPcm16(if (n == buf.size) buf else buf.copyOf(n))
                if (code != null) frames++
                receiver.takeScores().forEachIndexed { i, s ->
                    // Hold peaks briefly so a 64 ms preamble stays readable.
                    shown[i] = maxOf(s, shown[i] * SCORE_DECAY)
                    best[i] = maxOf(best[i], s)
                }
                onProgress(
                    Progress(
                        receiver.level(),
                        sourceName,
                        bandsHz.mapIndexed { i, hz -> BandScore(hz, shown[i], best[i]) },
                        frames,
                    ),
                )
                if (code != null && onCode != null) {
                    onCode(code)
                    code = null
                }
            }
            code
        } finally {
            runCatching { record.stop() }
            record.release()
            effects.forEach { it.release() }
            receiver.close()
        }
    }
}
