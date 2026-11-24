package io.github.hipotures.bring.media

import android.content.Context
import coil3.ImageLoader
import coil3.decode.DataSource
import coil3.decode.ImageSource
import coil3.fetch.FetchResult
import coil3.fetch.Fetcher
import coil3.fetch.SourceFetchResult
import coil3.key.Keyer
import coil3.request.Options
import io.github.hipotures.bring.ffi.Moon
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okio.Buffer
import okio.FileSystem
import okio.Path.Companion.toOkioPath
import java.io.File

/** A Ring thumbnail (`thumb` BLAKE3), fetched through the engine's pinned client. */
data class RingThumb(val hash: String)

/** A whole Ring file, for full-screen images. */
data class RingImage(val fileId: Long, val blake3: String?, val size: ULong)

/**
 * Thumbnails are immutable (content-addressed), so they are cached on disk
 * forever under `cacheDir/thumbs`.
 */
class RingThumbFetcher(
    private val context: Context,
    private val moon: () -> Moon,
    private val data: RingThumb,
) : Fetcher {
    override suspend fun fetch(): FetchResult = withContext(Dispatchers.IO) {
        val dir = File(context.cacheDir, "thumbs").apply { mkdirs() }
        val file = File(dir, "${data.hash}.jpg")
        val source = if (file.isFile) {
            DataSource.DISK
        } else {
            val bytes = moon().thumbnail(data.hash)
            val tmp = File(dir, "${data.hash}.tmp")
            tmp.writeBytes(bytes)
            tmp.renameTo(file)
            DataSource.NETWORK
        }
        SourceFetchResult(ImageSource(file.toOkioPath(), FileSystem.SYSTEM), "image/jpeg", source)
    }

    class Factory(private val context: Context, private val moon: () -> Moon) : Fetcher.Factory<RingThumb> {
        override fun create(data: RingThumb, options: Options, imageLoader: ImageLoader) =
            RingThumbFetcher(context, moon, data)
    }
}

class RingImageFetcher(private val moon: () -> Moon, private val data: RingImage, private val options: Options) : Fetcher {
    override suspend fun fetch(): FetchResult = withContext(Dispatchers.IO) {
        val bytes = moon().readRange(data.fileId, 0u, data.size)
        SourceFetchResult(ImageSource(Buffer().write(bytes), options.fileSystem), null, DataSource.NETWORK)
    }

    class Factory(private val moon: () -> Moon) : Fetcher.Factory<RingImage> {
        override fun create(data: RingImage, options: Options, imageLoader: ImageLoader) =
            RingImageFetcher(moon, data, options)
    }
}

object RingThumbKeyer : Keyer<RingThumb> {
    override fun key(data: RingThumb, options: Options) = "thumb:${data.hash}"
}

object RingImageKeyer : Keyer<RingImage> {
    override fun key(data: RingImage, options: Options) =
        "ring:${data.fileId}:${data.blake3.orEmpty()}"
}
