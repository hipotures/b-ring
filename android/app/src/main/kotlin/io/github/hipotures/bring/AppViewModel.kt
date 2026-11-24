package io.github.hipotures.bring

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import io.github.hipotures.bring.ffi.RingInfo
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/** App-wide state: which Ring (if any) this Moon is paired with. */
class AppViewModel(app: Application) : AndroidViewModel(app) {
    private val bring = app.bring

    sealed interface Pairing {
        data object Loading : Pairing
        data object Unpaired : Pairing
        data class Paired(val ring: RingInfo) : Pairing
    }

    private val _pairing = MutableStateFlow<Pairing>(Pairing.Loading)
    val pairing: StateFlow<Pairing> = _pairing.asStateFlow()

    init {
        refresh()
    }

    fun refresh() {
        viewModelScope.launch(Dispatchers.IO) {
            val ring = runCatching { bring.moon.ring() }.getOrNull()
            // Without its token (e.g. Keystore reset) the Moon must pair again;
            // re-pairing with the same Ring keeps the index.
            _pairing.value = if (ring != null && bring.tokens.load() != null) {
                Pairing.Paired(ring)
            } else {
                Pairing.Unpaired
            }
        }
    }

    fun onPaired(ring: RingInfo) {
        _pairing.value = Pairing.Paired(ring)
    }

    /** Forgets the Ring and its index; local files are not touched. */
    fun forget() {
        viewModelScope.launch(Dispatchers.IO) {
            bring.moon.forgetRing()
            bring.tokens.clear()
            _pairing.value = Pairing.Unpaired
        }
    }
}
