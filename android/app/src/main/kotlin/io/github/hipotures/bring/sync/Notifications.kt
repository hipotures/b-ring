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

/**
 * The status-bar upload mark: the notification of the foreground service a
 * sync round runs in. While a file uploads its icon blinks (outline square ↔
 * filled square, the status bar is monochrome); when the round ends the
 * service stops and the mark disappears. No "done" notification.
 */
object Notifications {
    private const val CHANNEL = "upload"
    const val ID = 1

    fun createChannel(context: Context) {
        context.getSystemService(NotificationManager::class.java)?.run {
            deleteNotificationChannel("sync") // older builds' channel
            createNotificationChannel(
                // LOW: icon in the status bar, no sound, no pop-up.
                NotificationChannel(CHANNEL, "Upload in progress", NotificationManager.IMPORTANCE_LOW).apply {
                    setShowBadge(false)
                },
            )
        }
    }

    /** One progress report: the current file and the round's totals. */
    data class Upload(
        val path: String,
        val sent: Long,
        val total: Long,
        val files: Int,
        val bytes: Long,
        val ringName: String? = null,
    )

    fun build(context: Context, upload: Upload?, lit: Boolean): Notification {
        val open = PendingIntent.getActivity(
            context,
            0,
            Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE,
        )
        val b = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(if (lit) R.drawable.ic_stat_sync_lit else R.drawable.ic_stat_sync)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setSilent(true)
            .setShowWhen(false)
            .setContentIntent(open)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
        if (upload == null) {
            b.setContentTitle("Syncing with the Ring")
        } else {
            val prefix = upload.ringName?.takeIf { it.isNotBlank() }?.let { "$it · " } ?: ""
            b.setContentTitle("Uploading · $prefix${upload.files} ${if (upload.files == 1) "file" else "files"} · ${formatBytes(upload.bytes)}")
                .setContentText(upload.path.substringAfterLast('/'))
            if (upload.total > 0) b.setProgress(100, (upload.sent * 100 / upload.total).toInt(), false)
        }
        return b.build()
    }

    fun foreground(context: Context) =
        ForegroundInfo(ID, build(context, null, false), ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
}
