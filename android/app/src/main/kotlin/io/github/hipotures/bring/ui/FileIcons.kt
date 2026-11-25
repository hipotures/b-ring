package io.github.hipotures.bring.ui

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.path
import androidx.compose.ui.unit.dp
import io.github.hipotures.bring.media.MediaKind
import io.github.hipotures.bring.media.extensionOf
import io.github.hipotures.bring.media.kindOf

/**
 * Material Symbols Outlined file-type glyphs (24×24, from fonts.google.com/icons).
 * Used inside `Thumb` placeholders when a file has no rendered thumbnail yet.
 */
object FileIcons {
    fun forFile(mime: String?, name: String): ImageVector {
        when (kindOf(mime, name)) {
            MediaKind.IMAGE -> return Image
            MediaKind.VIDEO -> return Video
            MediaKind.AUDIO -> return Audio
            MediaKind.OTHER -> {}
        }
        val ext = extensionOf(name)
        return when {
            mime == "application/pdf" ||
                mime == "application/epub+zip" ||
                mime?.startsWith("text/") == true ||
                ext in DOC_EXTS -> Document
            ext in ARCHIVE_EXTS || mime in ARCHIVE_MIMES -> Archive
            ext in CODE_EXTS -> Code
            else -> Generic
        }
    }

    private val DOC_EXTS = setOf("pdf", "doc", "docx", "odt", "rtf", "txt", "md", "csv", "xls", "xlsx", "ods", "ppt", "pptx", "epub")
    private val ARCHIVE_EXTS = setOf("zip", "tar", "gz", "tgz", "bz2", "xz", "zst", "7z", "rar", "apk")
    private val ARCHIVE_MIMES = setOf("application/zip", "application/gzip", "application/zstd", "application/x-tar", "application/x-7z-compressed")
    private val CODE_EXTS = setOf("kt", "kts", "java", "rs", "py", "js", "ts", "tsx", "jsx", "json", "toml", "yaml", "yml", "xml", "html", "css", "sh", "sql", "gpx", "c", "cpp", "h", "go")

    /** Material `image` (outlined) */
    val Image: ImageVector by lazy {
        materialIcon("file_image") {
            // Outer frame
            moveTo(21f, 19f)
            verticalLineTo(5f)
            curveTo(21f, 3.9f, 20.1f, 3f, 19f, 3f)
            horizontalLineTo(5f)
            curveTo(3.9f, 3f, 3f, 3.9f, 3f, 5f)
            verticalLineTo(19f)
            curveTo(3f, 20.1f, 3.9f, 21f, 5f, 21f)
            horizontalLineTo(19f)
            curveTo(20.1f, 21f, 21f, 20.1f, 21f, 19f)
            close()
            moveTo(19f, 19f)
            horizontalLineTo(5f)
            verticalLineTo(5f)
            horizontalLineTo(19f)
            verticalLineTo(19f)
            close()
            // Mountains
            moveTo(8.5f, 13.5f)
            lineTo(11f, 16.51f)
            lineTo(14.5f, 12f)
            lineTo(19f, 18f)
            horizontalLineTo(5f)
            lineTo(8.5f, 13.5f)
            close()
        }
    }

    /** Material `movie` (outlined) */
    val Video: ImageVector by lazy {
        materialIcon("file_video") {
            moveTo(4f, 6.47f)
            lineTo(5.76f, 10f)
            horizontalLineTo(20f)
            verticalLineTo(18f)
            horizontalLineTo(4f)
            verticalLineTo(6.47f)
            moveTo(22f, 4f)
            horizontalLineTo(18f)
            lineTo(20f, 8f)
            horizontalLineTo(17f)
            lineTo(15f, 4f)
            horizontalLineTo(13f)
            lineTo(15f, 8f)
            horizontalLineTo(12f)
            lineTo(10f, 4f)
            horizontalLineTo(8f)
            lineTo(10f, 8f)
            horizontalLineTo(7f)
            lineTo(5f, 4f)
            horizontalLineTo(4f)
            curveTo(2.9f, 4f, 2.01f, 4.9f, 2.01f, 6f)
            lineTo(2f, 18f)
            curveTo(2f, 19.1f, 2.9f, 20f, 4f, 20f)
            horizontalLineTo(20f)
            curveTo(21.1f, 20f, 22f, 19.1f, 22f, 18f)
            verticalLineTo(4f)
            close()
        }
    }

    /** Material `music_note` (outlined) */
    val Audio: ImageVector by lazy {
        materialIcon("file_audio") {
            moveTo(12f, 3f)
            verticalLineTo(13.55f)
            curveTo(11.41f, 13.21f, 10.73f, 13f, 10f, 13f)
            curveTo(7.79f, 13f, 6f, 14.79f, 6f, 17f)
            curveTo(6f, 19.21f, 7.79f, 21f, 10f, 21f)
            curveTo(12.21f, 21f, 14f, 19.21f, 14f, 17f)
            verticalLineTo(7f)
            horizontalLineTo(18f)
            verticalLineTo(3f)
            horizontalLineTo(12f)
            close()
            moveTo(10f, 19f)
            curveTo(8.9f, 19f, 8f, 18.1f, 8f, 17f)
            curveTo(8f, 15.9f, 8.9f, 15f, 10f, 15f)
            curveTo(11.1f, 15f, 12f, 15.9f, 12f, 17f)
            curveTo(12f, 18.1f, 11.1f, 19f, 10f, 19f)
            close()
        }
    }

    /** Material `description` (outlined) */
    val Document: ImageVector by lazy {
        materialIcon("file_doc") {
            moveTo(8f, 16f)
            horizontalLineTo(16f)
            verticalLineTo(18f)
            horizontalLineTo(8f)
            close()
            moveTo(8f, 12f)
            horizontalLineTo(16f)
            verticalLineTo(14f)
            horizontalLineTo(8f)
            close()
            moveTo(14f, 2f)
            horizontalLineTo(6f)
            curveTo(4.9f, 2f, 4f, 2.9f, 4f, 4f)
            verticalLineTo(20f)
            curveTo(4f, 21.1f, 4.89f, 22f, 5.99f, 22f)
            horizontalLineTo(18f)
            curveTo(19.1f, 22f, 20f, 21.1f, 20f, 20f)
            verticalLineTo(8f)
            lineTo(14f, 2f)
            close()
            moveTo(18f, 20f)
            horizontalLineTo(6f)
            verticalLineTo(4f)
            horizontalLineTo(13f)
            verticalLineTo(9f)
            horizontalLineTo(18f)
            verticalLineTo(20f)
            close()
        }
    }

    /** Material `inventory_2` (outlined archive box) */
    val Archive: ImageVector by lazy {
        materialIcon("file_archive") {
            moveTo(20f, 2f)
            horizontalLineTo(4f)
            curveTo(3f, 2f, 2f, 2.9f, 2f, 4f)
            verticalLineTo(7.01f)
            curveTo(2f, 7.73f, 2.43f, 8.35f, 3f, 8.7f)
            verticalLineTo(20f)
            curveTo(3f, 21.1f, 4.1f, 22f, 5f, 22f)
            horizontalLineTo(19f)
            curveTo(19.9f, 22f, 21f, 21.1f, 21f, 20f)
            verticalLineTo(8.7f)
            curveTo(21.57f, 8.35f, 22f, 7.73f, 22f, 7.01f)
            verticalLineTo(4f)
            curveTo(22f, 2.9f, 21f, 2f, 20f, 2f)
            close()
            moveTo(19f, 20f)
            horizontalLineTo(5f)
            verticalLineTo(9f)
            horizontalLineTo(19f)
            verticalLineTo(20f)
            close()
            moveTo(20f, 7f)
            horizontalLineTo(4f)
            verticalLineTo(4f)
            horizontalLineTo(20f)
            verticalLineTo(7f)
            close()
            moveTo(9f, 12f)
            horizontalLineTo(15f)
            verticalLineTo(14f)
            horizontalLineTo(9f)
            close()
        }
    }

    /** Material `code` (outlined) */
    val Code: ImageVector by lazy {
        materialIcon("file_code") {
            moveTo(9.4f, 16.6f)
            lineTo(4.8f, 12f)
            lineTo(9.4f, 7.4f)
            lineTo(8f, 6f)
            lineTo(2f, 12f)
            lineTo(8f, 18f)
            lineTo(9.4f, 16.6f)
            close()
            moveTo(14.6f, 16.6f)
            lineTo(19.2f, 12f)
            lineTo(14.6f, 7.4f)
            lineTo(16f, 6f)
            lineTo(22f, 12f)
            lineTo(16f, 18f)
            lineTo(14.6f, 16.6f)
            close()
        }
    }

    /** Material `insert_drive_file` (outlined) */
    val Generic: ImageVector by lazy {
        materialIcon("file_generic") {
            moveTo(14f, 2f)
            horizontalLineTo(6f)
            curveTo(4.9f, 2f, 4.01f, 2.9f, 4.01f, 4f)
            lineTo(4f, 20f)
            curveTo(4f, 21.1f, 4.89f, 22f, 5.99f, 22f)
            horizontalLineTo(18f)
            curveTo(19.1f, 22f, 20f, 21.1f, 20f, 20f)
            verticalLineTo(8f)
            lineTo(14f, 2f)
            close()
            moveTo(18f, 20f)
            horizontalLineTo(6f)
            verticalLineTo(4f)
            horizontalLineTo(13f)
            verticalLineTo(9f)
            horizontalLineTo(18f)
            verticalLineTo(20f)
            close()
        }
    }

    private fun materialIcon(
        name: String,
        build: androidx.compose.ui.graphics.vector.PathBuilder.() -> Unit,
    ): ImageVector = ImageVector.Builder(name, 24.dp, 24.dp, 24f, 24f).apply {
        path(fill = SolidColor(Color.Black), pathBuilder = build)
    }.build()
}
