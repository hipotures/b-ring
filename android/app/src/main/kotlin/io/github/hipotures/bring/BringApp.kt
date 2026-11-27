package io.github.hipotures.bring

import android.app.Application
import android.app.NotificationManager
import android.content.Context
import coil3.ImageLoader
import coil3.PlatformContext
import coil3.SingletonImageLoader
import io.github.hipotures.bring.ffi.Moon
import io.github.hipotures.bring.media.LocalVideoThumbFetcher
import io.github.hipotures.bring.media.LocalVideoThumbKeyer
import io.github.hipotures.bring.media.RingImageFetcher
import io.github.hipotures.bring.media.RingImageKeyer
import io.github.hipotures.bring.media.RingThumbFetcher
import io.github.hipotures.bring.media.RingThumbKeyer
import io.github.hipotures.bring.sync.Notifications
import io.github.hipotures.bring.sync.RouteStatus
import io.github.hipotures.bring.sync.SyncLog
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.sync.Mutex
import java.io.File

class BringApp : Application(), SingletonImageLoader.Factory {
    val prefs: Prefs by lazy { Prefs(this) }
    val tokens: TokenStore by lazy { TokenStore(this) }
    /** Finished sync rounds, for the activity screen. */
    val syncLog: SyncLog by lazy { SyncLog(filesDir.resolve("sync-log.tsv")) }

    /** Last route selected per Ring (`ringId` → [RouteStatus]). */
    val routes = MutableStateFlow<Map<String, RouteStatus>>(emptyMap())

    fun setRoute(ringId: String, status: RouteStatus) {
        routes.update { it + (ringId to status) }
    }

    fun removeRoute(ringId: String) {
        routes.update { it - ringId }
    }

    /** Serialises SyncWorker rounds and Settings verification so workers never fight over FGS/progress/cancel. */
    val syncMutex = Mutex()

    /** Live progress of the worker currently holding [syncMutex]. */
    val activeSync = MutableStateFlow(SyncUi())

    /**
     * The sync engine. Opening it touches SQLite, so the first access must
     * happen off the main thread (all callers use Dispatchers.IO).
     */
    val moon: Moon by lazy {
        Moon.open(File(filesDir, "moon.db").path).also { m ->
            prefs.offloadThresholdBytes?.let(m::setOffloadThreshold)
            val rings = runCatching { m.rings() }.getOrDefault(emptyList())
            if (rings.size == 1) {
                tokens.migrateLegacy(rings.first().ringId)
            }
            for (r in rings) {
                tokens.load(r.ringId)?.let { tok -> m.setRingToken(r.ringId, tok) }
            }
        }
    }

    @Volatile private var demoDb: Moon? = null

    /**
     * The generated large demo database (`dist/gen-demo-db.py`), or null when it is not on the
     * phone. Always a separate file from [moon], so the real index is never touched. It is pushed to
     * the app's external files dir (`/sdcard/Android/data/<package>/files/demo.db`, which adb
     * can write even when the release build has no `run-as`) and copied into private storage
     * when new or changed. Re-checked on each call until it exists, so it can be pushed while
     * the app is running.
     */
    fun demoMoon(): Moon? = demoDb ?: synchronized(this) {
        demoDb ?: run {
            val local = filesDir.resolve("demo.db")
            val pushed = getExternalFilesDir(null)?.resolve("demo.db")?.takeIf { it.isFile }
            if (pushed != null && (!local.isFile || local.length() != pushed.length() || local.lastModified() < pushed.lastModified())) {
                runCatching {
                    pushed.copyTo(File(local.path + ".tmp"), overwrite = true)
                    File(local.path + ".tmp").renameTo(local)
                }
            }
            local.takeIf { it.isFile }
                ?.let { f -> runCatching { Moon.open(f.path) }.getOrNull() }
                ?.also { demoDb = it }
        }
    }

    override fun onCreate() {
        super.onCreate()
        // Open the index (native library, migrations, runtime) while the first frame is drawn,
        // so the Rings are ready sooner. Errors surface later, where the index is used.
        Thread({ runCatching { if (prefs.demoMode.value) demoMoon() ?: moon else moon } }, "moon-warmup").start()
        getSystemService(NotificationManager::class.java)?.cancelAll()
        Notifications.createChannel(this)
    }

    /** Coil: Ring thumbnails and images go through the engine's pinned client. */
    override fun newImageLoader(context: PlatformContext): ImageLoader =
        ImageLoader.Builder(context)
            .components {
                add(RingThumbFetcher.Factory(this@BringApp) { moon })
                add(RingThumbKeyer)
                add(RingImageFetcher.Factory { moon })
                add(RingImageKeyer)
                add(LocalVideoThumbFetcher.Factory())
                add(LocalVideoThumbKeyer)
            }
            .build()
}

val Context.bring: BringApp get() = applicationContext as BringApp
