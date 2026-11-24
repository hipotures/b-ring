package io.github.hipotures.bring.share

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ShareNamesTest {
    @Test fun keepsOrdinaryNames() {
        assertEquals("IMG_0042.jpg", sanitizeFileName("IMG_0042.jpg", "x"))
        assertEquals("Zażółć gęślą.pdf", sanitizeFileName("Zażółć gęślą.pdf", "x"))
    }

    @Test fun stripsSeparatorsControlCharsAndLeadingDots() {
        assertEquals("passwd", sanitizeFileName("../../etc/passwd", "x"))
        assertEquals("evil.txt", sanitizeFileName("C:\\tmp\\evil.txt", "x"))
        assertEquals("ab.txt", sanitizeFileName("a\u0000b\n.txt", "x"))
        assertEquals("hidden", sanitizeFileName(" .hidden ", "x"))
    }

    @Test fun fallsBackWhenNothingIsLeft() {
        assertEquals("shared-1", sanitizeFileName(null, "shared-1"))
        assertEquals("shared-1", sanitizeFileName("...", "shared-1"))
        assertEquals("shared-1", sanitizeFileName("dir/", "shared-1"))
    }

    @Test fun truncatesToUtf8BytesKeepingTheExtension() {
        val name = sanitizeFileName("ż".repeat(300) + ".jpeg", "x", maxBytes = 50)
        assertTrue(name.endsWith(".jpeg"))
        assertTrue(name.toByteArray(Charsets.UTF_8).size <= 50)
        val emoji = sanitizeFileName("😀".repeat(40) + ".png", "x", maxBytes = 30)
        assertTrue(emoji.endsWith(".png"))
        assertTrue(emoji.dropLast(4).all { it.isSurrogate() } && emoji.dropLast(4).length % 2 == 0)
    }
}
