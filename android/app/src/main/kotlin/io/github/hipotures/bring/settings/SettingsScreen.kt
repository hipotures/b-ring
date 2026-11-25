package io.github.hipotures.bring.settings

import android.Manifest
import android.annotation.SuppressLint
import android.content.ActivityNotFoundException
import android.content.pm.PackageManager
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.Refresh
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.hipotures.bring.AppViewModel
import io.github.hipotures.bring.bring
import io.github.hipotures.bring.ffi.RingInfo
import io.github.hipotures.bring.formatBytes
import io.github.hipotures.bring.storage.StorageAccess
import io.github.hipotures.bring.ui.ActionIcon
import io.github.hipotures.bring.ui.ErrorLine
import io.github.hipotures.bring.ui.FolderGlyph
import io.github.hipotures.bring.ui.Group
import io.github.hipotures.bring.ui.InfoLine
import io.github.hipotures.bring.ui.KeyValue
import io.github.hipotures.bring.ui.Panel
import io.github.hipotures.bring.ui.PlateMark
import io.github.hipotures.bring.ui.RowDivider
import io.github.hipotures.bring.ui.Segmented
import io.github.hipotures.bring.ui.SharpButton
import io.github.hipotures.bring.ui.StatusPlate
import io.github.hipotures.bring.ui.Tag
import io.github.hipotures.bring.ui.ThemeChoice
import io.github.hipotures.bring.ui.tmFieldColors
import io.github.hipotures.bring.versionLine
import themechanics.compose.Tm
import themechanics.compose.TmDimens
import themechanics.compose.TmTone
import themechanics.compose.tmHardShadow

private val MIB: ULong = 1024UL * 1024UL

/** Offload choices; ULong.MAX_VALUE means "never delete local copies". */
private val OFFLOAD_CHOICES = listOf(
    "25 MiB" to 25UL * MIB,
    "100 MiB" to 100UL * MIB,
    "500 MiB" to 500UL * MIB,
    "2 GiB" to 2048UL * MIB,
    "Never" to ULong.MAX_VALUE,
)

/** Global app settings (apply across all Rings): Permissions, Appearance, version. */
@Composable
fun AppSettingsScreen(vm: AppViewModel) {
    val context = LocalContext.current
    val storage by vm.storageGranted.collectAsStateWithLifecycle()
    val theme by context.bring.prefs.theme.collectAsStateWithLifecycle()
    val demoMode by context.bring.prefs.demoMode.collectAsStateWithLifecycle()

    Column(
        Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .imePadding()
            .padding(TmDimens.panelPad),
        verticalArrangement = Arrangement.spacedBy(TmDimens.gapPanel),
    ) {
        PermissionsPanel(storage)

        Panel(title = "Appearance") {
            Segmented(ThemeChoice.entries, theme, vm::setTheme, label = { it.label })
            Text(
                "System follows the phone's light or dark mode. Coffee is a warm dark theme.",
                style = MaterialTheme.typography.bodySmall,
                color = Tm.colors.text2,
            )
        }

        Panel(title = "Demo mode") {
            Segmented(
                listOf(false, true),
                demoMode,
                vm::setDemoMode,
                label = { if (it) "On" else "Off" },
            )
            Text(
                "Shows sample Rings, folders and sync activity for screenshots. Your real data is untouched.",
                style = MaterialTheme.typography.bodySmall,
                color = Tm.colors.text2,
            )
        }

        Text(versionLine(context), style = Tm.text.code, color = Tm.colors.text2)
    }
}

/** Settings specific to a single paired Ring. */
@Composable
fun RingSettingsScreen(
    rings: List<RingInfo>,
    ring: RingInfo,
    vm: AppViewModel,
    onOpenPermissions: () -> Unit,
) {
    val context = LocalContext.current
    val sync by vm.sync.collectAsStateWithLifecycle()
    val verify by vm.verify.collectAsStateWithLifecycle()
    val entries by vm.entries.collectAsStateWithLifecycle()
    val storage by vm.storageGranted.collectAsStateWithLifecycle()
    val folders by vm.folders.collectAsStateWithLifecycle()
    val message by vm.message.collectAsStateWithLifecycle()
    val defaultOffload = context.bring.prefs.offloadThresholdBytes ?: (100UL * MIB)
    var offload by remember(ring.ringId, ring.offloadThresholdBytes) {
        mutableStateOf(ring.offloadThresholdBytes ?: defaultOffload)
    }
    var picking by rememberSaveable { mutableStateOf(false) }
    var renaming by remember { mutableStateOf(false) }
    var draftName by remember(ring.ringId, ring.name) { mutableStateOf(ring.name) }
    var renamingDevice by remember { mutableStateOf(false) }
    var draftDeviceName by remember(ring.ringId, ring.deviceSlug) { mutableStateOf(ring.deviceSlug) }
    var confirmForget by remember { mutableStateOf(false) }
    val multiRing = rings.size > 1
    DisposableEffect(Unit) { onDispose { vm.clearMessage() } }

    Column(
        Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .imePadding()
            .padding(TmDimens.panelPad),
        verticalArrangement = Arrangement.spacedBy(TmDimens.gapPanel),
    ) {
        message?.let { m ->
            if (m.isError) ErrorLine(m.text) else InfoLine(m.text)
        }

        Panel(
            title = "Overview",
            trailing = { ActionIcon(Icons.Outlined.Refresh, "Check connection", vm::recheckRoute) },
        ) {
            KeyValue("Name", ring.name, onEdit = {
                draftName = ring.name
                renaming = true
            })
            KeyValue("Device", ring.deviceSlug, onEdit = {
                draftDeviceName = ring.deviceSlug
                renamingDevice = true
            })
            KeyValue("Device ID", ring.deviceId.take(8))
            KeyValue(
                "Transferred",
                "${ring.transferredFiles} ${if (ring.transferredFiles == 1UL) "file" else "files"} · " +
                    formatBytes(ring.transferredBytes.toLong()),
            )
            KeyValue("Ring ID", ring.ringId.take(16))
            KeyValue("LAN", ring.lanUrl)
            KeyValue("WAN", ring.wanUrl ?: "— (LAN only)")
            KeyValue("TLS pin", ring.tlsCertSha256.chunked(4).take(8).joinToString(" ") + " …")
            Text(
                "LAN on unmetered Wi-Fi when the Ring answers within 1.5 s; otherwise WAN if configured. Background sync runs on LAN only.",
                style = MaterialTheme.typography.bodySmall,
                color = Tm.colors.text2,
            )
        }

        val queued = remember(entries) { entries.count { it.uploadPending } }
        Panel(
            title = "Sync & index",
            trailing = {
                when (val v = verify) {
                    AppViewModel.VerifyState.Idle -> if (queued > 0) {
                        StatusPlate("$queued queued", PlateMark.PENDING, small = true)
                    }
                    AppViewModel.VerifyState.Running -> StatusPlate("Checking…", PlateMark.ACTIVE, small = true)
                    is AppViewModel.VerifyState.Done -> StatusPlate(
                        if (v.warning) "Attention" else "Verified",
                        if (v.warning) PlateMark.WARNING else PlateMark.COMPLETE,
                        small = true,
                    )
                    is AppViewModel.VerifyState.Failed -> StatusPlate("Failed", PlateMark.ERROR, small = true)
                }
            },
        ) {
            Text(
                "Re-reads ${ring.name}'s index from scratch, asks it to regenerate missing thumbnails, " +
                    "rescans watched folders and retries queued uploads.",
                style = MaterialTheme.typography.bodyMedium,
                color = Tm.colors.text2,
            )
            when (val v = verify) {
                is AppViewModel.VerifyState.Done -> Text(v.summary, style = Tm.text.code, color = Tm.colors.text)
                is AppViewModel.VerifyState.Failed -> ErrorLine(v.error)
                else -> {}
            }
            val checking = verify is AppViewModel.VerifyState.Running
            SharpButton(
                if (checking) "Checking…" else "Check sync & thumbnails",
                enabled = !sync.running && !checking,
                onClick = vm::verifySync,
            )
            if (sync.running) {
                Text(
                    "Wait for the current upload to finish.",
                    style = Tm.text.code,
                    color = Tm.colors.text2,
                )
            }
        }

        Panel(title = "Watched folders") {
            if (multiRing) {
                Text(
                    "Folders belong to ${ring.name}. Multiple Rings may watch the same folder.",
                    style = MaterialTheme.typography.bodySmall,
                    color = Tm.colors.text2,
                )
            }
            if (folders.isEmpty()) {
                Text("None yet.", style = MaterialTheme.typography.bodyMedium, color = Tm.colors.text2)
            }
            if (folders.isNotEmpty()) {
                Group {
                    folders.forEachIndexed { i, f ->
                        if (i > 0) RowDivider()
                        Row(
                            Modifier.fillMaxWidth().padding(start = TmDimens.cellPadX),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Icon(FolderGlyph, null, Modifier.size(TmDimens.iconLg), tint = Tm.colors.text2)
                            Column(
                                Modifier.weight(1f).padding(start = TmDimens.space3, top = TmDimens.space2, bottom = TmDimens.space2),
                            ) {
                                Text(
                                    f.localDir.removePrefix(StorageAccess.root().path + "/"),
                                    style = MaterialTheme.typography.bodyMedium,
                                    color = Tm.colors.text,
                                )
                                Text("→ ${ring.name}: ${f.ringFolder}", style = Tm.text.code, color = Tm.colors.text2)
                            }
                            ActionIcon(Icons.Outlined.Delete, "Stop watching", { vm.removeFolder(f.id) })
                        }
                    }
                }
            }
            val watched = folders.map { it.localDir }.toSet()
            val suggestions = remember(storage, watched) {
                if (storage) StorageAccess.suggestions().map { it.path }.filter { it !in watched } else emptyList()
            }
            if (suggestions.isNotEmpty()) {
                Text("QUICK ADD", style = Tm.text.eyebrow, color = Tm.colors.text2)
                FlowRow(horizontalArrangement = Arrangement.spacedBy(TmDimens.space2)) {
                    suggestions.forEach { path ->
                        SharpButton("+ ${path.substringAfterLast('/')}", onClick = { vm.addFolder(path) })
                    }
                }
            }
            SharpButton(
                "Choose folder…",
                enabled = storage,
                primary = true,
                onClick = { picking = true },
                modifier = Modifier.fillMaxWidth(),
            )
            if (!storage) {
                Text(
                    "All-files access is required to pick folders.",
                    style = Tm.text.code,
                    color = Tm.colors.text2,
                )
                SharpButton("Open app permissions", onClick = onOpenPermissions)
            }
        }

        Panel(title = "Offload") {
            Text(
                "After a verified upload, delete the local copy of files larger than:" +
                    if (multiRing) " (Shared folders wait until all watching Rings have verified the file.)" else "",
                style = MaterialTheme.typography.bodyMedium,
                color = Tm.colors.text2,
            )
            Segmented(
                options = OFFLOAD_CHOICES,
                selected = OFFLOAD_CHOICES.firstOrNull { it.second == offload } ?: OFFLOAD_CHOICES[1],
                onSelect = { (_, bytes) ->
                    offload = bytes
                    vm.setOffloadThreshold(bytes)
                },
                label = { it.first },
            )
        }

        Panel(title = "Pairing") {
            Text(
                "Forgetting ${ring.name} drops its index and watched folders on this device. " +
                    if (multiRing) "Other Rings and local files are not touched." else "Local files are not touched.",
                style = MaterialTheme.typography.bodyMedium,
                color = Tm.colors.text2,
            )
            SharpButton(
                if (multiRing) "Forget ${ring.name}" else "Forget Ring",
                danger = true,
                onClick = { confirmForget = true },
            )
        }
    }

    if (picking) {
        FolderPicker(
            watched = folders.map { it.localDir }.toSet(),
            onPick = {
                picking = false
                vm.addFolder(it)
            },
            onDismiss = { picking = false },
        )
    }

    if (renaming) {
        AlertDialog(
            onDismissRequest = { renaming = false },
            modifier = Modifier
                .padding(end = TmDimens.shadowOverlay, bottom = TmDimens.shadowOverlay)
                .tmHardShadow(Tm.colors.shadowOverlay, TmDimens.shadowOverlay)
                .border(TmDimens.bw1, Tm.colors.line1, MaterialTheme.shapes.extraLarge),
            containerColor = Tm.colors.surface,
            tonalElevation = 0.dp,
            title = { Text("Rename Ring") },
            text = {
                OutlinedTextField(
                    value = draftName,
                    onValueChange = { draftName = it },
                    label = { Text("Custom name") },
                    placeholder = { Text("e.g. Videos, Screenshots, NAS") },
                    singleLine = true,
                    colors = tmFieldColors(),
                    modifier = Modifier.fillMaxWidth(),
                )
            },
            confirmButton = {
                SharpButton("Save", primary = true, onClick = {
                    renaming = false
                    vm.renameRing(ring.ringId, draftName)
                })
            },
            dismissButton = { SharpButton("Cancel", onClick = { renaming = false }) },
        )
    }

    if (renamingDevice) {
        AlertDialog(
            onDismissRequest = { renamingDevice = false },
            modifier = Modifier
                .padding(end = TmDimens.shadowOverlay, bottom = TmDimens.shadowOverlay)
                .tmHardShadow(Tm.colors.shadowOverlay, TmDimens.shadowOverlay)
                .border(TmDimens.bw1, Tm.colors.line1, MaterialTheme.shapes.extraLarge),
            containerColor = Tm.colors.surface,
            tonalElevation = 0.dp,
            title = { Text("Rename this device") },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(TmDimens.space2)) {
                    OutlinedTextField(
                        value = draftDeviceName,
                        onValueChange = { draftDeviceName = it },
                        label = { Text("Device name on ${ring.name}") },
                        placeholder = { Text("e.g. S23 Work, Pixel 8") },
                        singleLine = true,
                        colors = tmFieldColors(),
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Text(
                        "Updates this phone's name and slug on ${ring.name}. Already-uploaded files stay where they are.",
                        style = MaterialTheme.typography.bodySmall,
                        color = Tm.colors.text2,
                    )
                }
            },
            confirmButton = {
                SharpButton(
                    "Save",
                    primary = true,
                    enabled = draftDeviceName.isNotBlank(),
                    onClick = {
                        renamingDevice = false
                        vm.renameDevice(ring.ringId, draftDeviceName)
                    },
                )
            },
            dismissButton = { SharpButton("Cancel", onClick = { renamingDevice = false }) },
        )
    }

    if (confirmForget) {
        AlertDialog(
            onDismissRequest = { confirmForget = false },
            modifier = Modifier
                .padding(end = TmDimens.shadowOverlay, bottom = TmDimens.shadowOverlay)
                .tmHardShadow(Tm.colors.shadowOverlay, TmDimens.shadowOverlay)
                .border(TmDimens.bw1, Tm.colors.line1, MaterialTheme.shapes.extraLarge),
            containerColor = Tm.colors.surface,
            tonalElevation = 0.dp,
            title = { Text("Forget ${ring.name}?") },
            text = {
                Text(
                    buildAnnotatedString {
                        append("You will need ")
                        withStyle(SpanStyle(fontFamily = Tm.text.code.fontFamily)) { append("bring pair") }
                        append(" to connect to this Ring again.")
                    },
                )
            },
            confirmButton = {
                SharpButton("Forget", danger = true, primary = true, onClick = {
                    confirmForget = false
                    vm.forget(ring.ringId)
                })
            },
            dismissButton = { SharpButton("Cancel", onClick = { confirmForget = false }) },
        )
    }
}

/** One permission: what it is for, and a way to grant it while it is missing. */
private class Permission(val name: String, val purpose: String, val granted: Boolean, val grant: () -> Unit)

/**
 * Everything the app needs from Android in one panel. All granted: folded,
 * the header says so and a tap shows the list. Anything missing: open, the
 * missing ones in `warning` with a button to grant.
 */
@SuppressLint("InlinedApi") // the notification request only runs on API 33+
@Composable
private fun PermissionsPanel(storage: Boolean) {
    val context = LocalContext.current
    fun notificationsOn() = Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
        ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) ==
        PackageManager.PERMISSION_GRANTED
    var notifications by remember { mutableStateOf(notificationsOn()) }
    // Back from the system screen: the user may have changed it there.
    LifecycleResumeEffect(Unit) {
        notifications = notificationsOn()
        onPauseOrDispose {}
    }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { notifications = it }
    val list = listOf(
        Permission(
            "All-files access",
            "Read watched folders by path and delete offloaded copies.",
            storage,
        ) {
            try {
                context.startActivity(StorageAccess.requestIntent(context))
            } catch (_: ActivityNotFoundException) {
                context.startActivity(StorageAccess.fallbackIntent())
            }
        },
        Permission(
            "Notifications",
            "The blinking upload mark in the status bar while a file uploads.",
            notifications,
        ) { ask.launch(Manifest.permission.POST_NOTIFICATIONS) },
    )
    val missing = list.count { !it.granted }
    var open by rememberSaveable(missing) { mutableStateOf(missing > 0) }
    Panel(
        title = "Permissions",
        trailing = {
            StatusPlate(
                if (missing == 0) "All granted" else "$missing required",
                if (missing == 0) PlateMark.COMPLETE else PlateMark.WARNING,
                small = true,
            )
        },
        expanded = open,
        onToggle = { open = !open },
    ) {
        list.forEachIndexed { i, p ->
            if (i > 0) RowDivider()
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                Column(Modifier.weight(1f).padding(end = TmDimens.space2)) {
                    Text(
                        p.name,
                        style = MaterialTheme.typography.bodyMedium,
                        color = if (p.granted) Tm.colors.text else Tm.colors.warning,
                    )
                    Text(
                        p.purpose,
                        style = MaterialTheme.typography.bodySmall,
                        color = if (p.granted) Tm.colors.text2 else Tm.colors.warning,
                    )
                }
                if (p.granted) Tag("Granted", TmTone.Success) else SharpButton("Grant", primary = true, onClick = p.grant)
            }
        }
    }
}
