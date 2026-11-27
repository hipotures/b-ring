package io.github.hipotures.bring.browser

import io.github.hipotures.bring.ffi.FolderSummary
import org.junit.Assert.assertEquals
import org.junit.Test

class WatchedFoldersTest {
    private val dcim = FolderSummary("DCIM", "DCIM", 3UL, 30UL)

    @Test fun addsEmptyWatchedFoldersInNameOrder() {
        val out = withWatchedFolders(listOf(dcim), "", listOf("Download", "DCIM/quest", "alpha/x/y"))
        assertEquals(listOf("alpha", "DCIM", "Download"), out.map { it.name })
        assertEquals(0UL, out.first { it.name == "alpha" }.files)
        assertEquals("Download", out.first { it.name == "Download" }.path)
    }

    @Test fun onlyDirectChildrenOfTheCurrentFolder() {
        val out = withWatchedFolders(emptyList(), "DCIM", listOf("DCIM/quest/raw", "DCIM-other", "Download"))
        assertEquals(listOf("DCIM/quest"), out.map { it.path })
    }

    @Test fun existingFolderIsKept() {
        assertEquals(listOf(dcim), withWatchedFolders(listOf(dcim), "", listOf("DCIM/pixel")))
    }
}
