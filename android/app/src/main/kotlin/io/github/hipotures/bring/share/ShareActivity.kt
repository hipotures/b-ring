package io.github.hipotures.bring.share

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.provider.OpenableColumns
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.core.content.IntentCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.lifecycleScope
import io.github.hipotures.bring.bring
import io.github.hipotures.bring.describe
import io.github.hipotures.bring.ffi.RingInfo
import io.github.hipotures.bring.sync.SyncScheduler
import io.github.hipotures.bring.ui.BringTheme
import io.github.hipotures.bring.ui.SharpButton
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import themechanics.compose.Tm
import themechanics.compose.TmDimens
import themechanics.compose.tmHardShadow
import java.io.File
import java.util.UUID

/**
 * Share Sheet target (§4): copies each shared item into app storage and
 * queues it for `Inbox/` on the chosen Ring, then starts a sync. With one Ring
 * paired it runs with no UI; with multiple Rings it asks which Ring to send to.
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
            when {
                rings.isEmpty() -> {
                    Toast.makeText(this@ShareActivity, "Pair B-Ring with a Ring first", Toast.LENGTH_LONG).show()
                    finish()
                }
                rings.size == 1 -> sendTo(uris, rings.first())
                else -> showPicker(uris, rings)
            }
        }
    }

    private fun showPicker(uris: List<Uri>, rings: List<RingInfo>) {
        setContent {
            val theme by applicationContext.bring.prefs.theme.collectAsStateWithLifecycle()
            BringTheme(theme) {
                AlertDialog(
                    onDismissRequest = { finish() },
                    modifier = Modifier
                        .padding(end = TmDimens.shadowOverlay, bottom = TmDimens.shadowOverlay)
                        .tmHardShadow(Tm.colors.shadowOverlay, TmDimens.shadowOverlay)
                        .border(TmDimens.bw1, Tm.colors.line1, MaterialTheme.shapes.extraLarge),
                    containerColor = Tm.colors.surface,
                    tonalElevation = 0.dp,
                    title = {
                        val n = uris.size
                        Text("Send $n ${if (n == 1) "file" else "files"} to Ring")
                    },
                    text = {
                        Column(verticalArrangement = Arrangement.spacedBy(TmDimens.space2)) {
                            rings.forEach { r ->
                                SharpButton(
                                    r.name,
                                    onClick = { sendTo(uris, r) },
                                    modifier = Modifier.fillMaxWidth(),
                                )
                            }
                        }
                    },
                    confirmButton = {},
                    dismissButton = { SharpButton("Cancel", onClick = { finish() }) },
                )
            }
        }
    }

    private fun sendTo(uris: List<Uri>, ring: RingInfo) {
        lifecycleScope.launch {
            val text = withContext(Dispatchers.IO) { importAll(uris, ring) }
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

    private fun importAll(uris: List<Uri>, ring: RingInfo): String {
        val bring = applicationContext.bring
        val inbox = getExternalFilesDir("Inbox") ?: File(filesDir, "Inbox")
        var queued = 0
        var lastError: String? = null
        for (uri in uris) {
            try {
                val name = sanitizeFileName(displayName(uri), "shared-${System.currentTimeMillis()}")
                // A directory per item: two shares of "image.jpg" never clash.
                val dir = File(inbox, UUID.randomUUID().toString().take(8)).apply { mkdirs() }
                val dest = File(dir, name)
                contentResolver.openInputStream(uri)?.use { input ->
                    dest.outputStream().use { input.copyTo(it) }
                } ?: error("cannot read $uri")
                bring.moon.enqueueUploadFor(ring.ringId, dest.path, "Inbox/$name")
                queued++
            } catch (e: Exception) {
                lastError = e.describe()
            }
        }
        if (queued > 0) SyncScheduler.syncNow(applicationContext, queueAfter = true, ringId = ring.ringId)
        return when {
            lastError == null -> "Queued $queued file(s) for ${ring.name} Inbox"
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
