package io.github.hipotures.bring.sync

import android.app.NotificationManager
import android.content.Context
import android.os.SystemClock
import android.util.Log
import androidx.work.CoroutineWorker
import androidx.work.Data
import androidx.work.ForegroundInfo
import androidx.work.WorkerParameters
import androidx.work.workDataOf
import io.github.hipotures.bring.SyncUi
import io.github.hipotures.bring.bring
import io.github.hipotures.bring.describe
import io.github.hipotures.bring.ffi.MoonException
import io.github.hipotures.bring.ffi.RingInfo
import io.github.hipotures.bring.ffi.SyncReport
import io.github.hipotures.bring.ffi.TransferListener
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Sync round across all paired Rings (or one specific Ring if [KEY_RING_ID] is
 * given): route selection (§4), then `run_sync_for` (index → scan → outbox →
 * index) as a foreground `dataSync` job, so switching apps or taking a
 * screenshot does not stop an upload.
 */
class SyncWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {

    override suspend fun getForegroundInfo(): ForegroundInfo = Notifications.foreground(applicationContext)

    override suspend fun doWork(): Result = withContext(Dispatchers.IO) {
        val bring = applicationContext.bring
        val trigger = if (inputData.getBoolean(KEY_MANUAL, false)) Trigger.MANUAL else Trigger.AUTO
        val isMedia = inputData.getBoolean(KEY_MEDIA, false)
        val targetRingId = inputData.getString(KEY_RING_ID)
        if (!bring.tokens.hasAny()) return@withContext Result.success()

        // Only one round runs at a time. Redundant periodic/resume auto checks
        // step aside immediately; manual and media-triggered rounds wait cleanly
        // without touching the active worker's notification, progress, or cancel flag.
        if (trigger == Trigger.AUTO && !isMedia) {
            if (!bring.syncMutex.tryLock()) return@withContext Result.success()
        } else {
            bring.syncMutex.lock()
        }

        try {
            // Keep watching for new files: re-arm the media trigger once we hold the lock.
            SyncScheduler.watchMedia(applicationContext, rearm = isMedia)
            val moon = bring.moon
            val paired = runCatching { moon.rings() }.getOrDefault(emptyList())
                .filter { bring.tokens.load(it.ringId) != null }
            if (paired.isEmpty()) return@withContext Result.success()

            // Background rounds carry no network constraint (see SyncScheduler.schedulePeriodic),
            // so check unmetered Wi-Fi here. The network may come back a moment after Samsung
            // unfreezes the app for this job, hence the short wait.
            if (trigger == Trigger.AUTO && !awaitUnmeteredWifi()) {
                SyncScheduler.syncSoon(applicationContext)
                return@withContext Result.success()
            }

            val targets = targetRingId?.let { id ->
                paired.filter { it.ringId == id }.ifEmpty { paired }
            } ?: paired

            try {
                setForeground(getForegroundInfo())
            } catch (e: Exception) {
                // Android 12+ may refuse a foreground start from the background;
                // the round still runs, as an ordinary (stoppable) job.
                Log.w(TAG, "running without a foreground service: $e")
            }

            val unmetered = applicationContext.onUnmeteredWifi()
            var totalReceived = 0
            var totalUploaded = 0
            var totalOffloaded = 0
            var totalFailed = 0
            var totalRemaining = 0
            var retryNeeded = false
            var lastError: String? = null
            var anyRan = false

            for (ring in targets) {
                val route = try {
                    moon.selectRouteFor(ring.ringId, unmetered)
                } catch (_: MoonException.NotPaired) {
                    continue
                }
                bring.setRoute(ring.ringId, RouteStatus.of(route))
                if (!RoutePolicy.mayRun(trigger, route)) {
                    val why = if (route == null) "Ring unreachable" else "Not on the Ring's LAN"
                    if (trigger == Trigger.MANUAL && targets.size == 1) {
                        record(
                            SyncRound(
                                endedAt = System.currentTimeMillis(),
                                durationMs = 0,
                                manual = true,
                                route = route?.name ?: "",
                                uploaded = 0,
                                uploadedBytes = 0,
                                received = 0,
                                found = 0,
                                offloaded = 0,
                                failed = 0,
                                remaining = 0,
                                error = why,
                                files = emptyList(),
                                ringName = ring.name,
                            ),
                        )
                        lastError = why
                    }
                    continue
                }

                anyRan = true
                bring.activeSync.value = SyncUi(running = true, ringId = ring.ringId, ringName = ring.name)
                val started = SystemClock.elapsedRealtime()
                val progress = Progress(ring, showRingName = paired.size > 1)
                fun round(report: SyncReport?, error: String?) = SyncRound(
                    endedAt = System.currentTimeMillis(),
                    durationMs = SystemClock.elapsedRealtime() - started,
                    manual = trigger == Trigger.MANUAL,
                    route = route?.name ?: "",
                    uploaded = report?.let { (it.outbox.uploaded + it.outbox.deduplicated).toInt() } ?: progress.completedCount,
                    uploadedBytes = progress.completedBytes,
                    received = report?.index?.received?.toInt() ?: 0,
                    found = report?.let { (it.scan.newFiles + it.scan.changed).toInt() } ?: 0,
                    offloaded = report?.outbox?.offloaded?.toInt() ?: 0,
                    failed = report?.outbox?.failed?.toInt() ?: 0,
                    remaining = report?.outbox?.remaining?.toInt() ?: 0,
                    error = error,
                    files = progress.completed.toList(),
                    ringName = ring.name,
                )

                try {
                    val report = coroutineScope {
                        val run = async { moon.runSyncFor(ring.ringId, progress) }
                        val blink = launch { blink(progress) }
                        try {
                            run.await()
                        } catch (e: CancellationException) {
                            moon.cancel()
                            throw e
                        } finally {
                            blink.cancelAndJoin()
                        }
                    }
                    record(round(report, null))
                    totalReceived += report.index.received.toInt()
                    totalUploaded += (report.outbox.uploaded + report.outbox.deduplicated).toInt()
                    totalOffloaded += report.outbox.offloaded.toInt()
                    totalFailed += report.outbox.failed.toInt()
                    totalRemaining += report.outbox.remaining.toInt()
                } catch (_: MoonException.Cancelled) {
                    return@withContext Result.retry()
                } catch (e: MoonException.Network) {
                    val msg = e.describe()
                    record(round(null, msg))
                    lastError = msg
                    if (trigger != Trigger.MANUAL) retryNeeded = true
                } catch (e: MoonException) {
                    val msg = e.describe()
                    record(round(null, msg))
                    lastError = msg
                }
            }

            when {
                retryNeeded -> Result.retry()
                !anyRan && lastError != null -> failure(lastError)
                lastError != null && targets.size == 1 -> failure(lastError)
                else -> Result.success(
                    workDataOf(
                        KEY_RECEIVED to totalReceived,
                        KEY_UPLOADED to totalUploaded,
                        KEY_OFFLOADED to totalOffloaded,
                        KEY_FAILED to totalFailed,
                        KEY_REMAINING to totalRemaining,
                    ),
                )
            }
        } finally {
            bring.activeSync.value = SyncUi()
            bring.syncMutex.unlock()
        }
    }

    /** Unmetered Wi-Fi now, or within a few seconds (network unblocked after unfreeze). */
    private suspend fun awaitUnmeteredWifi(): Boolean {
        repeat(WIFI_WAIT_TRIES) {
            if (applicationContext.onUnmeteredWifi()) return true
            delay(1_000)
        }
        return applicationContext.onUnmeteredWifi()
    }

    /**
     * Blink the status-bar mark while a file uploads: re-post the service's
     * notification with the other icon frame (well under Android's limit of
     * a few updates a second).
     */
    private suspend fun blink(progress: Progress) {
        val nm = applicationContext.getSystemService(NotificationManager::class.java) ?: return
        var lit = false
        while (currentCoroutineContext().isActive) {
            delay(BLINK_MS)
            val upload = progress.snapshot() ?: continue
            lit = !lit
            runCatching { nm.notify(Notifications.ID, Notifications.build(applicationContext, upload, lit)) }
        }
    }

    /** Log the round for the history screen. */
    private fun record(round: SyncRound) {
        applicationContext.bring.syncLog.add(round)
    }

    private fun failure(message: String) = Result.failure(workDataOf(KEY_ERROR to message))

    /**
     * Upload progress → live [BringApp.activeSync] + WorkManager progress (at
     * most once a second for SQLite persistence).
     */
    private inner class Progress(
        private val ring: RingInfo,
        private val showRingName: Boolean,
    ) : TransferListener {
        private var lastAt = 0L
        private var current: String? = null
        private var currentSent = 0L
        private var doneBytes = 0L
        private var files = 0
        private var lastCompleted: String? = null

        @Volatile private var last: Notifications.Upload? = null

        /** The file being sent and the round's totals; null before the first one or once it is done. */
        fun snapshot(): Notifications.Upload? = last?.takeIf { it.sent < it.total }

        /** The first [SyncRound.MAX_FILES] Ring paths fully sent in this round. */
        val completed = ArrayList<String>(SyncRound.MAX_FILES)
        @Volatile var completedCount = 0
            private set
        @Volatile var completedBytes = 0L
            private set

        @Synchronized
        override fun onUploadProgress(ringPath: String, sentBytes: ULong, totalBytes: ULong) {
            val sent = sentBytes.toLong()
            val total = totalBytes.toLong()
            if (ringPath != current) {
                doneBytes += currentSent
                current = ringPath
                files++
            }
            currentSent = sent
            val totalSent = doneBytes + sent
            last = Notifications.Upload(
                path = ringPath,
                sent = sent,
                total = total,
                files = files,
                bytes = totalSent,
                ringName = if (showRingName) ring.name else null,
            )
            applicationContext.bring.activeSync.value = SyncUi(
                running = true,
                ringId = ring.ringId,
                ringName = ring.name,
                path = ringPath,
                sent = sent,
                total = total,
                files = files,
                bytes = totalSent,
            )
            if (sent >= total && lastCompleted != ringPath) {
                lastCompleted = ringPath
                completedCount++
                completedBytes += total
                if (completed.size < SyncRound.MAX_FILES) completed.add(ringPath)
            }
            val now = SystemClock.elapsedRealtime()
            if (now - lastAt < 1_000) return
            lastAt = now
            setProgressAsync(
                workDataOf(
                    KEY_RING_ID to ring.ringId,
                    KEY_RING_NAME to ring.name,
                    KEY_PATH to ringPath,
                    KEY_SENT to sent,
                    KEY_TOTAL to total,
                    KEY_FILES to files,
                    KEY_BYTES to totalSent,
                ),
            )
        }
    }

    companion object {
        private const val TAG = "SyncWorker"
        private const val BLINK_MS = 600L
        private const val WIFI_WAIT_TRIES = 3
        const val KEY_MANUAL = "manual"
        const val KEY_MEDIA = "media"
        const val KEY_RING_ID = "ring_id"
        const val KEY_RING_NAME = "ring_name"
        const val KEY_PATH = "path"
        const val KEY_SENT = "sent"
        const val KEY_TOTAL = "total"
        /** Files whose upload started in this round (the current one included). */
        const val KEY_FILES = "files"
        /** Bytes sent in this round, all files together. */
        const val KEY_BYTES = "bytes"
        const val KEY_ERROR = "error"
        const val KEY_RECEIVED = "received"
        const val KEY_UPLOADED = "uploaded"
        const val KEY_OFFLOADED = "offloaded"
        const val KEY_FAILED = "failed"
        const val KEY_REMAINING = "remaining"
    }
}
