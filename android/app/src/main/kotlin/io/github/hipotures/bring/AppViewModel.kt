package io.github.hipotures.bring

import android.app.Application
import android.media.MediaScannerConnection
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.os.SystemClock
import android.util.Log
import android.widget.Toast
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import androidx.work.WorkInfo
import androidx.work.WorkManager
import coil3.SingletonImageLoader
import io.github.hipotures.bring.browser.Browse
import io.github.hipotures.bring.ffi.Entry
import io.github.hipotures.bring.ffi.EntryState
import io.github.hipotures.bring.ffi.RingInfo
import io.github.hipotures.bring.ffi.VerifyReport
import io.github.hipotures.bring.ffi.WatchedFolder
import io.github.hipotures.bring.storage.StorageAccess
import io.github.hipotures.bring.sync.RouteStatus
import io.github.hipotures.bring.sync.SyncScheduler
import io.github.hipotures.bring.sync.SyncWorker
import io.github.hipotures.bring.sync.onUnmeteredWifi
import io.github.hipotures.bring.ui.ThemeChoice
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.ensureActive
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
import kotlinx.coroutines.withContext
import java.io.File
import java.util.UUID

/** What the status bar shows about sync. */
data class SyncUi(
    val running: Boolean = false,
    val ringId: String? = null,
    val ringName: String? = null,
    val path: String? = null,
    val sent: Long = 0,
    val total: Long = 0,
    /** Files uploaded or uploading in this round. */
    val files: Int = 0,
    /** Bytes sent in this round, all files together. */
    val bytes: Long = 0,
)

/** App-wide state: multi-Ring pairing, routes, sync, storage and watched folders. */
@OptIn(FlowPreview::class)
class AppViewModel(app: Application) : AndroidViewModel(app) {
    private val bring = app.bring
    private val wm = WorkManager.getInstance(app)

    sealed interface Pairing {
        data object Loading : Pairing
        data object Unpaired : Pairing
        data class Paired(val rings: List<RingInfo>, val activeRingId: String) : Pairing {
            val active: RingInfo get() = rings.firstOrNull { it.ringId == activeRingId } ?: rings.first()
        }
    }

    private val _pairing = MutableStateFlow<Pairing>(Pairing.Loading)
    val pairing: StateFlow<Pairing> = _pairing.asStateFlow()

    /** Rings of the generated demo database (all shown as reachable over LAN). */
    private val demoDbRings = MutableStateFlow<Set<String>>(emptySet())

    val routes: StateFlow<Map<String, RouteStatus>> = combine(bring.prefs.demoMode, bring.routes, demoDbRings) { demo, real, db ->
        if (demo) DemoData.routes + db.associateWith { RouteStatus.LAN } else real
    }.stateIn(viewModelScope, SharingStarted.Eagerly, if (bring.prefs.demoMode.value) DemoData.routes else bring.routes.value)

    /** Route status of the currently selected Ring. */
    val route: StateFlow<RouteStatus> = combine(_pairing, routes) { p, map ->
        val id = (p as? Pairing.Paired)?.activeRingId ?: return@combine RouteStatus.UNKNOWN
        map[id] ?: RouteStatus.UNKNOWN
    }.stateIn(viewModelScope, SharingStarted.Eagerly, RouteStatus.UNKNOWN)

    private val _storageGranted = MutableStateFlow(StorageAccess.granted())
    val storageGranted: StateFlow<Boolean> = _storageGranted.asStateFlow()

    private val _folders = MutableStateFlow<List<WatchedFolder>>(emptyList())
    val folders: StateFlow<List<WatchedFolder>> = _folders.asStateFlow()

    private val _browse = MutableStateFlow<Browse?>(null)
    /** The folder the browser asked for, read from the index (not the whole index). */
    val browse: StateFlow<Browse?> = _browse.asStateFlow()

    private data class Want(val dir: String, val state: EntryState?, val limit: Int)

    @Volatile private var want = Want("", null, 200)
    private var browseJob: Job? = null

    /** The browser wants [dir] (a Ring path, "" for the root) filtered by [state], with [limit] files. */
    fun showDir(dir: String, state: EntryState?, limit: Int) {
        want = Want(dir, state, limit)
        reloadBrowse()
    }

    /** Re-reads what the browser last asked for, e.g. after a sync changed the index. */
    private fun reloadBrowse() {
        val id = activeRingId() ?: return
        val w = want
        browseJob?.cancel()
        browseJob = viewModelScope.launch(Dispatchers.IO) {
            val result = runCatching { queryBrowse(id, w) }.getOrNull() ?: return@launch
            ensureActive()
            if (activeRingId() == id) _browse.value = result
        }
    }

    private fun queryBrowse(id: String, w: Want): Browse {
        val moon = if (isDemo()) bring.demoMoon() else bring.moon
        if (moon == null) return DemoData.browse(id, w.dir, w.state, w.limit)
        val page = moon.browseDirFor(id, w.dir, w.state, 0u, w.limit.toUInt())
        return Browse.of(id, w.dir, w.state, w.limit, page, moon.dirCountsFor(id, w.dir), moon.queuedFor(id))
    }

    data class StatusMessage(val text: String, val isError: Boolean = false)

    private val _message = MutableStateFlow<StatusMessage?>(null)
    /** A one-line result of the last user action (errors flagged). */
    val message: StateFlow<StatusMessage?> = _message.asStateFlow()

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

    val sync: StateFlow<SyncUi> = combine(bring.prefs.demoMode, bring.activeSync, workInfos) { demo, live, infos ->
        if (demo) SyncUi() else if (live.running) live else syncUi(infos)
    }
        .distinctUntilChanged()
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), SyncUi())

    private val _syncError = MutableStateFlow<String?>(null)
    /** Error of the last round that ended while the app watched, if it failed. */
    val syncError: StateFlow<String?> = _syncError.asStateFlow()

    /** Finished rounds, newest first, and the end of the last check. */
    val rounds = combine(bring.prefs.demoMode, app.bring.syncLog.rounds) { demo, real ->
        if (demo) DemoData.rounds() else real
    }.stateIn(viewModelScope, SharingStarted.Eagerly, app.bring.syncLog.rounds.value)

    val lastChecked = combine(bring.prefs.demoMode, app.bring.syncLog.lastChecked) { demo, real ->
        if (demo) System.currentTimeMillis() - 4 * 60_000L else real
    }.stateIn(viewModelScope, SharingStarted.Eagerly, app.bring.syncLog.lastChecked.value)

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
            networkChanged.debounce(1_000).collect { probeAllRoutes() }
        }
        viewModelScope.launch { trackRounds() }
        viewModelScope.launch {
            syncFinished.collect {
                loadEntries()
                loadFolders()
                reloadRings()
            }
        }
        // Reload once a round starts (so newly scanned local files appear while
        // large files are still hashing) and as uploads move from file to file.
        viewModelScope.launch {
            sync.map { it.running }.distinctUntilChanged().collect { running ->
                if (running) {
                    delay(1_000)
                    loadEntries()
                }
            }
        }
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

    private fun activeRingId(): String? = (_pairing.value as? Pairing.Paired)?.activeRingId

    private fun isDemo(): Boolean = bring.prefs.demoMode.value

    fun setDemoMode(enabled: Boolean) {
        bring.prefs.setDemoMode(enabled)
        _verify.value = VerifyState.Idle
        _message.value = null
        refresh()
    }

    /**
     * Demo data: the generated `demo.db` when it is on the phone (read through its own
     * [Moon], like real data), otherwise the small hand-made [DemoData] sample.
     */
    private fun loadDemo(ringId: String?) {
        val moon = bring.demoMoon()
        if (moon == null) {
            demoDbRings.value = emptySet()
            val active = ringId ?: DemoData.RING_HOME_NAS
            _pairing.value = Pairing.Paired(DemoData.rings, active)
            _folders.value = DemoData.foldersFor(active)
            reloadBrowse()
            return
        }
        viewModelScope.launch(Dispatchers.IO) {
            val rings = runCatching { moon.rings() }.getOrDefault(emptyList())
            if (rings.isEmpty()) return@launch
            val active = ringId?.takeIf { id -> rings.any { it.ringId == id } } ?: rings.first().ringId
            val folders = runCatching { moon.watchedFoldersFor(active) }.getOrDefault(emptyList())
            if (!isDemo()) return@launch
            demoDbRings.value = rings.mapTo(HashSet()) { it.ringId }
            _pairing.value = Pairing.Paired(rings, active)
            _folders.value = folders
            reloadBrowse()
        }
    }

    fun refresh() {
        if (isDemo()) {
            loadDemo(null)
            return
        }
        viewModelScope.launch(Dispatchers.IO) {
            val all = runCatching { bring.moon.rings() }.getOrDefault(emptyList())
            if (all.size == 1) {
                bring.tokens.migrateLegacy(all.first().ringId)?.let {
                    bring.moon.setRingToken(all.first().ringId, it)
                }
            }
            val valid = all.filter { r ->
                val tok = bring.tokens.load(r.ringId)
                if (tok != null) bring.moon.setRingToken(r.ringId, tok)
                tok != null
            }
            if (valid.isNotEmpty()) {
                val chosen = bring.prefs.selectedRingId?.takeIf { id -> valid.any { it.ringId == id } }
                    ?: valid.first().ringId
                bring.prefs.selectedRingId = chosen
                _pairing.value = Pairing.Paired(valid, chosen)
                SyncScheduler.schedulePeriodic(getApplication())
                SyncScheduler.watchMedia(getApplication())
                loadFolders(chosen)
                loadEntries(chosen)
                probeAllRoutes()
            } else {
                _pairing.value = Pairing.Unpaired
                _folders.value = emptyList()
                _browse.value = null
            }
        }
    }

    /** Switches the active Ring across Browser and Settings. */
    fun selectRing(ringId: String) {
        val current = _pairing.value as? Pairing.Paired ?: return
        if (current.activeRingId == ringId) return
        if (current.rings.none { it.ringId == ringId }) return
        if (isDemo()) {
            _pairing.value = current.copy(activeRingId = ringId)
            _verify.value = VerifyState.Idle
            _message.value = null
            loadDemo(ringId)
            return
        }
        bring.prefs.selectedRingId = ringId
        _pairing.value = current.copy(activeRingId = ringId)
        _verify.value = VerifyState.Idle
        _message.value = null
        loadFolders(ringId)
        loadEntries(ringId)
        if ((bring.routes.value[ringId] ?: RouteStatus.UNKNOWN) == RouteStatus.UNKNOWN) {
            probeRoute(ringId)
        }
    }

    /** Called on resume: the user may have granted access in Settings. */
    fun onResume() {
        _storageGranted.value = StorageAccess.granted()
        if (isDemo()) return
        if (_pairing.value is Pairing.Paired) {
            networkChanged.tryEmit(Unit)
            loadEntries()
            if (System.currentTimeMillis() - bring.syncLog.lastChecked.value > 60_000) {
                SyncScheduler.syncSoon(getApplication())
            }
        }
    }

    fun onPaired(ring: RingInfo) {
        viewModelScope.launch(Dispatchers.IO) {
            val all = runCatching { bring.moon.rings() }.getOrDefault(listOf(ring))
                .filter { bring.tokens.load(it.ringId) != null }
            val list = all.ifEmpty { listOf(ring) }
            bring.prefs.selectedRingId = ring.ringId
            _pairing.value = Pairing.Paired(list, ring.ringId)
            _verify.value = VerifyState.Idle
            SyncScheduler.schedulePeriodic(getApplication())
            SyncScheduler.watchMedia(getApplication())
            loadFolders(ring.ringId)
            loadEntries(ring.ringId)
            // First listing of the newly paired Ring right away, over whichever route works.
            SyncScheduler.syncNow(getApplication(), ringId = ring.ringId)
            probeRoute(ring.ringId)
        }
    }

    fun renameRing(ringId: String, name: String) = action {
        val updated = bring.moon.renameRing(ringId, name)
        val cur = _pairing.value as? Pairing.Paired
        if (cur != null) {
            _pairing.value = cur.copy(
                rings = cur.rings.map { if (it.ringId == ringId) updated else it },
            )
        }
        null
    }

    fun renameDevice(ringId: String, name: String) = action {
        val updated = bring.moon.renameDeviceFor(ringId, name)
        val cur = _pairing.value as? Pairing.Paired
        if (cur != null) {
            _pairing.value = cur.copy(
                rings = cur.rings.map { if (it.ringId == ringId) updated else it },
            )
        }
        loadFolders(ringId)
        loadEntries(ringId)
        "Device renamed to ${updated.deviceSlug}"
    }

    /** Reloads the current folder view (the index may have changed). */
    @Suppress("UNUSED_PARAMETER")
    fun loadEntries(ringId: String? = null) = reloadBrowse()

    private fun reloadRings() {
        if (isDemo()) return
        viewModelScope.launch(Dispatchers.IO) {
            val cur = _pairing.value as? Pairing.Paired ?: return@launch
            val fresh = runCatching { bring.moon.rings() }.getOrDefault(emptyList())
                .filter { bring.tokens.load(it.ringId) != null }
            if (!isDemo() && fresh.isNotEmpty()) {
                val active = cur.activeRingId.takeIf { id -> fresh.any { it.ringId == id } }
                    ?: fresh.first().ringId
                _pairing.value = Pairing.Paired(fresh, active)
            }
        }
    }

    private fun probeAllRoutes() {
        if (isDemo()) return
        val rings = (_pairing.value as? Pairing.Paired)?.rings ?: return
        rings.forEach { probeRoute(it.ringId) }
    }

    private fun probeRoute(ringId: String) {
        viewModelScope.launch(Dispatchers.IO) {
            if (_pairing.value !is Pairing.Paired) return@launch
            val wifi = getApplication<Application>().onUnmeteredWifi()
            val t0 = SystemClock.elapsedRealtime()
            val result = runCatching { RouteStatus.of(bring.moon.selectRouteFor(ringId, wifi)) }
            Log.i(TAG, "route[$ringId]: unmeteredWifi=$wifi -> $result in ${SystemClock.elapsedRealtime() - t0} ms")
            bring.setRoute(ringId, result.getOrDefault(RouteStatus.OFFLINE))
        }
    }

    fun recheckRoute() {
        if (isDemo()) return
        val id = activeRingId() ?: return
        bring.setRoute(id, RouteStatus.UNKNOWN)
        probeRoute(id)
    }

    fun syncNow() {
        if (isDemo()) return
        _syncError.value = null
        SyncScheduler.syncNow(getApplication(), ringId = activeRingId())
    }

    /**
     * Full check & repair for the active Ring: asks the Ring to rescan missing
     * hashes/thumbnails, re-reads the Ring index from seq 0, rescans watched
     * folders, clears empty cached thumbnails and retries queued uploads.
     */
    fun verifySync() {
        if (isDemo()) {
            _message.value = StatusMessage("Demo mode is active")
            return
        }
        if (_verify.value is VerifyState.Running) return
        val id = activeRingId() ?: return
        _verify.value = VerifyState.Running
        viewModelScope.launch(Dispatchers.IO) {
            val app = getApplication<Application>()
            _verify.value = try {
                bring.syncMutex.withLock {
                    val route = bring.moon.selectRouteFor(id, app.onUnmeteredWifi())
                    bring.setRoute(id, RouteStatus.of(route))
                    if (route == null) {
                        VerifyState.Failed("Ring unreachable")
                    } else {
                        File(app.cacheDir, "thumbs").listFiles()
                            ?.filter { it.isFile && it.length() == 0L }
                            ?.forEach { it.delete() }
                        val r = bring.moon.verifySyncFor(id)
                        SingletonImageLoader.get(app).memoryCache?.clear()
                        reloadBrowse()
                        if (r.queued > 0u) SyncScheduler.syncNow(app, ringId = id)
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
     * request going from RUNNING back to ENQUEUED.
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
     * become remote-only. Skips files another Ring still needs to upload.
     */
    fun freeLocal(files: List<Entry>) {
        if (isDemo()) return
        viewModelScope.launch(Dispatchers.IO) {
            val text = try {
                val r = bring.moon.freeLocal(files.map { it.id })
                val paths = files.mapNotNull { it.localPath }.filter { !File(it).exists() }
                if (paths.isNotEmpty()) MediaScannerConnection.scanFile(getApplication(), paths.toTypedArray(), null, null)
                buildString {
                    val n = r.freed.toInt()
                    append("Removed $n ${if (n == 1) "file" else "files"} from the phone (${formatBytes(r.freedBytes)})")
                    if (r.skipped > 0u) append(" · ${r.skipped} skipped: changed, pending or needed by another Ring")
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

    private fun loadFolders(ringId: String? = null) {
        if (isDemo()) return
        viewModelScope.launch(Dispatchers.IO) {
            val id = ringId ?: activeRingId() ?: return@launch
            val list = runCatching { bring.moon.watchedFoldersFor(id) }.getOrDefault(emptyList())
            if (!isDemo() && activeRingId() == id) _folders.value = list
        }
    }

    fun addFolder(localDir: String) = action {
        val id = activeRingId() ?: return@action null
        bring.moon.addWatchedFolderFor(id, localDir, null)
        val list = runCatching { bring.moon.watchedFoldersFor(id) }.getOrDefault(emptyList())
        if (activeRingId() == id) _folders.value = list
        // Scan right away (fast stat-only walk, no hashing) so the new folder
        // and its files appear on Home immediately before hashing/upload begins.
        if (!bring.syncMutex.isLocked) {
            runCatching { bring.moon.scanLocalFor(id) }
            reloadBrowse()
        }
        SyncScheduler.syncNow(getApplication(), queueAfter = true, ringId = id)
        "Watching $localDir"
    }

    fun removeFolder(folderId: Long) = action {
        val id = activeRingId() ?: return@action null
        bring.moon.removeWatchedFolder(folderId)
        val list = runCatching { bring.moon.watchedFoldersFor(id) }.getOrDefault(emptyList())
        if (activeRingId() == id) _folders.value = list
        loadEntries(id)
        null
    }

    fun setOffloadThreshold(bytes: ULong) = action {
        val id = activeRingId() ?: return@action null
        bring.moon.setRingOffloadThreshold(id, bytes)
        val cur = _pairing.value as? Pairing.Paired
        if (cur != null) {
            _pairing.value = cur.copy(
                rings = cur.rings.map { if (it.ringId == id) it.copy(offloadThresholdBytes = bytes) else it },
            )
        }
        null
    }

    fun setTheme(choice: ThemeChoice) = bring.prefs.setTheme(choice)

    /** Forgets the given Ring and its index; other Rings and local files are untouched. */
    fun forget(ringId: String) = action {
        bring.moon.forgetRingById(ringId)
        bring.tokens.remove(ringId)
        bring.removeRoute(ringId)
        val remaining = runCatching { bring.moon.rings() }.getOrDefault(emptyList())
            .filter { bring.tokens.load(it.ringId) != null }
        if (remaining.isEmpty()) {
            SyncScheduler.cancelAll(getApplication())
            bring.prefs.selectedRingId = null
            _folders.value = emptyList()
            _browse.value = null
            _pairing.value = Pairing.Unpaired
        } else {
            val next = remaining.first().ringId
            bring.prefs.selectedRingId = next
            _pairing.value = Pairing.Paired(remaining, next)
            _verify.value = VerifyState.Idle
            loadFolders(next)
            loadEntries(next)
        }
        null
    }

    fun clearMessage() {
        _message.value = null
    }

    /** Runs [block] on IO; its result (or error) becomes [message]. */
    private fun action(block: suspend () -> String?) {
        if (isDemo()) {
            _message.value = StatusMessage("Demo mode is active")
            return
        }
        viewModelScope.launch(Dispatchers.IO) {
            _message.value = try {
                block()?.let { StatusMessage(it, isError = false) }
            } catch (e: Exception) {
                StatusMessage(e.describe(), isError = true)
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
            ringId = p?.getString(SyncWorker.KEY_RING_ID),
            ringName = p?.getString(SyncWorker.KEY_RING_NAME),
            path = p?.getString(SyncWorker.KEY_PATH),
            sent = p?.getLong(SyncWorker.KEY_SENT, 0) ?: 0,
            total = p?.getLong(SyncWorker.KEY_TOTAL, 0) ?: 0,
            files = p?.getInt(SyncWorker.KEY_FILES, 0) ?: 0,
            bytes = p?.getLong(SyncWorker.KEY_BYTES, 0) ?: 0,
        )
    }
}

private const val TAG = "bring"
