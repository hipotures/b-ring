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

class PairingViewModel(app: Application) : AndroidViewModel(app) {
    private val bring = app.bring
    private val _state = MutableStateFlow<PairState>(PairState.Idle)
    val state: StateFlow<PairState> = _state.asStateFlow()
    private var job: Job? = null

    fun reset() {
        job?.cancel()
        job = null
        _state.value = PairState.Idle
    }

    fun start(code: String, deviceName: String, ringLabel: String = "") {
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
