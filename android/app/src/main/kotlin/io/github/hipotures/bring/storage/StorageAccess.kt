package io.github.hipotures.bring.storage

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Environment
import android.provider.Settings
import java.io.File

/** `MANAGE_EXTERNAL_STORAGE` (§4): the engine reads watched folders by path. */
object StorageAccess {
    fun granted(): Boolean = Environment.isExternalStorageManager()

    fun requestIntent(context: Context): Intent =
        Intent(
            Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION,
            Uri.parse("package:${context.packageName}"),
        )

    /** Fallback for builds (e.g. some headsets) without the per-app screen. */
    fun fallbackIntent(): Intent = Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION)

    @Suppress("DEPRECATION") // still the path the engine needs, not a SAF URI
    fun root(): File = Environment.getExternalStorageDirectory()

    /** Standard top-level folders worth watching that exist on this device. */
    fun suggestions(root: File = root()): List<File> =
        listOf("DCIM", "Pictures", "Movies", "Download", "Documents", "Music", "Recordings")
            .map { File(root, it) }
            .filter { it.isDirectory }
}
