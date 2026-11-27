package io.github.hipotures.bring.browser

import io.github.hipotures.bring.ffi.Entry
import io.github.hipotures.bring.ffi.EntryState
import io.github.hipotures.bring.ffi.FolderSummary
import io.github.hipotures.bring.ffi.QueuedSummary
import io.github.hipotures.bring.ffi.StateCounts

/**
 * What the browser shows: one folder of one Ring, read from the index (never the whole
 * index) and filled in step by step, so the screen is up at once and the numbers arrive as
 * their queries finish. [foldersReady] / [filesReady] say whether the lists are loaded;
 * [counts] and [queued] are null until read. [files] holds the first [limit] files directly
 * in [dir]; [total] is how many there are.
 */
data class Browse(
    val ringId: String,
    val dir: String,
    val state: EntryState?,
    val limit: Int,
    val folders: List<FolderSummary> = emptyList(),
    val foldersReady: Boolean = false,
    val files: List<Entry> = emptyList(),
    val total: Long = 0,
    val filesReady: Boolean = false,
    /** Counts under [dir] (subfolders included) per state, whatever [state] is. */
    val counts: StateCounts? = null,
    val queued: QueuedSummary? = null,
) {
    val ready: Boolean get() = foldersReady && filesReady
}

/**
 * [folders] plus the watched folders directly under [dir] that hold no files yet
 * (shown with `0` files), merged in name order like the index listing.
 */
fun withWatchedFolders(folders: List<FolderSummary>, dir: String, watched: List<String>): List<FolderSummary> {
    if (watched.isEmpty()) return folders
    val prefix = if (dir.isEmpty()) "" else "${dir.trimEnd('/')}/"
    val known = folders.mapTo(HashSet()) { it.name }
    val extra = mutableMapOf<String, FolderSummary>()
    for (path in watched) {
        val clean = path.trim('/')
        if (clean.isEmpty() || !clean.startsWith(prefix)) continue
        val name = clean.substring(prefix.length).substringBefore('/')
        if (name.isEmpty() || name in known || name in extra) continue
        extra[name] = FolderSummary(name, prefix + name, 0UL, 0UL)
    }
    if (extra.isEmpty()) return folders
    return (folders + extra.values).sortedWith(compareBy(String.CASE_INSENSITIVE_ORDER) { it.name })
}
