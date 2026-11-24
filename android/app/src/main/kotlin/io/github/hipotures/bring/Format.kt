package io.github.hipotures.bring

import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale

/** Binary units, one decimal under 100: `0 B`, `999 B`, `1.5 KiB`, `120 MiB`. */
fun formatBytes(bytes: Long): String {
    if (bytes < 1024) return "$bytes B"
    val units = arrayOf("KiB", "MiB", "GiB", "TiB")
    var v = bytes / 1024.0
    var i = 0
    while (v >= 1024 && i < units.lastIndex) {
        v /= 1024
        i++
    }
    return if (v < 100) String.format(Locale.ROOT, "%.1f %s", v, units[i])
    else String.format(Locale.ROOT, "%.0f %s", v, units[i])
}

fun formatBytes(bytes: ULong): String = formatBytes(bytes.toLong())

private val stamp = DateTimeFormatter.ofPattern("yyyy-MM-dd HH:mm", Locale.ROOT)

/** File mtime (ns since the epoch) as local `yyyy-MM-dd HH:mm`. */
fun formatMtime(mtimeNs: Long, zone: ZoneId = ZoneId.systemDefault()): String =
    stamp.format(Instant.ofEpochSecond(0, mtimeNs).atZone(zone))
