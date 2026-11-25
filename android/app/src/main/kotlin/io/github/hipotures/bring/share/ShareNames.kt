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

/**
 * Normalises a user-typed Ring destination folder (`"Invoices/2026"`,
 * `"/sm-s918b\\Docs/"` → `"sm-s918b/Docs"`). Drops empty, `.`, `..`,
 * control characters and the reserved `.bring` top-level folder; falls back
 * to [fallback] when nothing valid remains.
 */
fun sanitizeFolderPath(raw: String?, fallback: String = "Inbox"): String {
    val rawParts = (raw ?: "")
        .replace('\\', '/')
        .split('/')
        .map { seg ->
            seg.filter { it >= ' ' && it != '\u007f' }
                .trim()
        }
        .filter { it.isNotEmpty() }
    if (rawParts.firstOrNull() == ".bring") return fallback
    val parts = rawParts
        .map { it.trimStart('.').trim() }
        .filter { it.isNotEmpty() }
    if (parts.isEmpty()) return fallback
    return parts.joinToString("/")
}
