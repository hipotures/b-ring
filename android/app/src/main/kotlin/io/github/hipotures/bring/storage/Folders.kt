package io.github.hipotures.bring.storage

import android.content.Context
import android.os.storage.StorageManager
import java.io.File

/** A storage volume the folder picker can start from. */
data class Volume(val label: String, val root: File)

/** Internal storage first, then SD cards / USB drives that are mounted. */
fun volumes(context: Context): List<Volume> {
    val sm = context.getSystemService(StorageManager::class.java)
    val found = sm?.storageVolumes.orEmpty().mapNotNull { v ->
        val dir = v.directory ?: return@mapNotNull null
        Volume(if (v.isPrimary) "Internal" else v.getDescription(context), dir)
    }
    return found.sortedByDescending { it.root == StorageAccess.root() }
        .ifEmpty { listOf(Volume("Internal", StorageAccess.root())) }
}

/** Visible subfolders of [dir], by name, ignoring case; hidden (dot) ones are left out. */
fun subfolders(dir: File): List<File> =
    dir.listFiles { f -> f.isDirectory && !f.name.startsWith(".") }
        ?.sortedWith(compareBy(String.CASE_INSENSITIVE_ORDER) { it.name })
        .orEmpty()
