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

private val clock = DateTimeFormatter.ofPattern("HH:mm", Locale.ROOT)

/** "Today 19:24", "Yesterday 08:10", else "2026-11-20 19:24". */
fun formatWhen(epochMs: Long, nowMs: Long = System.currentTimeMillis(), zone: ZoneId = ZoneId.systemDefault()): String {
    val at = Instant.ofEpochMilli(epochMs).atZone(zone)
    val today = Instant.ofEpochMilli(nowMs).atZone(zone).toLocalDate()
    return when (at.toLocalDate()) {
        today -> "Today ${clock.format(at)}"
        today.minusDays(1) -> "Yesterday ${clock.format(at)}"
        else -> stamp.format(at)
    }
}

/** "0.8 s", "42 s", "3 min 05 s". */
fun formatDuration(ms: Long): String = when {
    ms < 10_000 -> String.format(Locale.ROOT, "%.1f s", ms / 1000.0)
    ms < 60_000 -> "${ms / 1000} s"
    else -> String.format(Locale.ROOT, "%d min %02d s", ms / 60_000, ms / 1000 % 60)
}
