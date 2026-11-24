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

    /** One progress report: the current file and the round's totals. */
    data class Upload(val path: String, val sent: Long, val total: Long, val files: Int, val bytes: Long)

    fun sync(context: Context, upload: Upload?): Notification {
        val open = PendingIntent.getActivity(
            context,
            0,
            Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE,
        )
        val b = NotificationCompat.Builder(context, CHANNEL_SYNC)
            .setSmallIcon(R.drawable.ic_stat_sync)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setSilent(true)
            .setContentIntent(open)
        if (upload == null) {
            b.setContentTitle("Syncing with the Ring").setProgress(0, 0, true)
        } else {
            b.setContentTitle("Uploading · ${upload.files} files · ${formatBytes(upload.bytes)}")
                .setContentText(upload.path.substringAfterLast('/'))
            if (upload.total > 0) b.setProgress(100, (upload.sent * 100 / upload.total).toInt(), false)
        }
        return b.build()
    }

    fun syncForeground(context: Context, upload: Upload?) = ForegroundInfo(
        ID_SYNC,
        sync(context, upload),
        ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC,
    )
}
