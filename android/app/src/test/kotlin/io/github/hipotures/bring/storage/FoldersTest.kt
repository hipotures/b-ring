package io.github.hipotures.bring.storage

import org.junit.Assert.assertEquals
import org.junit.Test
import java.nio.file.Files

class FoldersTest {
    @Test
    fun subfoldersAreSortedVisibleDirectories() {
        val root = Files.createTempDirectory("folders").toFile()
        try {
            listOf("b", "A", "c", ".hidden").forEach { root.resolve(it).mkdir() }
            root.resolve("file.txt").writeText("x")
            assertEquals(listOf("A", "b", "c"), subfolders(root).map { it.name })
            assertEquals(emptyList<String>(), subfolders(root.resolve("missing")).map { it.name })
        } finally {
            root.deleteRecursively()
        }
    }
}
