package io.github.hipotures.bring

import android.content.Context
import io.github.hipotures.bring.ffi.coreVersion

/** "app 0.1.0+8debb6f · core 0.1.0": the APK's versionName and the engine's. */
fun versionLine(context: Context): String {
    val app = runCatching {
        context.packageManager.getPackageInfo(context.packageName, 0).versionName
    }.getOrNull() ?: "?"
    val core = runCatching { coreVersion() }.getOrDefault("?")
    return "app $app · core $core"
}
