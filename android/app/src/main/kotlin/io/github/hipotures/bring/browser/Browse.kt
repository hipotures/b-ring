package io.github.hipotures.bring.browser

import io.github.hipotures.bring.ffi.DirPage
import io.github.hipotures.bring.ffi.Entry
import io.github.hipotures.bring.ffi.EntryState
import io.github.hipotures.bring.ffi.FolderSummary
import io.github.hipotures.bring.ffi.QueuedSummary
import io.github.hipotures.bring.ffi.StateCounts

/**
 * What the browser shows: one folder of one Ring, read from the index
 * (never the whole index). [files] holds the first [limit] files directly in
 * [dir]; [total] is how many there are.
 */
data class Browse(
    val ringId: String,
    val dir: String,
    val state: EntryState?,
    val limit: Int,
    val folders: List<FolderSummary>,
    val files: List<Entry>,
    val total: Long,
    /** Counts under [dir] (subfolders included) per state, whatever [state] is. */
    val counts: StateCounts,
    val queued: QueuedSummary,
) {
    companion object {
        fun of(
            ringId: String,
            dir: String,
            state: EntryState?,
            limit: Int,
            page: DirPage,
            counts: StateCounts,
            queued: QueuedSummary,
        ) = Browse(ringId, dir, state, limit, page.folders, page.files, page.totalFiles.toLong(), counts, queued)
    }
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
