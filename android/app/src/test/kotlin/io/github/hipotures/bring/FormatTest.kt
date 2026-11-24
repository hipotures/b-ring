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
}
