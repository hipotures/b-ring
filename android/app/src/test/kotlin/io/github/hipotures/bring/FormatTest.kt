package io.github.hipotures.bring

import org.junit.Assert.assertEquals
import org.junit.Test
import java.time.ZoneOffset

class FormatTest {
    @Test fun bytesUseBinaryUnits() {
        assertEquals("0 B", formatBytes(0))
        assertEquals("1023 B", formatBytes(1023))
        assertEquals("1.0 KiB", formatBytes(1024))
        assertEquals("1.5 MiB", formatBytes(1536L * 1024))
        assertEquals("150 MiB", formatBytes(150L * 1024 * 1024))
        assertEquals("2.0 GiB", formatBytes(2L * 1024 * 1024 * 1024))
    }

    @Test fun mtimeIsNanosecondsSinceEpoch() {
        assertEquals("2025-11-24 16:00", formatMtime(1_764_000_000_000_000_000, ZoneOffset.UTC))
    }

    @Test fun whenIsRelativeToToday() {
        val now = 1_764_000_000_000L // 2025-11-24 16:00 UTC
        assertEquals("Today 15:30", formatWhen(now - 30 * 60_000, now, ZoneOffset.UTC))
        assertEquals("Yesterday 16:00", formatWhen(now - 86_400_000, now, ZoneOffset.UTC))
        assertEquals("2025-11-21 16:00", formatWhen(now - 3 * 86_400_000L, now, ZoneOffset.UTC))
    }

    @Test fun durations() {
        assertEquals("0.8 s", formatDuration(800))
        assertEquals("42 s", formatDuration(42_000))
        assertEquals("3 min 05 s", formatDuration(185_000))
    }
}
