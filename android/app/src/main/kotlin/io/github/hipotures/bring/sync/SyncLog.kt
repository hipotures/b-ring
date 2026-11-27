package io.github.hipotures.bring.sync

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.io.File

/** One finished sync round, as shown in the activity list. */
data class SyncRound(
    /** Wall-clock end, epoch milliseconds. */
    val endedAt: Long,
    val durationMs: Long,
    val manual: Boolean,
    /** "LAN", "WAN" or "" when the Ring was not reached. */
    val route: String,
    val uploaded: Int,
    val uploadedBytes: Long,
    /** Changes received from the Ring's index (new or changed files there). */
    val received: Int,
    /** New local files found by the scan. */
    val found: Int,
    val offloaded: Int,
    val failed: Int,
    /** Still queued for upload after the round. */
    val remaining: Int,
    val error: String?,
    /** The first [MAX_FILES] Ring paths uploaded in this round, in upload order. */
    val files: List<String>,
    /** Custom name of the Ring synced in this round (empty in older log entries). */
    val ringName: String = "",
) {
    /** Anything worth showing besides "checked, nothing new". */
    val changed: Boolean
        get() = uploaded > 0 || received > 0 || found > 0 || offloaded > 0 || failed > 0 || error != null

    companion object {
        const val MAX_FILES = 3
        private const val SEP = '\t'
        private const val LIST_SEP = '\u001f'

        private fun clean(s: String) = s.replace('\t', ' ').replace('\n', ' ').replace('\r', ' ').replace(LIST_SEP, ' ')

        fun encode(r: SyncRound): String = listOf(
            r.endedAt, r.durationMs, if (r.manual) 1 else 0, clean(r.route),
            r.uploaded, r.uploadedBytes, r.received, r.found, r.offloaded, r.failed, r.remaining,
            clean(r.error ?: ""), r.files.joinToString(LIST_SEP.toString(), transform = ::clean),
            clean(r.ringName),
        ).joinToString(SEP.toString())

        fun decode(line: String): SyncRound? {
            val f = line.split(SEP)
            if (f.size != 13 && f.size != 14) return null
            return runCatching {
                SyncRound(
                    endedAt = f[0].toLong(),
                    durationMs = f[1].toLong(),
                    manual = f[2] == "1",
                    route = f[3],
                    uploaded = f[4].toInt(),
                    uploadedBytes = f[5].toLong(),
                    received = f[6].toInt(),
                    found = f[7].toInt(),
                    offloaded = f[8].toInt(),
                    failed = f[9].toInt(),
                    remaining = f[10].toInt(),
                    error = f[11].ifEmpty { null },
                    files = if (f[12].isEmpty()) emptyList() else f[12].split(LIST_SEP),
                    ringName = f.getOrNull(13).orEmpty(),
                )
            }.getOrNull()
        }
    }
}

/**
 * The last [keep] rounds, newest first, persisted as one line per round in
 * [file]. Rounds that found nothing are kept only when started by hand, so
 * the 15-minute background checks do not push real activity out.
 */
class SyncLog(private val file: File, private val keep: Int = 500) {
    private val _rounds = MutableStateFlow(read())
    val rounds: StateFlow<List<SyncRound>> = _rounds.asStateFlow()

    private val checkedFile = File(file.path + ".checked")

    /** End of the last round, idle ones included (epoch ms, 0 = never). */
    private val _lastChecked = MutableStateFlow(
        maxOf(
            runCatching { checkedFile.readText().trim().toLong() }.getOrDefault(0L),
            _rounds.value.firstOrNull()?.endedAt ?: 0L,
        ),
    )
    val lastChecked: StateFlow<Long> = _lastChecked.asStateFlow()

    @Synchronized
    fun add(round: SyncRound) {
        _lastChecked.value = round.endedAt
        runCatching { checkedFile.writeText(round.endedAt.toString()) }
        if (!round.changed && !round.manual) return
        val next = (listOf(round) + _rounds.value).take(keep)
        _rounds.value = next
        runCatching {
            file.parentFile?.mkdirs()
            val tmp = File(file.path + ".tmp")
            tmp.writeText(next.joinToString("\n", transform = SyncRound::encode))
            tmp.renameTo(file)
        }
    }

    private fun read(): List<SyncRound> =
        runCatching { file.readLines().mapNotNull(SyncRound::decode) }.getOrDefault(emptyList())
}
