package io.github.hipotures.bring.share

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.provider.OpenableColumns
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.KeyboardArrowUp
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.core.content.IntentCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.lifecycleScope
import io.github.hipotures.bring.bring
import io.github.hipotures.bring.browser.crumbs
import io.github.hipotures.bring.browser.parentOf
import io.github.hipotures.bring.describe
import io.github.hipotures.bring.ffi.RingInfo
import io.github.hipotures.bring.sync.SyncScheduler
import io.github.hipotures.bring.ui.ActionIcon
import io.github.hipotures.bring.ui.BringTheme
import io.github.hipotures.bring.ui.FolderGlyph
import io.github.hipotures.bring.ui.RowDivider
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
 * Share Sheet target:
 * 1. If multiple Rings are paired, asks which Ring to send to.
 * 2. Opens a full-screen Ring folder browser (same UX as `FolderPicker`) to
 *    browse level-by-level or create a new subfolder, then queues a remote-only
 *    upload so no local copy stays in B-Ring after the Ring verifies the file.
 */
class ShareActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
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
            showShareFlow(uris, rings)
        }
    }

    private fun showShareFlow(uris: List<Uri>, rings: List<RingInfo>) {
        setContent {
            val theme by applicationContext.bring.prefs.theme.collectAsStateWithLifecycle()
            var chosenRing by remember {
                mutableStateOf(if (rings.size == 1) rings.first() else null)
            }
            var sending by remember { mutableStateOf(false) }

            BringTheme(theme) {
                val ring = chosenRing
                if (ring == null) {
                    RingChoiceDialog(
                        fileCount = uris.size,
                        rings = rings,
                        onPick = { chosenRing = it },
                        onDismiss = { finish() },
                    )
                } else {
                    RingFolderBrowser(
                        ring = ring,
                        fileCount = uris.size,
                        canChangeRing = rings.size > 1,
                        sending = sending,
                        loadFolders = { loadAllRingFolders(ring.ringId) },
                        onBackToRings = { chosenRing = null },
                        onClose = { finish() },
                        onSend = { targetFolder ->
                            sending = true
                            sendTo(uris, ring, targetFolder)
                        },
                    )
                }
            }
        }
    }

    private suspend fun loadAllRingFolders(ringId: String): Set<String> = withContext(Dispatchers.IO) {
        val moon = applicationContext.bring.moon
        val out = linkedSetOf<String>()
        runCatching { moon.watchedFoldersFor(ringId) }.getOrDefault(emptyList())
            .forEach { wf -> addFolderAndParents(out, wf.ringFolder) }
        runCatching { moon.entriesFor(ringId, null) }.getOrDefault(emptyList())
            .forEach { e ->
                val slash = e.ringPath.lastIndexOf('/')
                if (slash > 0) addFolderAndParents(out, e.ringPath.substring(0, slash))
            }
        out
    }

    private fun addFolderAndParents(out: MutableSet<String>, raw: String) {
        val clean = raw.trim('/')
        if (clean.isEmpty()) return
        var acc = ""
        for (part in clean.split('/')) {
            if (part.isEmpty()) continue
            acc = if (acc.isEmpty()) part else "$acc/$part"
            out += acc
        }
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

@Composable
private fun RingChoiceDialog(
    fileCount: Int,
    rings: List<RingInfo>,
    onPick: (RingInfo) -> Unit,
    onDismiss: () -> Unit,
) {
    val c = Tm.colors
    AlertDialog(
        onDismissRequest = onDismiss,
        modifier = Modifier
            .padding(end = TmDimens.shadowOverlay, bottom = TmDimens.shadowOverlay)
            .tmHardShadow(c.shadowOverlay, TmDimens.shadowOverlay)
            .border(TmDimens.bw1, c.line1, MaterialTheme.shapes.extraLarge),
        containerColor = c.surface,
        tonalElevation = 0.dp,
        title = { Text("Send $fileCount ${if (fileCount == 1) "file" else "files"} to Ring") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(TmDimens.space2)) {
                rings.forEach { r ->
                    SharpButton(
                        r.name,
                        onClick = { onPick(r) },
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
            }
        },
        confirmButton = {},
        dismissButton = { SharpButton("Cancel", onClick = onDismiss) },
    )
}

@Composable
private fun RingFolderBrowser(
    ring: RingInfo,
    fileCount: Int,
    canChangeRing: Boolean,
    sending: Boolean,
    loadFolders: suspend () -> Set<String>,
    onBackToRings: () -> Unit,
    onClose: () -> Unit,
    onSend: (String) -> Unit,
) {
    var dir by remember(ring.ringId) { mutableStateOf("") }
    var allFolders by remember(ring.ringId) { mutableStateOf<Set<String>?>(null) }
    var creating by remember { mutableStateOf(false) }
    var draftFolder by remember { mutableStateOf("") }

    LaunchedEffect(ring.ringId) {
        allFolders = null
        allFolders = loadFolders()
    }

    BackHandler(enabled = !sending) {
        when {
            dir.isNotEmpty() -> dir = parentOf(dir)
            canChangeRing -> onBackToRings()
            else -> onClose()
        }
    }

    val children = remember(allFolders, dir) {
        val set = allFolders ?: return@remember null
        val prefix = if (dir.isEmpty()) "" else "$dir/"
        set.asSequence()
            .filter { it.startsWith(prefix) }
            .map { it.substring(prefix.length) }
            .filter { it.isNotEmpty() && '/' !in it }
            .sortedWith(String.CASE_INSENSITIVE_ORDER)
            .toList()
    }

    Column(
        Modifier
            .fillMaxSize()
            .background(Tm.colors.chrome)
            .safeDrawingPadding()
            .background(Tm.colors.canvas),
    ) {
        Row(
            Modifier
                .fillMaxWidth()
                .background(Tm.colors.head1)
                .height(56.dp)
                .padding(start = if (canChangeRing) TmDimens.space1 else TmDimens.regionPad, end = TmDimens.space1),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (canChangeRing) {
                ActionIcon(Icons.AutoMirrored.Outlined.ArrowBack, "Choose Ring", onBackToRings, enabled = !sending)
            }
            Text(
                "Choose folder · ${ring.name}",
                style = MaterialTheme.typography.titleLarge,
                color = Tm.colors.text,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f),
            )
            ActionIcon(Icons.Outlined.Add, "New folder", {
                draftFolder = ""
                creating = true
            }, enabled = !sending)
            ActionIcon(Icons.Outlined.Close, "Close", onClose, enabled = !sending)
        }
        HorizontalDivider(thickness = TmDimens.bw1, color = Tm.colors.line1)

        // Breadcrumbs: ring / a / b, each tappable.
        Row(
            Modifier
                .fillMaxWidth()
                .horizontalScroll(rememberScrollState())
                .padding(horizontal = TmDimens.space1),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            ShareCrumb(ring.name, current = dir.isEmpty()) { dir = "" }
            crumbs(dir).forEach { (name, path) ->
                Text("/", style = Tm.text.code, color = Tm.colors.text3)
                ShareCrumb(name, current = path == dir) { dir = path }
            }
        }
        HorizontalDivider(thickness = TmDimens.bw2, color = Tm.colors.line2)

        LazyColumn(Modifier.weight(1f)) {
            if (dir.isNotEmpty()) {
                item {
                    ShareDirRow("Up one level", up = true) { dir = parentOf(dir) }
                    RowDivider()
                }
            }
            val list = children
            when {
                list == null -> item { ShareNote("Reading…") }
                list.isEmpty() -> item { ShareNote("No subfolders. Tap + to create one, or send here.") }
                else -> items(list, key = { it }) { name ->
                    val next = if (dir.isEmpty()) name else "$dir/$name"
                    ShareDirRow(name) { dir = next }
                    RowDivider()
                }
            }
        }

        HorizontalDivider(thickness = TmDimens.bw1, color = Tm.colors.line1)
        Column(
            Modifier
                .fillMaxWidth()
                .background(Tm.colors.chrome)
                .padding(horizontal = TmDimens.regionPad, vertical = TmDimens.space2),
            verticalArrangement = Arrangement.spacedBy(TmDimens.space1),
        ) {
            Text(
                if (dir.isEmpty()) "${ring.name}/ (open or create a folder)" else "${ring.name}/$dir · Remote only",
                style = Tm.text.code,
                color = Tm.colors.text2,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            SharpButton(
                when {
                    sending -> "Sending…"
                    dir.isEmpty() -> "Open or create a folder to send"
                    else -> "Send $fileCount ${if (fileCount == 1) "file" else "files"} here"
                },
                enabled = !sending && dir.isNotEmpty(),
                primary = true,
                onClick = { onSend(dir) },
                modifier = Modifier.fillMaxWidth(),
            )
        }
    }

    if (creating) {
        val c = Tm.colors
        val clean = sanitizeFolderPath(draftFolder, fallback = "")
        AlertDialog(
            onDismissRequest = { creating = false },
            modifier = Modifier
                .padding(end = TmDimens.shadowOverlay, bottom = TmDimens.shadowOverlay)
                .tmHardShadow(c.shadowOverlay, TmDimens.shadowOverlay)
                .border(TmDimens.bw1, c.line1, MaterialTheme.shapes.extraLarge),
            containerColor = c.surface,
            tonalElevation = 0.dp,
            title = { Text("New folder") },
            text = {
                OutlinedTextField(
                    value = draftFolder,
                    onValueChange = { draftFolder = it },
                    label = { Text(if (dir.isEmpty()) "Folder name" else "Subfolder in $dir") },
                    placeholder = { Text("e.g. Docs, Receipts") },
                    singleLine = true,
                    colors = tmFieldColors(),
                    modifier = Modifier.fillMaxWidth(),
                )
            },
            confirmButton = {
                SharpButton(
                    "Create",
                    primary = true,
                    enabled = clean.isNotEmpty(),
                    onClick = {
                        creating = false
                        val full = if (dir.isEmpty()) clean else "$dir/$clean"
                        val next = (allFolders ?: emptySet()).toMutableSet()
                        var acc = ""
                        for (part in full.split('/')) {
                            if (part.isEmpty()) continue
                            acc = if (acc.isEmpty()) part else "$acc/$part"
                            next += acc
                        }
                        allFolders = next
                        dir = full
                    },
                )
            },
            dismissButton = { SharpButton("Cancel", onClick = { creating = false }) },
        )
    }
}

@Composable
private fun ShareCrumb(text: String, current: Boolean, onClick: () -> Unit) {
    Text(
        text,
        style = MaterialTheme.typography.bodyMedium,
        fontWeight = if (current) FontWeight.SemiBold else FontWeight.Normal,
        color = if (current) Tm.colors.text else Tm.colors.link,
        modifier = Modifier
            .clickable(enabled = !current, onClick = onClick)
            .padding(vertical = 14.dp, horizontal = TmDimens.space2),
    )
}

@Composable
private fun ShareDirRow(name: String, up: Boolean = false, onClick: () -> Unit) {
    val c = Tm.colors
    Row(
        Modifier
            .fillMaxWidth()
            .background(c.surface)
            .clickable(onClick = onClick)
            .heightIn(min = 56.dp)
            .padding(horizontal = TmDimens.regionPad, vertical = TmDimens.space2),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            if (up) Icons.Outlined.KeyboardArrowUp else FolderGlyph,
            null,
            Modifier.size(TmDimens.iconLg),
            tint = c.text2,
        )
        Spacer(Modifier.width(TmDimens.space4))
        Text(
            name,
            style = MaterialTheme.typography.bodyMedium,
            fontWeight = FontWeight.Medium,
            color = c.text,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.weight(1f),
        )
        if (!up) Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, null, Modifier.size(TmDimens.iconLg), tint = c.text3)
    }
}

@Composable
private fun ShareNote(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.bodyMedium,
        color = Tm.colors.text2,
        modifier = Modifier.padding(TmDimens.regionPad),
    )
}
