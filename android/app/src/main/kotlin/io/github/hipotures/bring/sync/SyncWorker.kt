package io.github.hipotures.bring.sync

import android.content.Context
import android.os.SystemClock
import android.util.Log
import androidx.work.CoroutineWorker
import androidx.work.Data
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
 * outbox → index). No notifications: the app's top bar shows an upload in
 * progress, the history lists finished rounds. Without a foreground service
 * a background round may be stopped after about 10 minutes; uploads resume
 * chunk by chunk in the next round.
 */
class SyncWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {

    override suspend fun doWork(): Result = withContext(Dispatchers.IO) {
        val bring = applicationContext.bring
        val trigger = if (inputData.getBoolean(KEY_MANUAL, false)) Trigger.MANUAL else Trigger.AUTO
        if (bring.tokens.load() == null) return@withContext Result.success()
        // Keep watching for new files: re-arm the media trigger.
        SyncScheduler.watchMedia(applicationContext, rearm = inputData.getBoolean(KEY_MEDIA, false))
        val moon = bring.moon

        val route = try {
            moon.selectRoute(applicationContext.onUnmeteredWifi())
        } catch (_: MoonException.NotPaired) {
            return@withContext Result.success()
        }
        bring.route.value = RouteStatus.of(route)
        if (!RoutePolicy.mayRun(trigger, route)) {
            val why = if (route == null) "Ring unreachable" else "Not on the Ring's LAN"
            if (trigger != Trigger.MANUAL) return@withContext Result.success()
            record(SyncRound(System.currentTimeMillis(), 0, true, route?.name ?: "", 0, 0, 0, 0, 0, 0, 0, why, emptyList()))
            return@withContext failure(why)
        }

        val started = SystemClock.elapsedRealtime()
        val progress = Progress()
        fun round(report: SyncReport?, error: String?) = SyncRound(
            endedAt = System.currentTimeMillis(),
            durationMs = SystemClock.elapsedRealtime() - started,
            manual = trigger == Trigger.MANUAL,
            route = route?.name ?: "",
            uploaded = report?.let { (it.outbox.uploaded + it.outbox.deduplicated).toInt() } ?: progress.completed.size,
            uploadedBytes = progress.completedBytes,
            // Files new to this device; the engine leaves out our own
            // uploads echoed back by the index.
            received = report?.index?.received?.toInt() ?: 0,
            found = report?.let { (it.scan.newFiles + it.scan.changed).toInt() } ?: 0,
            offloaded = report?.outbox?.offloaded?.toInt() ?: 0,
            failed = report?.outbox?.failed?.toInt() ?: 0,
            remaining = report?.outbox?.remaining?.toInt() ?: 0,
            error = error,
            files = progress.completed.toList(),
        )

        try {
            val report = coroutineScope {
                val run = async { moon.runSync(progress) }
                try {
                    run.await()
                } catch (e: CancellationException) {
                    // The blocking call ends at the next file or chunk boundary.
                    moon.cancel()
                    throw e
                }
            }
            record(round(report, null))
            Result.success(summary(report))
        } catch (e: MoonException.Cancelled) {
            Result.retry()
        } catch (e: MoonException.Network) {
            record(round(null, e.describe()))
            if (trigger == Trigger.MANUAL) failure(e.describe()) else Result.retry()
        } catch (e: MoonException) {
            record(round(null, e.describe()))
            failure(e.describe())
        }
    }

    /** Log the round for the history screen. */
    private fun record(round: SyncRound) {
        applicationContext.bring.syncLog.add(round)
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
     * Upload progress → WorkManager progress (the app's top bar), at most
     * once a second: with many small files the engine reports
     * hundreds of files per second, far too fast to read.
     */
    private inner class Progress : TransferListener {
        private var lastAt = 0L
        private var current: String? = null
        private var currentSent = 0L
        private var doneBytes = 0L
        private var files = 0

        /** Ring paths fully sent in this round (the last [SyncRound.MAX_FILES]). */
        val completed = ArrayDeque<String>()
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
            if (sent >= total && completed.lastOrNull() != ringPath) {
                completed.addLast(ringPath)
                completedBytes += total
                if (completed.size > SyncRound.MAX_FILES) completed.removeFirst()
            }
            val now = SystemClock.elapsedRealtime()
            if (now - lastAt < 1_000) return
            lastAt = now
            setProgressAsync(
                workDataOf(
                    KEY_PATH to ringPath,
                    KEY_SENT to sent,
                    KEY_TOTAL to total,
                    KEY_FILES to files,
                    KEY_BYTES to doneBytes + sent,
                ),
            )
        }
    }

    companion object {
        private const val TAG = "SyncWorker"
        const val KEY_MANUAL = "manual"
        const val KEY_MEDIA = "media"
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
