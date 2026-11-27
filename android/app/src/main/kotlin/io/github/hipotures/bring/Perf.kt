package io.github.hipotures.bring

import android.util.Log
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * Lightweight timing log for the Demo-mode load test. Active only while Demo
 * mode is on; each line goes to logcat (tag `BringPerf`) and to `perf.log`
 * in the app's files dir (pull it with `run-as`, see docs/ANDROID_DEV.md).
 */
object Perf {
    @Volatile var enabled = false
    @Volatile private var file: File? = null
    private val stamp = SimpleDateFormat("HH:mm:ss.SSS", Locale.US)

    fun init(dir: File) {
        file = File(dir, "perf.log")
    }

    fun clear() {
        file?.delete()
    }

    private fun heapMiB(): Long {
        val rt = Runtime.getRuntime()
        return (rt.totalMemory() - rt.freeMemory()) / (1024 * 1024)
    }

    fun note(text: String) {
        if (!enabled) return
        val line = "${stamp.format(Date())} heap=${heapMiB()}MiB/${Runtime.getRuntime().maxMemory() / (1024 * 1024)}MiB $text"
        Log.i("BringPerf", line)
        runCatching { file?.appendText(line + "\n") }
    }

    /** Runs [block], logging how long it took when enabled. */
    inline fun <T> time(label: String, block: () -> T): T {
        if (!enabled) return block()
        val t0 = System.nanoTime()
        val r = block()
        note("$label ${(System.nanoTime() - t0) / 1_000_000} ms")
        return r
    }
}
