package io.github.hipotures.bring.share

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.provider.OpenableColumns
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.core.content.IntentCompat
import androidx.lifecycle.lifecycleScope
import io.github.hipotures.bring.bring
import io.github.hipotures.bring.describe
import io.github.hipotures.bring.sync.SyncScheduler
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.util.UUID

/**
 * Share Sheet target (§4): copies each shared item into app storage and
 * queues it for `Inbox/` on the Ring, then starts a sync. No UI of its own.
 */
class ShareActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val uris = sharedUris(intent)
        lifecycleScope.launch {
            val text = withContext(Dispatchers.IO) { importAll(uris) }
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

    private fun importAll(uris: List<Uri>): String {
        if (uris.isEmpty()) return "Nothing to share"
        val bring = applicationContext.bring
        if (bring.tokens.load() == null) return "Pair B-Ring with a Ring first"
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
                bring.moon.enqueueUpload(dest.path, "Inbox/$name")
                queued++
            } catch (e: Exception) {
                lastError = e.describe()
            }
        }
        if (queued > 0) SyncScheduler.syncNow(applicationContext, queueAfter = true)
        return when {
            lastError == null -> "Queued $queued file(s) for Inbox"
            queued == 0 -> "Share failed: $lastError"
            else -> "Queued $queued of ${uris.size}; last error: $lastError"
        }
    }

    private fun displayName(uri: Uri): String? =
        runCatching {
            contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
                if (c.moveToFirst()) c.getString(0) else null
            }
        }.getOrNull() ?: uri.lastPathSegment
}
