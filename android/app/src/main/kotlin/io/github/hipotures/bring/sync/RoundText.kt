package io.github.hipotures.bring.sync

import io.github.hipotures.bring.formatBytes

/** "Uploaded 12 files (34.0 MiB)", "Sync failed", "Nothing new". */
fun roundTitle(r: SyncRound): String = when {
    r.error != null -> "Sync failed"
    r.uploaded > 0 -> "Uploaded ${files(r.uploaded)}" + if (r.uploadedBytes > 0) " (${formatBytes(r.uploadedBytes)})" else ""
    r.received > 0 -> "${files(r.received)} new on the Ring"
    r.found > 0 -> "Found ${files(r.found)}"
    else -> "Nothing new"
}

/** The rest of the round, one short line; the error if it failed. */
fun roundDetail(r: SyncRound): String = r.error ?: buildList {
    if (r.uploaded > 0 && r.received > 0) add("${r.received} new on the Ring")
    if (r.offloaded > 0) add("${r.offloaded} offloaded")
    if (r.failed > 0) add("${r.failed} failed")
    if (r.remaining > 0) add("${r.remaining} still queued")
}.joinToString(" · ").ifEmpty { "Everything is in sync" }

private fun files(n: Int) = if (n == 1) "1 file" else "$n files"
