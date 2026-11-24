package io.github.hipotures.bring

import android.app.Application
import android.app.NotificationManager
import android.content.Context
import coil3.ImageLoader
import coil3.PlatformContext
import coil3.SingletonImageLoader
import io.github.hipotures.bring.ffi.Moon
import io.github.hipotures.bring.media.RingImageFetcher
import io.github.hipotures.bring.media.RingImageKeyer
import io.github.hipotures.bring.media.RingThumbFetcher
import io.github.hipotures.bring.media.RingThumbKeyer
import io.github.hipotures.bring.sync.RouteStatus
import io.github.hipotures.bring.sync.SyncLog
import kotlinx.coroutines.flow.MutableStateFlow
import java.io.File

class BringApp : Application(), SingletonImageLoader.Factory {
    val prefs: Prefs by lazy { Prefs(this) }
    val tokens: TokenStore by lazy { TokenStore(this) }
    /** Finished sync rounds, for the activity screen. */
    val syncLog: SyncLog by lazy { SyncLog(filesDir.resolve("sync-log.tsv")) }

    /** Last route the engine selected (UI and worker both update it). */
    val route = MutableStateFlow(RouteStatus.UNKNOWN)

    /**
     * The sync engine. Opening it touches SQLite, so the first access must
     * happen off the main thread (all callers use Dispatchers.IO).
     */
    val moon: Moon by lazy {
        Moon.open(File(filesDir, "moon.db").path).also { m ->
            tokens.load()?.let(m::setDeviceToken)
            prefs.offloadThresholdBytes?.let(m::setOffloadThreshold)
        }
    }

    override fun onCreate() {
        super.onCreate()
        // Sync no longer posts notifications; remove the channel older builds made.
        getSystemService(NotificationManager::class.java)?.run {
            cancelAll()
            deleteNotificationChannel("sync")
        }
    }

    /** Coil: Ring thumbnails and images go through the engine's pinned client. */
    override fun newImageLoader(context: PlatformContext): ImageLoader =
        ImageLoader.Builder(context)
            .components {
                add(RingThumbFetcher.Factory(this@BringApp) { moon })
                add(RingThumbKeyer)
                add(RingImageFetcher.Factory { moon })
                add(RingImageKeyer)
            }
            .build()
}

val Context.bring: BringApp get() = applicationContext as BringApp
