package io.github.hipotures.bring.share

/**
 * A safe single path component for a shared file: no separators or control
 * characters, no leading dots (dot-files are skipped by scans), at most
 * [maxBytes] of UTF-8 with the extension kept.
 */
fun sanitizeFileName(raw: String?, fallback: String, maxBytes: Int = 200): String {
    var name = (raw ?: "")
        .substringAfterLast('/')
        .substringAfterLast('\\')
        .filter { it >= ' ' && it != '\u007f' }
        .trim()
        .trimStart('.')
        .trim()
    if (name.isEmpty()) name = fallback
    if (name.toByteArray(Charsets.UTF_8).size <= maxBytes) return name
    val dot = name.lastIndexOf('.')
    val ext = if (dot > 0 && name.length - dot <= 16) name.substring(dot) else ""
    var stem = name.substring(0, name.length - ext.length)
    while ((stem + ext).toByteArray(Charsets.UTF_8).size > maxBytes && stem.isNotEmpty()) {
        stem = stem.dropLast(1)
        // Never split a surrogate pair.
        if (stem.isNotEmpty() && stem.last().isHighSurrogate()) stem = stem.dropLast(1)
    }
    return stem + ext
}
