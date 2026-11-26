package io.github.hipotures.bring.sync

import android.content.Context
import android.provider.MediaStore
import androidx.work.BackoffPolicy
import androidx.work.Constraints
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.workDataOf
import java.time.Duration
import java.util.concurrent.TimeUnit

object SyncScheduler {
    const val TAG = "bring-sync"
    const val PERIODIC = "bring-sync-periodic"
    const val MANUAL = "bring-sync-now"
    const val MEDIA = "bring-sync-media-v2"
    private const val MEDIA_LEGACY = "bring-sync-media"
    const val AUTO = "bring-sync-auto"

    /**
     * Background rounds every 15 min. No network constraint on purpose: on
     * Samsung a frozen app has its network cut (`OEM_DENY`), so a job waiting
     * for the network never starts, and only a started job unfreezes the app.
     * [SyncWorker] checks for unmetered Wi-Fi itself (§4).
     * UPDATE carries the new constraints over to an already scheduled round.
     */
    fun schedulePeriodic(context: Context) {
        val request = PeriodicWorkRequestBuilder<SyncWorker>(15, TimeUnit.MINUTES)
            .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 1, TimeUnit.MINUTES)
            .addTag(TAG)
            .build()
        WorkManager.getInstance(context)
            .enqueueUniquePeriodicWork(PERIODIC, ExistingPeriodicWorkPolicy.UPDATE, request)
    }

    /**
     * New files right away: a round armed on MediaStore changes (photos,
     * videos, screenshots, downloads; app closed or not), started 5 s after
     * the last change and at most 30 s after the first. Same rules as the
     * periodic round: no network constraint (see [schedulePeriodic]), the
     * worker itself checks unmetered Wi-Fi and the LAN. One-shot by nature
     * (JobScheduler content triggers), so each run arms the next one.
     *
     * [rearm] is for the triggered round itself: the next trigger is chained
     * after it (KEEP would see the running round and do nothing).
     */
    fun watchMedia(context: Context, rearm: Boolean = false) {
        val constraints = Constraints.Builder()
            .addContentUriTrigger(MediaStore.Images.Media.getContentUri(MediaStore.VOLUME_EXTERNAL), true)
            .addContentUriTrigger(MediaStore.Video.Media.getContentUri(MediaStore.VOLUME_EXTERNAL), true)
            .addContentUriTrigger(MediaStore.Files.getContentUri(MediaStore.VOLUME_EXTERNAL), true)
            .setTriggerContentUpdateDelay(Duration.ofSeconds(5))
            .setTriggerContentMaxDelay(Duration.ofSeconds(30))
            .build()
        val request = OneTimeWorkRequestBuilder<SyncWorker>()
            .setInputData(workDataOf(SyncWorker.KEY_MEDIA to true))
            .setConstraints(constraints)
            .addTag(TAG)
            .build()
        val policy = if (rearm) ExistingWorkPolicy.APPEND_OR_REPLACE else ExistingWorkPolicy.KEEP
        val wm = WorkManager.getInstance(context)
        // The trigger armed by older builds still waits for the network (see above).
        // Not from a round itself: that round may be the legacy one.
        if (!rearm) wm.cancelUniqueWork(MEDIA_LEGACY)
        wm.enqueueUniqueWork(MEDIA, policy, request)
    }

    /**
     * An automatic round as soon as unmetered Wi-Fi is there: on app open, and
     * as a catch-up when a background round found no Wi-Fi. Keeps the network
     * constraint: it is a best effort on top of the periodic round.
     */
    fun syncSoon(context: Context) {
        val request = OneTimeWorkRequestBuilder<SyncWorker>()
            .setConstraints(
                Constraints.Builder().setRequiredNetworkType(NetworkType.UNMETERED).build(),
            )
            .addTag(TAG)
            .build()
        WorkManager.getInstance(context).enqueueUniqueWork(AUTO, ExistingWorkPolicy.KEEP, request)
    }

    /**
     * A user-started round, allowed over any network (WAN included).
     * [queueAfter] chains it after a running round instead of dropping it,
     * so freshly shared files go out in the next pass.
     * [ringId] limits the pass to one Ring when set; null syncs all Rings.
     */
    fun syncNow(context: Context, queueAfter: Boolean = false, ringId: String? = null) {
        val data = if (ringId != null) {
            workDataOf(SyncWorker.KEY_MANUAL to true, SyncWorker.KEY_RING_ID to ringId)
        } else {
            workDataOf(SyncWorker.KEY_MANUAL to true)
        }
        val request = OneTimeWorkRequestBuilder<SyncWorker>()
            .setInputData(data)
            .setConstraints(
                Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build(),
            )
            .addTag(TAG)
            .build()
        val policy = if (queueAfter) ExistingWorkPolicy.APPEND_OR_REPLACE else ExistingWorkPolicy.KEEP
        WorkManager.getInstance(context).enqueueUniqueWork(MANUAL, policy, request)
    }

    fun cancelAll(context: Context) {
        WorkManager.getInstance(context).cancelAllWorkByTag(TAG)
    }
}
