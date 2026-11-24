package io.github.hipotures.bring.sync

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import androidx.core.app.NotificationCompat
import androidx.work.ForegroundInfo
import io.github.hipotures.bring.MainActivity
import io.github.hipotures.bring.R
import io.github.hipotures.bring.formatBytes

object Notifications {
    const val CHANNEL_SYNC = "sync"
    const val ID_SYNC = 1

    fun createChannels(context: Context) {
        val nm = context.getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL_SYNC, "Sync", NotificationManager.IMPORTANCE_LOW).apply {
                description = "Uploads to the Ring"
                setShowBadge(false)
            },
        )
    }

    fun sync(context: Context, path: String?, sent: Long, total: Long): Notification {
        val open = PendingIntent.getActivity(
            context,
            0,
            Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE,
        )
        val b = NotificationCompat.Builder(context, CHANNEL_SYNC)
            .setSmallIcon(R.drawable.ic_stat_sync)
            .setContentTitle(if (path == null) "Syncing with the Ring" else "Uploading")
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setSilent(true)
            .setContentIntent(open)
        if (path != null && total > 0) {
            val pct = (sent * 100 / total).toInt()
            b.setContentText("${path.substringAfterLast('/')} · ${formatBytes(sent)} / ${formatBytes(total)}")
                .setProgress(100, pct, false)
        } else {
            b.setProgress(0, 0, true)
        }
        return b.build()
    }

    fun syncForeground(context: Context, path: String?, sent: Long, total: Long) = ForegroundInfo(
        ID_SYNC,
        sync(context, path, sent, total),
        ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC,
    )
}
