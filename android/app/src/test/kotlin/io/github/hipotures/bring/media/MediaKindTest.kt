package io.github.hipotures.bring.media

import org.junit.Assert.assertEquals
import org.junit.Test

class MediaKindTest {
    @Test fun mimeWins() {
        assertEquals(MediaKind.VIDEO, kindOf("video/mp4", "clip.bin"))
        assertEquals(MediaKind.IMAGE, kindOf("image/heic", "x"))
        assertEquals(MediaKind.OTHER, kindOf("application/pdf", "fake.jpg"))
    }

    @Test fun extensionWhenMimeIsUnknown() {
        assertEquals(MediaKind.IMAGE, kindOf(null, "DCIM/IMG_1.JPG"))
        assertEquals(MediaKind.AUDIO, kindOf("application/octet-stream", "rec.opus"))
        assertEquals(MediaKind.OTHER, kindOf(null, "notes"))
    }

    @Test fun dotFilesHaveNoExtension() {
        assertEquals("", extensionOf(".nomedia"))
        assertEquals("gz", extensionOf("a/b.tar.gz"))
    }
}
