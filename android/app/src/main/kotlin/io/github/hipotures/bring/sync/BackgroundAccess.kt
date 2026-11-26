package io.github.hipotures.bring.sync

import android.annotation.SuppressLint
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.os.PowerManager
import android.provider.Settings
import androidx.core.net.toUri

/**
 * Exemption from battery optimisation ("Unrestricted" on Samsung). Without it,
 * Samsung's Freecess freezes the app in the background and cuts its network
 * (`OEM_DENY`), so media-triggered and periodic rounds wait until the app is
 * opened. See docs/ANDROID_DEV.md §5.
 */
object BackgroundAccess {
    fun granted(context: Context): Boolean =
        context.getSystemService(PowerManager::class.java)
            ?.isIgnoringBatteryOptimizations(context.packageName) ?: true

    /** The one-tap system dialog; falls back to the list of apps where it is missing. */
    @SuppressLint("BatteryLife") // a file-sync app: background rounds are its job
    fun request(context: Context) {
        try {
            context.startActivity(
                Intent(
                    Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS,
                    "package:${context.packageName}".toUri(),
                ),
            )
        } catch (_: ActivityNotFoundException) {
            context.startActivity(Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS))
        }
    }
}
