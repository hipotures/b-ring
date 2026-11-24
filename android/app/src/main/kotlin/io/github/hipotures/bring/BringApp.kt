package io.github.hipotures.bring

import android.app.Application
import android.content.Context
import io.github.hipotures.bring.ffi.Moon
import java.io.File

class BringApp : Application() {
    val prefs: Prefs by lazy { Prefs(this) }
    val tokens: TokenStore by lazy { TokenStore(this) }

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
}

val Context.bring: BringApp get() = applicationContext as BringApp
