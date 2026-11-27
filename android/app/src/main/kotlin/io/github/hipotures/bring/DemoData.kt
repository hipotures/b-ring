package io.github.hipotures.bring

import io.github.hipotures.bring.ffi.Entry
import io.github.hipotures.bring.ffi.EntryState
import io.github.hipotures.bring.ffi.RingInfo
import io.github.hipotures.bring.ffi.WatchedFolder
import io.github.hipotures.bring.sync.RouteStatus
import io.github.hipotures.bring.sync.SyncRound

/**
 * Synthetic in-memory data shown when Demo mode is enabled in Settings.
 * Never touches `moon.db`, real Ring tokens or files on disk.
 */
/** Number of synthetic files Demo mode generates for the active Ring. */
enum class DemoScale(val label: String, val count: Int) {
    SAMPLE("Sample", 0),
    K10("10k", 10_000),
    K100("100k", 100_000),
    M1("1M", 1_000_000),
}

object DemoData {
    const val RING_HOME_NAS = "demo01a4f8c29e10b7d3e6a1c8402f9b"
    const val RING_WORKSTATION = "demo02b7e1d40a83f5c291e4b8a73d0c"
    const val RING_ARCHIVE_VPS = "demo03c9a2f71b64d8e305a1f2c94e7a"

    private val MIB = 1024UL * 1024UL
    private val GIB = 1024UL * MIB

    val rings: List<RingInfo> = listOf(
        RingInfo(
            id = 101L,
            ringId = RING_HOME_NAS,
            name = "home-nas",
            lanUrl = "https://192.168.1.50:7443",
            wanUrl = "https://nas.example.com",
            tlsCertSha256 = "d4e5f6a7b8c90123456789abcdef0123456789abcdef0123456789abcdef1234",
            deviceSlug = "pixel-9",
            deviceId = "a1b2c3d4e5f60718293a4b5c6d7e8f90",
            offloadThresholdBytes = 100UL * MIB,
            transferredFiles = 1842UL,
            transferredBytes = 45UL * GIB + 312UL * MIB,
        ),
        RingInfo(
            id = 102L,
            ringId = RING_WORKSTATION,
            name = "workstation",
            lanUrl = "https://192.168.1.24:7443",
            wanUrl = null,
            tlsCertSha256 = "91ab82cd73ef64015293a4b5c6d7e8f90123456789abcdef0123456789ab5678",
            deviceSlug = "pixel-9",
            deviceId = "a1b2c3d4e5f60718293a4b5c6d7e8f90",
            offloadThresholdBytes = 500UL * MIB,
            transferredFiles = 418UL,
            transferredBytes = 6UL * GIB + 740UL * MIB,
        ),
        RingInfo(
            id = 103L,
            ringId = RING_ARCHIVE_VPS,
            name = "archive-vps",
            lanUrl = "https://10.8.0.10:7443",
            wanUrl = "https://archive.example.org",
            tlsCertSha256 = "55aa66bb77cc88dd99ee00ff112233445566778899aabbccddeeff0011229012",
            deviceSlug = "pixel-9",
            deviceId = "a1b2c3d4e5f60718293a4b5c6d7e8f90",
            offloadThresholdBytes = ULong.MAX_VALUE,
            transferredFiles = 296UL,
            transferredBytes = 11UL * GIB + 410UL * MIB,
        ),
    )

    val routes: Map<String, RouteStatus> = mapOf(
        RING_HOME_NAS to RouteStatus.LAN,
        RING_WORKSTATION to RouteStatus.LAN,
        RING_ARCHIVE_VPS to RouteStatus.WAN,
    )

    fun foldersFor(ringId: String): List<WatchedFolder> = when (ringId) {
        RING_HOME_NAS -> listOf(
            WatchedFolder(1L, ringId, "/storage/emulated/0/DCIM/Camera", "pixel-9/Camera", true),
            WatchedFolder(2L, ringId, "/storage/emulated/0/Pictures/Screenshots", "pixel-9/Screenshots", true),
            WatchedFolder(3L, ringId, "/storage/emulated/0/Documents", "pixel-9/Documents", true),
            WatchedFolder(4L, ringId, "/storage/emulated/0/Recordings", "pixel-9/Recordings", true),
        )
        RING_WORKSTATION -> listOf(
            WatchedFolder(5L, ringId, "/storage/emulated/0/Documents", "pixel-9/Documents", true),
            WatchedFolder(6L, ringId, "/storage/emulated/0/Pictures/Screenshots", "pixel-9/Screenshots", true),
        )
        RING_ARCHIVE_VPS -> listOf(
            WatchedFolder(7L, ringId, "/storage/emulated/0/Documents/Invoices", "invoices", true),
        )
        else -> emptyList()
    }

    /** The demo Rings; at a load-test [scale] the home Ring's counters match the generated files. */
    fun ringsFor(scale: DemoScale): List<RingInfo> =
        if (scale == DemoScale.SAMPLE) rings else rings.map {
            if (it.ringId == RING_HOME_NAS) {
                it.copy(transferredFiles = scale.count.toULong(), transferredBytes = scale.count.toULong() * 3UL * MIB)
            } else it
        }

    private val MIMES = listOf(
        "image/jpeg" to "jpg", "image/heic" to "heic", "video/mp4" to "mp4", "application/pdf" to "pdf",
        "image/png" to "png", "audio/flac" to "flac", "text/plain" to "txt",
    )
    private val TOPS = listOf("pixel-9/Camera", "pixel-9/Screenshots", "pixel-9/Documents", "tablet/Books", "shared/Trips", "music", "projects", "backups")

    /**
     * [DemoScale.count] deterministic fake files in a realistic tree
     * (top/year/month/NNNN), with a state mix of about 60% remote-only,
     * 38% synced and 2% local-only. Real-sized Entry objects, so heap use is
     * representative.
     */
    private fun generated(ringId: String, count: Int): List<Entry> {
        val nowNs = System.currentTimeMillis() * 1_000_000L
        val hex = "0123456789abcdef"
        var seed = 0x9E3779B97F4A7C15uL.toLong()
        fun next(): Long { seed = seed * 6364136223846793005L + 1442695040888963407L; return seed ushr 33 }
        val out = ArrayList<Entry>(count)
        for (i in 0 until count) {
            val (mime, ext) = MIMES[(next() % MIMES.size).toInt()]
            val top = TOPS[i % TOPS.size]
            val path = "$top/${2020 + (i / 97) % 7}/${1 + (i / 13) % 12}/F${i.toString().padStart(7, '0')}.$ext"
            val r = (next() % 100).toInt()
            val state = when { r < 2 -> EntryState.LOCAL_ONLY; r < 40 -> EntryState.SYNCED; else -> EntryState.REMOTE_ONLY }
            val h = CharArray(64) { hex[(next() and 15).toInt()] }
            out += Entry(
                id = 1_000_000L + i,
                ringId = ringId,
                ringPath = path,
                localPath = if (state == EntryState.REMOTE_ONLY) null else "/storage/emulated/0/$path",
                remoteFileId = if (state == EntryState.LOCAL_ONLY) null else 1_000_000L + i,
                size = (next() % (50L * 1024 * 1024)).toULong(),
                mtimeNs = nowNs - (next() % (4_000L * 3_600_000_000_000L)),
                mime = mime,
                blake3 = if (state == EntryState.LOCAL_ONLY) null else String(h),
                thumb = null,
                state = state,
                suppressed = false,
                uploadPending = state == EntryState.LOCAL_ONLY,
                lastError = null,
            )
        }
        return out
    }

    fun entriesFor(ringId: String, scale: DemoScale): List<Entry> =
        if (scale == DemoScale.SAMPLE || ringId != RING_HOME_NAS) entriesFor(ringId)
        else Perf.time("demo.generate ${scale.label}") { generated(ringId, scale.count) }

    fun entriesFor(ringId: String): List<Entry> {
        val nowNs = System.currentTimeMillis() * 1_000_000L
        var nextId = 10_000L
        fun e(
            path: String,
            sizeBytes: Long,
            mime: String,
            state: EntryState,
            hoursAgo: Long,
        ): Entry {
            val id = nextId++
            return Entry(
                id = id,
                ringId = ringId,
                ringPath = path,
                localPath = if (state == EntryState.REMOTE_ONLY) null else "/storage/emulated/0/$path",
                remoteFileId = if (state == EntryState.LOCAL_ONLY) null else id,
                size = sizeBytes.toULong(),
                mtimeNs = nowNs - hoursAgo * 3_600_000_000_000L,
                mime = mime,
                blake3 = if (state == EntryState.LOCAL_ONLY) null else "demo$id",
                thumb = null,
                state = state,
                suppressed = false,
                uploadPending = state == EntryState.LOCAL_ONLY,
                lastError = null,
            )
        }

        return when (ringId) {
            RING_HOME_NAS -> listOf(
                // pixel-9 / Camera
                e("pixel-9/Camera/IMG_20261124_184210.jpg", 4_820_000, "image/jpeg", EntryState.SYNCED, 1),
                e("pixel-9/Camera/IMG_20261124_183904.jpg", 5_140_000, "image/jpeg", EntryState.SYNCED, 2),
                e("pixel-9/Camera/IMG_20261124_171255.heic", 3_610_000, "image/heic", EntryState.SYNCED, 3),
                e("pixel-9/Camera/VID_20261123_201540.mp4", 184_500_000, "video/mp4", EntryState.REMOTE_ONLY, 14),
                e("pixel-9/Camera/VID_20261122_142018.mp4", 312_000_000, "video/mp4", EntryState.REMOTE_ONLY, 36),
                e("pixel-9/Camera/IMG_20261125_073112.jpg", 6_240_000, "image/jpeg", EntryState.LOCAL_ONLY, 0),
                e("pixel-9/Camera/IMG_20261125_073205.jpg", 5_890_000, "image/jpeg", EntryState.LOCAL_ONLY, 0),
                // pixel-9 / Screenshots
                e("pixel-9/Screenshots/Screenshot_20261124-2110.png", 640_000, "image/png", EntryState.SYNCED, 5),
                e("pixel-9/Screenshots/Screenshot_20261124-1645.png", 512_000, "image/png", EntryState.SYNCED, 9),
                e("pixel-9/Screenshots/Screenshot_20261123-0912.png", 890_000, "image/png", EntryState.REMOTE_ONLY, 28),
                // pixel-9 / Documents
                e("pixel-9/Documents/architecture-notes.md", 24_500, "text/markdown", EntryState.SYNCED, 4),
                e("pixel-9/Documents/home-network-map.pdf", 2_450_000, "application/pdf", EntryState.SYNCED, 18),
                e("pixel-9/Documents/packing-checklist.txt", 4_200, "text/plain", EntryState.SYNCED, 40),
                e("pixel-9/Documents/warranty-monitor.pdf", 1_180_000, "application/pdf", EntryState.REMOTE_ONLY, 72),
                // pixel-9 / Recordings
                e("pixel-9/Recordings/voice-note-014.m4a", 3_840_000, "audio/mp4", EntryState.SYNCED, 6),
                e("pixel-9/Recordings/interview-draft.flac", 42_100_000, "audio/flac", EntryState.REMOTE_ONLY, 48),
                // tablet
                e("tablet/Books/designing-data-systems.epub", 8_400_000, "application/epub+zip", EntryState.REMOTE_ONLY, 50),
                e("tablet/Books/rust-nomicon.pdf", 4_920_000, "application/pdf", EntryState.REMOTE_ONLY, 65),
                e("tablet/Sketches/floorplan-v3.png", 2_150_000, "image/png", EntryState.REMOTE_ONLY, 20),
                // shared
                e("shared/Trips/alps-2026/panorama-ridge.jpg", 12_400_000, "image/jpeg", EntryState.REMOTE_ONLY, 120),
                e("shared/Trips/alps-2026/timelapse-4k.mp4", 640_000_000, "video/mp4", EntryState.REMOTE_ONLY, 122),
                e("shared/Trips/alps-2026/route.gpx", 185_000, "application/gpx+xml", EntryState.REMOTE_ONLY, 125),
                e("shared/Manuals/coffee-grinder.pdf", 3_120_000, "application/pdf", EntryState.REMOTE_ONLY, 200),
                e("shared/Manuals/router-setup.pdf", 1_940_000, "application/pdf", EntryState.REMOTE_ONLY, 240),
                // music
                e("music/ambient-sessions/01-drift.flac", 38_500_000, "audio/flac", EntryState.REMOTE_ONLY, 300),
                e("music/ambient-sessions/02-night-train.flac", 44_200_000, "audio/flac", EntryState.REMOTE_ONLY, 300),
                // projects
                e("projects/3d-prints/cable-clip-v2.stl", 1_420_000, "model/stl", EntryState.REMOTE_ONLY, 16),
                e("projects/3d-prints/desk-hook.3mf", 920_000, "application/octet-stream", EntryState.REMOTE_ONLY, 22),
            )

            RING_WORKSTATION -> listOf(
                e("pixel-9/Documents/release-plan.md", 18_200, "text/markdown", EntryState.SYNCED, 2),
                e("pixel-9/Documents/benchmark-results.csv", 142_000, "text/csv", EntryState.SYNCED, 5),
                e("pixel-9/Screenshots/ui-grid-check.png", 720_000, "image/png", EntryState.SYNCED, 3),
                e("assets/icons/app-mark.svg", 12_800, "image/svg+xml", EntryState.REMOTE_ONLY, 12),
                e("assets/banners/hero-light.png", 3_400_000, "image/png", EntryState.REMOTE_ONLY, 15),
                e("assets/banners/hero-dark.png", 3_180_000, "image/png", EntryState.REMOTE_ONLY, 15),
                e("builds/bringd-linux-x86_64.tar.zst", 14_800_000, "application/zstd", EntryState.REMOTE_ONLY, 8),
                e("notes/todo-2026.md", 6_400, "text/markdown", EntryState.REMOTE_ONLY, 1),
            )

            RING_ARCHIVE_VPS -> listOf(
                e("invoices/2026-11-hosting.pdf", 215_000, "application/pdf", EntryState.SYNCED, 10),
                e("invoices/2026-10-domain.pdf", 184_000, "application/pdf", EntryState.SYNCED, 420),
                e("backups/2026-11/configs.tar.zst", 28_400_000, "application/zstd", EntryState.REMOTE_ONLY, 24),
                e("backups/2026-10/configs.tar.zst", 27_900_000, "application/zstd", EntryState.REMOTE_ONLY, 740),
                e("photos-archive/2025/summer-album.zip", 1_420_000_000, "application/zip", EntryState.REMOTE_ONLY, 900),
            )

            else -> emptyList()
        }
    }

    fun rounds(nowMs: Long = System.currentTimeMillis()): List<SyncRound> = listOf(
        SyncRound(
            endedAt = nowMs - 4 * 60_000L,
            durationMs = 3_400L,
            manual = true,
            route = "LAN",
            uploaded = 6,
            uploadedBytes = 34_800_000L,
            received = 0,
            found = 6,
            offloaded = 1,
            failed = 0,
            remaining = 0,
            error = null,
            files = listOf(
                "pixel-9/Camera/IMG_20261124_184210.jpg",
                "pixel-9/Camera/IMG_20261124_183904.jpg",
                "pixel-9/Camera/IMG_20261124_171255.heic",
                "pixel-9/Screenshots/Screenshot_20261124-2110.png",
                "pixel-9/Documents/architecture-notes.md",
                "pixel-9/Recordings/voice-note-014.m4a",
            ),
            ringName = "home-nas",
        ),
        SyncRound(
            endedAt = nowMs - 28 * 60_000L,
            durationMs = 900L,
            manual = false,
            route = "LAN",
            uploaded = 0,
            uploadedBytes = 0L,
            received = 3,
            found = 0,
            offloaded = 0,
            failed = 0,
            remaining = 0,
            error = null,
            files = emptyList(),
            ringName = "workstation",
        ),
        SyncRound(
            endedAt = nowMs - 2 * 3_600_000L,
            durationMs = 11_200L,
            manual = false,
            route = "LAN",
            uploaded = 14,
            uploadedBytes = 198_400_000L,
            received = 1,
            found = 14,
            offloaded = 2,
            failed = 0,
            remaining = 0,
            error = null,
            files = listOf(
                "pixel-9/Camera/VID_20261123_201540.mp4",
                "pixel-9/Screenshots/Screenshot_20261124-1645.png",
            ),
            ringName = "home-nas",
        ),
        SyncRound(
            endedAt = nowMs - 5 * 3_600_000L,
            durationMs = 1_600L,
            manual = true,
            route = "WAN",
            uploaded = 2,
            uploadedBytes = 412_000L,
            received = 0,
            found = 2,
            offloaded = 0,
            failed = 0,
            remaining = 0,
            error = null,
            files = listOf("invoices/2026-11-hosting.pdf", "invoices/2026-10-domain.pdf"),
            ringName = "archive-vps",
        ),
        SyncRound(
            endedAt = nowMs - 9 * 3_600_000L,
            durationMs = 420L,
            manual = false,
            route = "LAN",
            uploaded = 1,
            uploadedBytes = 2_450_000L,
            received = 0,
            found = 1,
            offloaded = 0,
            failed = 0,
            remaining = 0,
            error = null,
            files = listOf("pixel-9/Documents/home-network-map.pdf"),
            ringName = "workstation",
        ),
    )
}
