package io.github.hipotures.bring.browser

/** One row of a folder view. */
sealed interface DirRow<out T> {
    data class Folder(val name: String, val path: String, val files: Int, val bytes: Long) : DirRow<Nothing>
    data class File<T>(val item: T, val name: String) : DirRow<T>
}

/**
 * The direct children of [dir] (`""` is the root) among [items] with
 * '/'-separated paths: subfolders first, aggregated (file count, bytes) and
 * sorted by name, then files ordered by [fileOrder].
 */
fun <T> listDir(
    items: List<T>,
    dir: String,
    pathOf: (T) -> String,
    sizeOf: (T) -> Long,
    fileOrder: Comparator<T>,
): List<DirRow<T>> {
    val prefix = if (dir.isEmpty()) "" else "${dir.trimEnd('/')}/"
    val folders = sortedMapOf<String, LongArray>(String.CASE_INSENSITIVE_ORDER)
    val files = mutableListOf<T>()
    for (item in items) {
        val path = pathOf(item)
        if (!path.startsWith(prefix)) continue
        val rest = path.substring(prefix.length)
        val slash = rest.indexOf('/')
        if (slash < 0) {
            files += item
        } else {
            val agg = folders.getOrPut(rest.substring(0, slash)) { LongArray(2) }
            agg[0]++
            agg[1] += sizeOf(item)
        }
    }
    return buildList {
        folders.forEach { (name, agg) -> add(DirRow.Folder(name, prefix + name, agg[0].toInt(), agg[1])) }
        files.sortedWith(fileOrder).forEach { add(DirRow.File(it, pathOf(it).substringAfterLast('/'))) }
    }
}

/** `a/b/c` -> `[("a","a"), ("b","a/b"), ("c","a/b/c")]` for the breadcrumb bar. */
fun crumbs(dir: String): List<Pair<String, String>> {
    if (dir.isEmpty()) return emptyList()
    val parts = dir.split('/')
    return parts.indices.map { i -> parts[i] to parts.subList(0, i + 1).joinToString("/") }
}

fun parentOf(dir: String): String = dir.substringBeforeLast('/', "")
