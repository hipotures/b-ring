package io.github.hipotures.bring

import android.app.Application
import android.net.ConnectivityManager
import android.os.SystemClock
import android.util.Log
import android.net.Network
import android.net.NetworkCapabilities
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import androidx.work.WorkInfo
import androidx.work.WorkManager
import io.github.hipotures.bring.ffi.Entry
import io.github.hipotures.bring.ffi.RingInfo
import io.github.hipotures.bring.ffi.WatchedFolder
import io.github.hipotures.bring.storage.StorageAccess
import io.github.hipotures.bring.sync.RouteStatus
import io.github.hipotures.bring.sync.SyncScheduler
import io.github.hipotures.bring.sync.SyncWorker
import io.github.hipotures.bring.sync.onUnmeteredWifi
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.FlowPreview
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.debounce
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import java.util.UUID

/** What the status bar shows about sync. */
data class SyncUi(
    val running: Boolean = false,
    val path: String? = null,
    val sent: Long = 0,
    val total: Long = 0,
    /** Files uploaded or uploading in this round. */
    val files: Int = 0,
    /** Bytes sent in this round, all files together. */
    val bytes: Long = 0,
)

/** App-wide state: pairing, route, sync, storage and watched folders. */
@OptIn(FlowPreview::class)
class AppViewModel(app: Application) : AndroidViewModel(app) {
    private val bring = app.bring
    private val wm = WorkManager.getInstance(app)

    sealed interface Pairing {
        data object Loading : Pairing
        data object Unpaired : Pairing
        data class Paired(val ring: RingInfo) : Pairing
    }

    private val _pairing = MutableStateFlow<Pairing>(Pairing.Loading)
    val pairing: StateFlow<Pairing> = _pairing.asStateFlow()

    val route: StateFlow<RouteStatus> = bring.route.asStateFlow()

    private val _storageGranted = MutableStateFlow(StorageAccess.granted())
    val storageGranted: StateFlow<Boolean> = _storageGranted.asStateFlow()

    private val _folders = MutableStateFlow<List<WatchedFolder>>(emptyList())
    val folders: StateFlow<List<WatchedFolder>> = _folders.asStateFlow()

    private val _entries = MutableStateFlow<List<Entry>>(emptyList())
    /** Every entry the Moon knows (local, synced and remote-only). */
    val entries: StateFlow<List<Entry>> = _entries.asStateFlow()

    private val _message = MutableStateFlow<String?>(null)
    /** A one-line result of the last user action (errors included). */
    val message: StateFlow<String?> = _message.asStateFlow()

    private val workInfos = wm.getWorkInfosByTagFlow(SyncScheduler.TAG)
        .stateIn(viewModelScope, SharingStarted.Eagerly, emptyList())

    val sync: StateFlow<SyncUi> = workInfos
        .map(::syncUi)
        .distinctUntilChanged()
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), SyncUi())

    private val _syncError = MutableStateFlow<String?>(null)
    /** Error of the last round that ended while the app watched, if it failed. */
    val syncError: StateFlow<String?> = _syncError.asStateFlow()

    /** Finished rounds, newest first, and the end of the last check. */
    val rounds = app.bring.syncLog.rounds
    val lastChecked = app.bring.syncLog.lastChecked

    /** Bumped when a sync round ends, so lists reload. */
    val syncFinished = MutableStateFlow(0)

    private val networkChanged = MutableSharedFlow<Unit>(extraBufferCapacity = 1)
    private val networkCallback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) { networkChanged.tryEmit(Unit) }
        override fun onLost(network: Network) { networkChanged.tryEmit(Unit) }
        override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) {
            networkChanged.tryEmit(Unit)
        }
    }

    init {
        refresh()
        app.getSystemService(ConnectivityManager::class.java)
            ?.registerDefaultNetworkCallback(networkCallback)
        viewModelScope.launch {
            networkChanged.debounce(1_000).collect { probeRoute() }
        }
        viewModelScope.launch { trackRounds() }
        viewModelScope.launch { syncFinished.collect { loadEntries() } }
        // During a long round, reload the list as uploads move on (at most
        // every 3 s), so files turn SYNCED without waiting for the end.
        viewModelScope.launch {
            var lastAt = 0L
            sync.map { it.path }.distinctUntilChanged().collect { path ->
                val now = SystemClock.elapsedRealtime()
                if (path != null && now - lastAt > 3_000) {
                    lastAt = now
                    loadEntries()
                }
            }
        }
    }

    override fun onCleared() {
        getApplication<Application>().getSystemService(ConnectivityManager::class.java)
            ?.unregisterNetworkCallback(networkCallback)
    }

    fun refresh() {
        viewModelScope.launch(Dispatchers.IO) {
            val ring = runCatching { bring.moon.ring() }.getOrNull()
            // Without its token (e.g. Keystore reset) the Moon must pair again;
            // re-pairing with the same Ring keeps the index.
            if (ring != null && bring.tokens.load() != null) {
                _pairing.value = Pairing.Paired(ring)
                SyncScheduler.schedulePeriodic(getApplication())
                loadFolders()
                loadEntries()
                probeRoute()
            } else {
                _pairing.value = Pairing.Unpaired
            }
        }
    }

    /** Called on resume: the user may have granted access in Settings. */
    fun onResume() {
        _storageGranted.value = StorageAccess.granted()
        if (_pairing.value is Pairing.Paired) {
            networkChanged.tryEmit(Unit)
            loadEntries()
        }
    }

    fun onPaired(ring: RingInfo) {
        _pairing.value = Pairing.Paired(ring)
        SyncScheduler.schedulePeriodic(getApplication())
        // First listing of the Ring right away, over whichever route works.
        SyncScheduler.syncNow(getApplication())
        probeRoute()
    }

    fun loadEntries() {
        viewModelScope.launch(Dispatchers.IO) {
            if (_pairing.value !is Pairing.Paired) return@launch
            runCatching { bring.moon.entries(null) }.onSuccess { _entries.value = it }
        }
    }

    private fun probeRoute() {
        viewModelScope.launch(Dispatchers.IO) {
            if (_pairing.value !is Pairing.Paired) return@launch
            val wifi = getApplication<Application>().onUnmeteredWifi()
            val t0 = SystemClock.elapsedRealtime()
            val result = runCatching { RouteStatus.of(bring.moon.selectRoute(wifi)) }
            Log.i(TAG, "route: unmeteredWifi=$wifi -> $result in ${SystemClock.elapsedRealtime() - t0} ms")
            bring.route.value = result.getOrDefault(RouteStatus.OFFLINE)
        }
    }

    fun recheckRoute() {
        bring.route.value = RouteStatus.UNKNOWN
        probeRoute()
    }

    fun syncNow() {
        _syncError.value = null
        SyncScheduler.syncNow(getApplication())
    }

    /**
     * Detects the end of each round from state transitions seen in this
     * session: a one-time request reaching a final state, or the periodic
     * request going from RUNNING back to ENQUEUED. Old finished requests
     * still in WorkManager's history are ignored.
     */
    private suspend fun trackRounds() {
        val active = mutableSetOf<UUID>()
        val running = mutableSetOf<UUID>()
        workInfos.collect { infos ->
            var ended = false
            for (i in infos) {
                when {
                    i.state == WorkInfo.State.RUNNING -> {
                        active += i.id
                        running += i.id
                    }
                    i.state.isFinished -> if (active.remove(i.id)) {
                        running.remove(i.id)
                        ended = true
                        _syncError.value = i.outputData.getString(SyncWorker.KEY_ERROR)
                    }
                    else -> {
                        if (running.remove(i.id)) ended = true
                        active += i.id
                    }
                }
            }
            if (ended) syncFinished.value++
        }
    }

    fun cancelSync() {
        wm.cancelUniqueWork(SyncScheduler.MANUAL)
    }

    private fun loadFolders() {
        _folders.value = runCatching { bring.moon.watchedFolders() }.getOrDefault(emptyList())
    }

    fun addFolder(localDir: String) = action {
        bring.moon.addWatchedFolder(localDir, null)
        loadFolders()
        "Watching $localDir"
    }

    fun removeFolder(id: Long) = action {
        bring.moon.removeWatchedFolder(id)
        loadFolders()
        null
    }

    fun setOffloadThreshold(bytes: ULong) = action {
        bring.prefs.offloadThresholdBytes = bytes
        bring.moon.setOffloadThreshold(bytes)
        null
    }

    fun setDarkTheme(dark: Boolean) = bring.prefs.setDarkTheme(dark)

    /** Forgets the Ring and its index; local files are not touched. */
    fun forget() = action {
        SyncScheduler.cancelAll(getApplication())
        bring.moon.forgetRing()
        bring.tokens.clear()
        bring.route.value = RouteStatus.UNKNOWN
        _folders.value = emptyList()
        _entries.value = emptyList()
        _pairing.value = Pairing.Unpaired
        null
    }

    fun clearMessage() {
        _message.value = null
    }

    /** Runs [block] on IO; its result (or error) becomes [message]. */
    private fun action(block: suspend () -> String?) {
        viewModelScope.launch(Dispatchers.IO) {
            _message.value = try {
                block()
            } catch (e: Exception) {
                e.describe()
            }
        }
    }

    private fun syncUi(infos: List<WorkInfo>): SyncUi {
        val running = infos.firstOrNull { it.state == WorkInfo.State.RUNNING }
        val p = running?.progress
        return SyncUi(
            running = running != null,
            path = p?.getString(SyncWorker.KEY_PATH),
            sent = p?.getLong(SyncWorker.KEY_SENT, 0) ?: 0,
            total = p?.getLong(SyncWorker.KEY_TOTAL, 0) ?: 0,
            files = p?.getInt(SyncWorker.KEY_FILES, 0) ?: 0,
            bytes = p?.getLong(SyncWorker.KEY_BYTES, 0) ?: 0,
        )
    }
}

private const val TAG = "bring"
