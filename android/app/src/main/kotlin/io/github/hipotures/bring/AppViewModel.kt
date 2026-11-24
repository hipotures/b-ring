package io.github.hipotures.bring

import kotlinx.coroutines.withContext
import java.io.File
import android.widget.Toast
import android.media.MediaScannerConnection
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
import coil3.SingletonImageLoader
import io.github.hipotures.bring.ffi.Entry
import io.github.hipotures.bring.ffi.RingInfo
import io.github.hipotures.bring.ffi.VerifyReport
import io.github.hipotures.bring.ffi.WatchedFolder
import io.github.hipotures.bring.storage.StorageAccess
import io.github.hipotures.bring.ui.ThemeChoice
import io.github.hipotures.bring.sync.RouteStatus
import io.github.hipotures.bring.sync.SyncScheduler
import io.github.hipotures.bring.sync.SyncWorker
import io.github.hipotures.bring.sync.onUnmeteredWifi
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.FlowPreview
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.conflate
import kotlinx.coroutines.flow.debounce
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.withLock
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

    sealed interface VerifyState {
        data object Idle : VerifyState
        data object Running : VerifyState
        data class Done(val summary: String, val warning: Boolean = false) : VerifyState
        data class Failed(val error: String) : VerifyState
    }

    private val _verify = MutableStateFlow<VerifyState>(VerifyState.Idle)
    val verify: StateFlow<VerifyState> = _verify.asStateFlow()

    private val workInfos = wm.getWorkInfosByTagFlow(SyncScheduler.TAG)
        .stateIn(viewModelScope, SharingStarted.Eagerly, emptyList())

    val sync: StateFlow<SyncUi> = combine(bring.activeSync, workInfos) { live, infos ->
        if (live.running) live else syncUi(infos)
    }
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
        // every 1.5 s, without dropping the last transition), so files turn
        // SYNCED/REMOTE and pick up thumbnails without waiting for the end.
        viewModelScope.launch {
            sync.map { it.path }.distinctUntilChanged().conflate().collect { path ->
                if (path != null) {
                    loadEntries()
                    delay(1_500)
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
                SyncScheduler.watchMedia(getApplication())
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
            // Opening the app means "show me the current state": check now
            // unless a round just ended (LAN only, as in the background).
            if (System.currentTimeMillis() - bring.syncLog.lastChecked.value > 60_000) {
                SyncScheduler.syncSoon(getApplication())
            }
        }
    }

    fun onPaired(ring: RingInfo) {
        _pairing.value = Pairing.Paired(ring)
        SyncScheduler.schedulePeriodic(getApplication())
        SyncScheduler.watchMedia(getApplication())
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
     * Full check & repair: asks the Ring to rescan missing hashes/thumbnails,
     * re-reads the Ring index from seq 0, rescans watched folders, clears
     * empty cached thumbnails and retries any queued uploads.
     */
    fun verifySync() {
        if (_verify.value is VerifyState.Running) return
        _verify.value = VerifyState.Running
        viewModelScope.launch(Dispatchers.IO) {
            val app = getApplication<Application>()
            _verify.value = try {
                bring.syncMutex.withLock {
                    val route = bring.moon.selectRoute(app.onUnmeteredWifi())
                    bring.route.value = RouteStatus.of(route)
                    if (route == null) {
                        VerifyState.Failed("Ring unreachable")
                    } else {
                        File(app.cacheDir, "thumbs").listFiles()
                            ?.filter { it.isFile && it.length() == 0L }
                            ?.forEach { it.delete() }
                        val r = bring.moon.verifySync()
                        SingletonImageLoader.get(app).memoryCache?.clear()
                        runCatching { bring.moon.entries(null) }.onSuccess { _entries.value = it }
                        if (r.queued > 0u) SyncScheduler.syncNow(app)
                        VerifyState.Done(
                            summary = formatVerifyReport(r),
                            warning = r.missingThumbs > 0u || r.queued > 0u,
                        )
                    }
                }
            } catch (e: Exception) {
                VerifyState.Failed(e.describe())
            }
        }
    }

    private fun formatVerifyReport(r: VerifyReport): String = buildList {
        add("${r.totalEntries} files indexed")
        if (r.thumbsUpdated > 0u) add("${r.thumbsUpdated} thumbs refreshed")
        if (r.ringQueued > 0u) add("${r.ringQueued} queued on Ring")
        if (r.missingThumbs > 0u) add("${r.missingThumbs} thumbs still missing")
        val localChanges = r.scan.newFiles + r.scan.changed + r.scan.gone
        if (localChanges > 0u) add("$localChanges local changes found")
        if (r.retried > 0u) add("${r.retried} retries reset")
        if (r.queued > 0u) add("${r.queued} queued to upload")
        if (r.thumbsUpdated == 0u && r.missingThumbs == 0u && localChanges == 0u && r.queued == 0u) {
            add("everything in sync")
        }
    }.joinToString(" · ")

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

    /**
     * Deletes the local copies of synced files; they stay on the Ring and
     * become remote-only. The gallery forgets the deleted paths.
     */
    fun freeLocal(files: List<Entry>) {
        viewModelScope.launch(Dispatchers.IO) {
            val text = try {
                val r = bring.moon.freeLocal(files.map { it.id })
                val paths = files.mapNotNull { it.localPath }.filter { !File(it).exists() }
                if (paths.isNotEmpty()) MediaScannerConnection.scanFile(getApplication(), paths.toTypedArray(), null, null)
                buildString {
                    val n = r.freed.toInt()
                    append("Removed $n ${if (n == 1) "file" else "files"} from the phone (${formatBytes(r.freedBytes)})")
                    if (r.skipped > 0u) append(" · ${r.skipped} skipped: changed or not synced")
                }
            } catch (e: Exception) {
                e.describe()
            }
            loadEntries()
            withContext(Dispatchers.Main) { Toast.makeText(getApplication(), text, Toast.LENGTH_LONG).show() }
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

    fun setTheme(choice: ThemeChoice) = bring.prefs.setTheme(choice)

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
        val running = infos.filter { it.state == WorkInfo.State.RUNNING }
        val active = running.firstOrNull { it.progress.getString(SyncWorker.KEY_PATH) != null }
            ?: running.firstOrNull()
        val p = active?.progress
        return SyncUi(
            running = active != null,
            path = p?.getString(SyncWorker.KEY_PATH),
            sent = p?.getLong(SyncWorker.KEY_SENT, 0) ?: 0,
            total = p?.getLong(SyncWorker.KEY_TOTAL, 0) ?: 0,
            files = p?.getInt(SyncWorker.KEY_FILES, 0) ?: 0,
            bytes = p?.getLong(SyncWorker.KEY_BYTES, 0) ?: 0,
        )
    }
}

private const val TAG = "bring"
