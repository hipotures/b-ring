package io.github.hipotures.bring.media

enum class MediaKind { IMAGE, VIDEO, AUDIO, OTHER }

private val IMAGE_EXT = setOf("jpg", "jpeg", "png", "gif", "webp", "bmp", "heic", "heif", "avif", "dng")
private val VIDEO_EXT = setOf("mp4", "m4v", "mov", "mkv", "webm", "3gp", "avi", "ts")
private val AUDIO_EXT = setOf("mp3", "m4a", "aac", "ogg", "opus", "flac", "wav", "amr")

/** By MIME type when the Ring sent one, else by extension (local-only files). */
fun kindOf(mime: String?, name: String): MediaKind {
    when {
        mime == null -> {}
        mime.startsWith("image/") -> return MediaKind.IMAGE
        mime.startsWith("video/") -> return MediaKind.VIDEO
        mime.startsWith("audio/") -> return MediaKind.AUDIO
        mime != "application/octet-stream" -> return MediaKind.OTHER
    }
    return when (extensionOf(name)) {
        in IMAGE_EXT -> MediaKind.IMAGE
        in VIDEO_EXT -> MediaKind.VIDEO
        in AUDIO_EXT -> MediaKind.AUDIO
        else -> MediaKind.OTHER
    }
}

/** Lower-case extension without the dot, or "" (dot-files have none). */
fun extensionOf(name: String): String {
    val base = name.substringAfterLast('/')
    val dot = base.lastIndexOf('.')
    return if (dot <= 0) "" else base.substring(dot + 1).lowercase()
}
