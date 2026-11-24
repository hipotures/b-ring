package io.github.hipotures.bring.sync

import android.content.Context
import androidx.work.BackoffPolicy
import androidx.work.Constraints
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.workDataOf
import java.util.concurrent.TimeUnit

object SyncScheduler {
    const val TAG = "bring-sync"
    const val PERIODIC = "bring-sync-periodic"
    const val MANUAL = "bring-sync-now"

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
