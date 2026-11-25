package io.github.hipotures.bring.share

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.provider.OpenableColumns
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.core.content.IntentCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.lifecycleScope
import io.github.hipotures.bring.bring
import io.github.hipotures.bring.describe
import io.github.hipotures.bring.ffi.RingInfo
import io.github.hipotures.bring.sync.SyncScheduler
import io.github.hipotures.bring.ui.BringTheme
import io.github.hipotures.bring.ui.ControlShape
import io.github.hipotures.bring.ui.SharpButton
import io.github.hipotures.bring.ui.tmFieldColors
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import themechanics.compose.Tm
import themechanics.compose.TmDimens
import themechanics.compose.tmHardShadow
import java.io.File
import java.util.UUID

/**
 * Share Sheet target: lets the user pick a Ring and destination folder
 * (existing or custom), stages each shared item in app-private storage, and
 * queues a remote-only upload so the staged local copy is deleted as soon as
 * the Ring verifies the file.
 */
class ShareActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val uris = sharedUris(intent)
        if (uris.isEmpty()) {
            Toast.makeText(this, "Nothing to share", Toast.LENGTH_LONG).show()
            finish()
            return
        }
        lifecycleScope.launch {
            val rings = withContext(Dispatchers.IO) {
                val b = applicationContext.bring
                runCatching { b.moon.rings() }.getOrDefault(emptyList())
                    .filter { b.tokens.load(it.ringId) != null }
            }
            if (rings.isEmpty()) {
                Toast.makeText(this@ShareActivity, "Pair B-Ring with a Ring first", Toast.LENGTH_LONG).show()
                finish()
                return@launch
            }
            val initialRing = rings.firstOrNull { it.ringId == applicationContext.bring.prefs.selectedRingId }
                ?: rings.first()
            showPicker(uris, rings, initialRing)
        }
    }

    private fun showPicker(uris: List<Uri>, rings: List<RingInfo>, initialRing: RingInfo) {
        setContent {
            val theme by applicationContext.bring.prefs.theme.collectAsStateWithLifecycle()
            var selectedRing by remember { mutableStateOf(initialRing) }
            var folder by remember {
                mutableStateOf(applicationContext.bring.prefs.lastShareFolder.orEmpty())
            }
            var suggestions by remember { mutableStateOf<List<String>>(emptyList()) }
            var sending by remember { mutableStateOf(false) }

            LaunchedEffect(selectedRing.ringId) {
                suggestions = withContext(Dispatchers.IO) { knownFolders(selectedRing.ringId) }
            }

            BringTheme(theme) {
                val c = Tm.colors
                AlertDialog(
                    onDismissRequest = { if (!sending) finish() },
                    modifier = Modifier
                        .padding(end = TmDimens.shadowOverlay, bottom = TmDimens.shadowOverlay)
                        .tmHardShadow(c.shadowOverlay, TmDimens.shadowOverlay)
                        .border(TmDimens.bw1, c.line1, MaterialTheme.shapes.extraLarge),
                    containerColor = c.surface,
                    tonalElevation = 0.dp,
                    title = {
                        val n = uris.size
                        Text("Send $n ${if (n == 1) "file" else "files"} to Ring")
                    },
                    text = {
                        Column(
                            Modifier.verticalScroll(rememberScrollState()),
                            verticalArrangement = Arrangement.spacedBy(TmDimens.space3),
                        ) {
                            if (rings.size > 1) {
                                Text("RING", style = Tm.text.eyebrow, color = c.text2)
                                FlowRow(
                                    horizontalArrangement = Arrangement.spacedBy(TmDimens.space2),
                                    verticalArrangement = Arrangement.spacedBy(TmDimens.space2),
                                ) {
                                    rings.forEach { r ->
                                        val active = r.ringId == selectedRing.ringId
                                        Row(
                                            Modifier
                                                .border(
                                                    if (active) TmDimens.bwControl else TmDimens.bw2,
                                                    if (active) c.ink else c.line2,
                                                    ControlShape,
                                                )
                                                .background(if (active) c.selectBg else c.surface, ControlShape)
                                                .clickable(enabled = !sending) { selectedRing = r }
                                                .heightIn(min = TmDimens.controlSm)
                                                .padding(horizontal = TmDimens.controlPadXMd, vertical = TmDimens.space1),
                                            verticalAlignment = Alignment.CenterVertically,
                                        ) {
                                            Text(
                                                r.name,
                                                style = MaterialTheme.typography.labelLarge,
                                                color = c.text,
                                                maxLines = 1,
                                                overflow = TextOverflow.Ellipsis,
                                            )
                                        }
                                    }
                                }
                            }

                            OutlinedTextField(
                                value = folder,
                                onValueChange = { folder = it },
                                enabled = !sending,
                                label = { Text("Folder on ${selectedRing.name}") },
                                placeholder = { Text("Pick below or type a folder path") },
                                singleLine = true,
                                colors = tmFieldColors(),
                                modifier = Modifier.fillMaxWidth(),
                            )

                            if (suggestions.isNotEmpty()) {
                                Text("EXISTING FOLDERS", style = Tm.text.eyebrow, color = c.text2)
                                FlowRow(
                                    horizontalArrangement = Arrangement.spacedBy(TmDimens.space1_5),
                                    verticalArrangement = Arrangement.spacedBy(TmDimens.space1_5),
                                ) {
                                    suggestions.forEach { f ->
                                        val active = sanitizeFolderPath(folder) == f
                                        Row(
                                            Modifier
                                                .border(
                                                    if (active) TmDimens.bwControl else TmDimens.bw2,
                                                    if (active) c.ink else c.line2,
                                                    ControlShape,
                                                )
                                                .background(if (active) c.selectBg else c.surface, ControlShape)
                                                .clickable(enabled = !sending) { folder = f }
                                                .heightIn(min = 32.dp)
                                                .padding(horizontal = TmDimens.space2, vertical = TmDimens.space1),
                                            verticalAlignment = Alignment.CenterVertically,
                                        ) {
                                            Text(
                                                f,
                                                style = Tm.text.code,
                                                color = c.text,
                                                maxLines = 1,
                                                overflow = TextOverflow.Ellipsis,
                                            )
                                        }
                                    }
                                }
                            }

                            Text(
                                "Stored on the Ring as Remote — no local copy kept in B-Ring after upload.",
                                style = MaterialTheme.typography.bodySmall,
                                color = c.text2,
                            )
                        }
                    },
                            confirmButton = {
                        val cleanFolder = sanitizeFolderPath(folder, fallback = "")
                        SharpButton(
                            if (sending) "Sending…" else "Send",
                            primary = true,
                            enabled = !sending && cleanFolder.isNotEmpty(),
                            onClick = {
                                sending = true
                                applicationContext.bring.prefs.lastShareFolder = cleanFolder
                                sendTo(uris, selectedRing, cleanFolder)
                            },
                        )
                    },
                    dismissButton = {
                        SharpButton("Cancel", enabled = !sending, onClick = { finish() })
                    },
                )
            }
        }
    }

    private fun knownFolders(ringId: String): List<String> {
        val moon = applicationContext.bring.moon
        val out = sortedSetOf<String>(String.CASE_INSENSITIVE_ORDER)
        runCatching { moon.watchedFoldersFor(ringId) }.getOrDefault(emptyList())
            .forEach { wf ->
                val clean = wf.ringFolder.trim('/')
                if (clean.isNotEmpty()) out += clean
            }
        runCatching { moon.entriesFor(ringId, null) }.getOrDefault(emptyList())
            .forEach { e ->
                val slash = e.ringPath.lastIndexOf('/')
                if (slash > 0) {
                    val dir = e.ringPath.substring(0, slash)
                    out += dir
                    val top = dir.substringBefore('/')
                    if (top.isNotEmpty()) out += top
                }
            }
        return out.take(12)
    }

    private fun sendTo(uris: List<Uri>, ring: RingInfo, targetFolder: String) {
        lifecycleScope.launch {
            val text = withContext(Dispatchers.IO) { importAll(uris, ring, targetFolder) }
            Toast.makeText(this@ShareActivity, text, Toast.LENGTH_LONG).show()
            finish()
        }
    }

    private fun sharedUris(intent: Intent): List<Uri> = when (intent.action) {
        Intent.ACTION_SEND ->
            listOfNotNull(IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java))
        Intent.ACTION_SEND_MULTIPLE ->
            IntentCompat.getParcelableArrayListExtra(intent, Intent.EXTRA_STREAM, Uri::class.java).orEmpty()
        else -> emptyList()
    }

    private fun importAll(uris: List<Uri>, ring: RingInfo, targetFolder: String): String {
        val bring = applicationContext.bring
        val stagingRoot = getExternalFilesDir("Inbox") ?: File(filesDir, "Inbox")
        var queued = 0
        var lastError: String? = null
        for (uri in uris) {
            try {
                val name = sanitizeFileName(displayName(uri), "shared-${System.currentTimeMillis()}")
                // A directory per item: two shares of "image.jpg" never clash.
                val dir = File(stagingRoot, UUID.randomUUID().toString().take(8)).apply { mkdirs() }
                val dest = File(dir, name)
                contentResolver.openInputStream(uri)?.use { input ->
                    dest.outputStream().use { input.copyTo(it) }
                } ?: error("cannot read $uri")
                bring.moon.enqueueUploadRemoteFor(ring.ringId, dest.path, "$targetFolder/$name")
                queued++
            } catch (e: Exception) {
                lastError = e.describe()
            }
        }
        if (queued > 0) SyncScheduler.syncNow(applicationContext, queueAfter = true, ringId = ring.ringId)
        return when {
            lastError == null -> "Queued $queued file(s) for ${ring.name}: $targetFolder"
            queued == 0 -> "Share failed: $lastError"
            else -> "Queued $queued of ${uris.size} for ${ring.name}; last error: $lastError"
        }
    }

    private fun displayName(uri: Uri): String? =
        runCatching {
            contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
                if (c.moveToFirst()) c.getString(0) else null
            }
        }.getOrNull() ?: uri.lastPathSegment
}
