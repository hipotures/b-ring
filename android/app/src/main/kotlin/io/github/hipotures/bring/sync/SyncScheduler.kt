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
    const val MEDIA = "bring-sync-media"
    const val AUTO = "bring-sync-auto"

    /** Background rounds every 15 min, only on unmetered networks (§4). */
    fun schedulePeriodic(context: Context) {
        val request = PeriodicWorkRequestBuilder<SyncWorker>(15, TimeUnit.MINUTES)
            .setConstraints(
                Constraints.Builder().setRequiredNetworkType(NetworkType.UNMETERED).build(),
            )
            .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 1, TimeUnit.MINUTES)
            .addTag(TAG)
            .build()
        WorkManager.getInstance(context)
            .enqueueUniquePeriodicWork(PERIODIC, ExistingPeriodicWorkPolicy.KEEP, request)
    }

    /**
     * New files right away: a round armed on MediaStore changes (photos,
     * videos, screenshots, downloads; app closed or not), started 5 s after
     * the last change and at most 30 s after the first. Same rules as the
     * periodic round: unmetered network, LAN only. One-shot by nature
     * (JobScheduler content triggers), so each run arms the next one.
     *
     * [rearm] is for the triggered round itself: the next trigger is chained
     * after it (KEEP would see the running round and do nothing).
     */
    fun watchMedia(context: Context, rearm: Boolean = false) {
        val constraints = Constraints.Builder()
            .setRequiredNetworkType(NetworkType.UNMETERED)
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
        WorkManager.getInstance(context).enqueueUniqueWork(MEDIA, policy, request)
    }

    /** An automatic round now (app opened): same rules as the periodic one. */
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
     */
    fun syncNow(context: Context, queueAfter: Boolean = false) {
        val request = OneTimeWorkRequestBuilder<SyncWorker>()
            .setInputData(workDataOf(SyncWorker.KEY_MANUAL to true))
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
