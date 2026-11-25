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
 * on the session in any case.
 */
object SoundListener {
    const val SAMPLE_RATE = 48_000
    private const val CHUNK_SAMPLES = SAMPLE_RATE / 10

    fun available(context: Context): Boolean =
        context.packageManager.hasSystemFeature(PackageManager.FEATURE_MICROPHONE)

    const val PERMISSION = Manifest.permission.RECORD_AUDIO

    private fun source(context: Context): Int {
        val am = context.getSystemService(AudioManager::class.java)
        val unprocessed = am?.getProperty(AudioManager.PROPERTY_SUPPORT_AUDIO_SOURCE_UNPROCESSED) == "true"
        return if (unprocessed) MediaRecorder.AudioSource.UNPROCESSED else MediaRecorder.AudioSource.VOICE_RECOGNITION
    }

    /** Suspends until a pairing code is heard; cancel the coroutine to stop. */
    @SuppressLint("MissingPermission") // The caller requests RECORD_AUDIO first.
    suspend fun listen(context: Context, onLevel: (Float) -> Unit): String = withContext(Dispatchers.IO) {
        val receiver = SoundReceiver(SAMPLE_RATE.toUInt())
        val minBuf = AudioRecord.getMinBufferSize(
            SAMPLE_RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT,
        )
        if (minBuf <= 0) error("This device cannot record 48 kHz audio")
        val record = AudioRecord(
            source(context), SAMPLE_RATE, AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_16BIT, maxOf(minBuf, CHUNK_SAMPLES * 2 * 4),
        )
        if (record.state != AudioRecord.STATE_INITIALIZED) {
            record.release()
            error("Microphone is not available")
        }
        val effects = listOfNotNull(
            if (AcousticEchoCanceler.isAvailable()) AcousticEchoCanceler.create(record.audioSessionId) else null,
            if (NoiseSuppressor.isAvailable()) NoiseSuppressor.create(record.audioSessionId) else null,
            if (AutomaticGainControl.isAvailable()) AutomaticGainControl.create(record.audioSessionId) else null,
        )
        effects.forEach { runCatching { it.enabled = false } }
        try {
            record.startRecording()
            val buf = ByteArray(CHUNK_SAMPLES * 2)
            var code: String? = null
            while (code == null) {
                coroutineContext.ensureActive()
                val n = record.read(buf, 0, buf.size)
                if (n < 0) error("Microphone read failed ($n)")
                if (n == 0) continue
                code = receiver.pushPcm16(if (n == buf.size) buf else buf.copyOf(n))
                onLevel(receiver.level())
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
