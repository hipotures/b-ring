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
import io.github.hipotures.bring.bring
import io.github.hipotures.bring.describe
import io.github.hipotures.bring.ffi.MoonException
import io.github.hipotures.bring.ffi.SyncReport
import io.github.hipotures.bring.ffi.TransferListener
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.withContext

/**
 * One sync round: route selection (§4), then `run_sync` (index → scan →
 * outbox → index) as a foreground `dataSync` job so long uploads survive.
 */
class SyncWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {

    override suspend fun getForegroundInfo(): ForegroundInfo =
        Notifications.syncForeground(applicationContext, null)

    override suspend fun doWork(): Result = withContext(Dispatchers.IO) {
        val bring = applicationContext.bring
        val trigger = if (inputData.getBoolean(KEY_MANUAL, false)) Trigger.MANUAL else Trigger.AUTO
        if (bring.tokens.load() == null) return@withContext Result.success()
        val moon = bring.moon

        val route = try {
            moon.selectRoute(applicationContext.onUnmeteredWifi())
        } catch (_: MoonException.NotPaired) {
            return@withContext Result.success()
        }
        bring.route.value = RouteStatus.of(route)
        if (!RoutePolicy.mayRun(trigger, route)) {
            val why = if (route == null) "Ring unreachable" else "not on the Ring's LAN"
            return@withContext if (trigger == Trigger.MANUAL) failure(why) else Result.success()
        }

        try {
            setForeground(getForegroundInfo())
        } catch (e: Exception) {
            // Android 12+ may refuse a foreground start from the background;
            // the round still runs, as an ordinary (time-limited) worker.
            Log.w(TAG, "running without a foreground service: $e")
        }

        try {
            val report = coroutineScope {
                val run = async { moon.runSync(Progress()) }
                try {
                    run.await()
                } catch (e: CancellationException) {
                    // The blocking call ends at the next file or chunk boundary.
                    moon.cancel()
                    throw e
                }
            }
            Result.success(summary(report))
        } catch (e: MoonException.Cancelled) {
            Result.retry()
        } catch (e: MoonException.Network) {
            if (trigger == Trigger.MANUAL) failure(e.describe()) else Result.retry()
        } catch (e: MoonException) {
            failure(e.describe())
        }
    }

    private fun failure(message: String) = Result.failure(workDataOf(KEY_ERROR to message))

    private fun summary(r: SyncReport): Data = workDataOf(
        KEY_RECEIVED to r.index.received.toInt(),
        KEY_UPLOADED to (r.outbox.uploaded + r.outbox.deduplicated).toInt(),
        KEY_OFFLOADED to r.outbox.offloaded.toInt(),
        KEY_FAILED to r.outbox.failed.toInt(),
        KEY_REMAINING to r.outbox.remaining.toInt(),
    )

    /**
     * Upload progress → WorkManager progress + the foreground notification,
     * at most once a second: with many small files the engine reports
     * hundreds of files per second, far too fast to read.
     */
    private inner class Progress : TransferListener {
        private var lastAt = 0L
        private var current: String? = null
        private var currentSent = 0L
        private var doneBytes = 0L
        private var files = 0
        private val nm = applicationContext.getSystemService(NotificationManager::class.java)

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
            val now = SystemClock.elapsedRealtime()
            if (now - lastAt < 1_000) return
            lastAt = now
            val p = Notifications.Upload(ringPath, sent, total, files, doneBytes + sent)
            setProgressAsync(
                workDataOf(
                    KEY_PATH to p.path,
                    KEY_SENT to p.sent,
                    KEY_TOTAL to p.total,
                    KEY_FILES to p.files,
                    KEY_BYTES to p.bytes,
                ),
            )
            runCatching { nm.notify(Notifications.ID_SYNC, Notifications.sync(applicationContext, p)) }
        }
    }

    companion object {
        private const val TAG = "SyncWorker"
        const val KEY_MANUAL = "manual"
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
