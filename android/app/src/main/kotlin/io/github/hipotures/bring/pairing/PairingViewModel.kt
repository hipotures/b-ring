package io.github.hipotures.bring.pairing

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import io.github.hipotures.bring.bring
import io.github.hipotures.bring.describe
import io.github.hipotures.bring.ffi.MoonException
import io.github.hipotures.bring.ffi.RingInfo
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

sealed interface PairState {
    data object Idle : PairState
    data object Connecting : PairState
    /** Compare [sas] with the Ring, then approve there. */
    data class Verify(val sas: String, val ringUrl: String, val note: String? = null) : PairState
    data class Done(val ring: RingInfo) : PairState
    data class Failed(val message: String) : PairState
}

/** A code channel other than the camera or the keyboard (sound, Bluetooth). */
sealed interface ChannelState {
    data object Idle : ChannelState
    /**
     * [level] is the microphone RMS in `[0, 1]`; [source] the `AudioRecord`
     * source in use; [bands] the per-band detection scores (diagnostics).
     */
    data class Listening(
        val level: Float,
        val source: String = "",
        val bands: List<BandScore> = emptyList(),
    ) : ChannelState
    data class Bluetooth(val status: String) : ChannelState
    data class Failed(val message: String) : ChannelState
}

/** A code a channel delivered, and that channel's name for the UI. */
data class ReceivedCode(val code: String, val via: String)

class PairingViewModel(app: Application) : AndroidViewModel(app) {
    private val bring = app.bring
    private val _state = MutableStateFlow<PairState>(PairState.Idle)
    val state: StateFlow<PairState> = _state.asStateFlow()
    private var job: Job? = null

    private val _channel = MutableStateFlow<ChannelState>(ChannelState.Idle)
    val channel: StateFlow<ChannelState> = _channel.asStateFlow()
    /** A code delivered by a channel, for the screen to put into the field. */
    private val _received = MutableStateFlow<ReceivedCode?>(null)
    val received: StateFlow<ReceivedCode?> = _received.asStateFlow()
    private var channelJob: Job? = null
    /** Last sound diagnostics, kept after "Listen" ends so they can be read. */
    private val _lastSound = MutableStateFlow<ChannelState.Listening?>(null)
    val lastSound: StateFlow<ChannelState.Listening?> = _lastSound.asStateFlow()

    /** Listens for `bring pair --sound` (RECORD_AUDIO must be granted). */
    fun listen() = runChannel(ChannelState.Listening(0f), "sound") {
        _lastSound.value = null
        SoundListener.listen(getApplication(), bring.prefs.sound.value) { p ->
            if (_channel.value is ChannelState.Listening) {
                val now = ChannelState.Listening(p.level, p.source, p.bands)
                _channel.value = now
                _lastSound.value = now
            }
        }
    }

    /** Reads the code from `bring pair --ble` (Bluetooth permissions must be granted). */
    fun readBluetooth() = runChannel(ChannelState.Bluetooth("Starting…"), "Bluetooth") {
        BlePairingReader.read(getApplication()) { _channel.value = ChannelState.Bluetooth(it) }
    }

    private fun runChannel(initial: ChannelState, via: String, block: suspend () -> String) {
        channelJob?.cancel()
        _channel.value = initial
        channelJob = viewModelScope.launch {
            try {
                _received.value = ReceivedCode(block(), via)
                _channel.value = ChannelState.Idle
            } catch (e: kotlinx.coroutines.CancellationException) {
                throw e
            } catch (e: Exception) {
                _channel.value = ChannelState.Failed(e.message ?: e.toString())
            }
        }
    }

    fun stopChannel() {
        channelJob?.cancel()
        channelJob = null
        _channel.value = ChannelState.Idle
    }

    fun channelFailed(message: String) {
        stopChannel()
        _channel.value = ChannelState.Failed(message)
    }

    fun consumeReceived() {
        _received.value = null
    }

    fun reset() {
        job?.cancel()
        job = null
        _state.value = PairState.Idle
    }

    fun start(code: String, deviceName: String, ringLabel: String = "") {
        stopChannel()
        job?.cancel()
        job = viewModelScope.launch(Dispatchers.IO) {
            _state.value = PairState.Connecting
            val moon = bring.moon
            val sas = try {
                moon.pairStart(code, deviceName.trim().ifEmpty { "moon" })
            } catch (e: Exception) {
                _state.value = PairState.Failed(e.describe())
                return@launch
            }
            var verify = PairState.Verify(sas.sas, sas.ringUrl)
            _state.value = verify
            var networkErrors = 0
            while (isActive) {
                try {
                    val paired = moon.pairFinish(POLL_SECS)
                    bring.tokens.save(paired.ring.ringId, paired.deviceToken)
                    val label = ringLabel.trim()
                    val finalRing = if (label.isNotEmpty()) {
                        runCatching { moon.renameRing(paired.ring.ringId, label) }.getOrDefault(paired.ring)
                    } else {
                        paired.ring
                    }
                    _state.value = PairState.Done(finalRing)
                    return@launch
                } catch (_: MoonException.PairingPending) {
                    networkErrors = 0
                    if (verify.note != null) {
                        verify = verify.copy(note = null)
                        _state.value = verify
                    }
                } catch (e: MoonException.Network) {
                    // The session survives network errors; keep polling a while.
                    if (++networkErrors > MAX_NETWORK_ERRORS) {
                        _state.value = PairState.Failed(e.describe())
                        return@launch
                    }
                    verify = verify.copy(note = "Ring unreachable, retrying…")
                    _state.value = verify
                    delay(2_000)
                } catch (e: Exception) {
                    _state.value = PairState.Failed(e.describe())
                    return@launch
                }
            }
        }
    }

    fun cancel() {
        job?.cancel()
        job = null
        _state.value = PairState.Idle
        viewModelScope.launch(Dispatchers.IO) { bring.moon.pairCancel() }
    }

    override fun onCleared() {
        val s = _state.value
        if (s is PairState.Connecting || s is PairState.Verify) bring.moon.pairCancel()
    }

    private companion object {
        const val POLL_SECS = 5u
        const val MAX_NETWORK_ERRORS = 10
    }
}
