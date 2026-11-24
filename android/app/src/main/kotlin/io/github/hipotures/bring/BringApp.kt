package io.github.hipotures.bring

import android.app.Application
import android.content.Context
import io.github.hipotures.bring.ffi.Moon
import io.github.hipotures.bring.sync.Notifications
import io.github.hipotures.bring.sync.RouteStatus
import kotlinx.coroutines.flow.MutableStateFlow
import java.io.File

class BringApp : Application() {
    val prefs: Prefs by lazy { Prefs(this) }
    val tokens: TokenStore by lazy { TokenStore(this) }

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
        Notifications.createChannels(this)
    }
}

val Context.bring: BringApp get() = applicationContext as BringApp
