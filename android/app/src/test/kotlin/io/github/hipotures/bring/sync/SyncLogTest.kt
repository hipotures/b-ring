package io.github.hipotures.bring.sync

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import java.nio.file.Files

class SyncLogTest {
    private fun round(
        uploaded: Int = 0,
        manual: Boolean = false,
        error: String? = null,
        files: List<String> = emptyList(),
        at: Long = 1_000,
    ) = SyncRound(at, 1500, manual, "LAN", uploaded, uploaded * 10L, 0, 0, 0, 0, 0, error, files)

    @Test
    fun roundTripsThroughOneLine() {
        val r = round(uploaded = 2, manual = true, error = "bad\tthing\nhere", files = listOf("DCIM/a b.jpg", "x\ty"))
        val back = SyncRound.decode(SyncRound.encode(r))!!
        assertEquals(r.copy(error = "bad thing here", files = listOf("DCIM/a b.jpg", "x y")), back)
        assertEquals(round(), SyncRound.decode(SyncRound.encode(round())))
        val withRing = round().copy(receivedFiles = listOf("DCIM/x.jpg", "DCIM/y.jpg"))
        assertEquals(withRing, SyncRound.decode(SyncRound.encode(withRing)))
        assertNull(SyncRound.decode("garbage"))
    }

    @Test
    fun keepsOnlyRoundsWithSomethingToShowNewestFirstAndPersists() {
        val dir = Files.createTempDirectory("synclog").toFile()
        try {
            val file = dir.resolve("log.tsv")
            val log = SyncLog(file, keep = 2)
            log.add(round(uploaded = 1, at = 1))
            log.add(round(at = 2)) // idle background check: not listed
            log.add(round(manual = true, at = 3)) // idle manual check: not listed either
            log.add(round(error = "x", at = 4))
            assertEquals(listOf(4L, 1L), log.rounds.value.map { it.endedAt })
            assertEquals(4L, log.lastChecked.value)
            assertEquals(listOf(4L, 1L), SyncLog(file).rounds.value.map { it.endedAt })
        } finally {
            dir.deleteRecursively()
        }
    }

    @Test
    fun titlesAndDetails() {
        assertEquals("Uploaded 2 files (20 B)", roundTitle(round(uploaded = 2)))
        assertEquals("Sync failed", roundTitle(round(error = "Ring unreachable")))
        assertEquals("Ring unreachable", roundDetail(round(error = "Ring unreachable")))
        assertEquals("Nothing new", roundTitle(round()))
        assertEquals("", roundDetail(round()))
    }

    @Test
    fun dropsIdleManualRoundsSavedByOlderVersions() {
        val dir = Files.createTempDirectory("synclog").toFile()
        try {
            val file = dir.resolve("log.tsv")
            file.writeText(
                listOf(round(manual = true, at = 3), round(uploaded = 1, at = 2), round(manual = true, at = 1))
                    .joinToString("\n", transform = SyncRound::encode),
            )
            assertEquals(listOf(2L), SyncLog(file).rounds.value.map { it.endedAt })
            // The file itself was cleaned, not just the view of it.
            assertEquals(1, file.readLines().size)
        } finally {
            dir.deleteRecursively()
        }
    }
}
