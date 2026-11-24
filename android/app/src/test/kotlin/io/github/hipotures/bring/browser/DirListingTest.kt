package io.github.hipotures.bring.browser

import org.junit.Assert.assertEquals
import org.junit.Test

class DirListingTest {
    private data class F(val path: String, val size: Long, val mtime: Long)

    private val files = listOf(
        F("DCIM/pixel-8/a.jpg", 10, 1),
        F("DCIM/pixel-8/b.jpg", 20, 3),
        F("DCIM/quest/c.mp4", 300, 2),
        F("Download/x.pdf", 5, 9),
        F("readme.txt", 1, 4),
        F("dcim-not-a-child.txt", 1, 5),
    )

    private fun list(dir: String) =
        listDir(files, dir, { it.path }, { it.size }, compareByDescending { it.mtime })

    @Test fun rootListsFoldersThenFiles() {
        assertEquals(
            listOf(
                DirRow.Folder("DCIM", "DCIM", 3, 330),
                DirRow.Folder("Download", "Download", 1, 5),
                DirRow.File(files[5], "dcim-not-a-child.txt"),
                DirRow.File(files[4], "readme.txt"),
            ),
            list(""),
        )
    }

    @Test fun subfolderAggregatesAndOrdersNewestFirst() {
        assertEquals(
            listOf(DirRow.Folder("pixel-8", "DCIM/pixel-8", 2, 30), DirRow.Folder("quest", "DCIM/quest", 1, 300)),
            list("DCIM"),
        )
        assertEquals(
            listOf(DirRow.File(files[1], "b.jpg"), DirRow.File(files[0], "a.jpg")),
            list("DCIM/pixel-8/"),
        )
    }

    @Test fun prefixMustBeAWholeComponent() {
        assertEquals(emptyList<DirRow<F>>(), list("DCI"))
    }

    @Test fun breadcrumbsAndParents() {
        assertEquals(listOf("a" to "a", "b" to "a/b", "c" to "a/b/c"), crumbs("a/b/c"))
        assertEquals(emptyList<Pair<String, String>>(), crumbs(""))
        assertEquals("a/b", parentOf("a/b/c"))
        assertEquals("", parentOf("a"))
    }
}
