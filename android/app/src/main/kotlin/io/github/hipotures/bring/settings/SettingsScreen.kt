package io.github.hipotures.bring.settings

import android.Manifest
import android.annotation.SuppressLint
import android.content.ActivityNotFoundException
import android.content.pm.PackageManager
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.foundation.layout.size
import io.github.hipotures.bring.ui.FolderGlyph
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
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.hipotures.bring.AppViewModel
import io.github.hipotures.bring.bring
import io.github.hipotures.bring.ffi.RingInfo
import io.github.hipotures.bring.storage.StorageAccess
import io.github.hipotures.bring.versionLine
import io.github.hipotures.bring.ui.BringTheme
import io.github.hipotures.bring.ui.ErrorLine
import io.github.hipotures.bring.ui.KeyValue
import io.github.hipotures.bring.ui.MetaStyle
import io.github.hipotures.bring.ui.Panel
import io.github.hipotures.bring.ui.Sharp
import io.github.hipotures.bring.ui.SharpButton
import io.github.hipotures.bring.ui.Tag
import io.github.hipotures.bring.ui.routeColor

private val MIB: ULong = 1024UL * 1024UL

/** Offload choices; ULong.MAX_VALUE means "never delete local copies". */
private val OFFLOAD_CHOICES = listOf(
    "25 MiB" to 25UL * MIB,
    "100 MiB" to 100UL * MIB,
    "500 MiB" to 500UL * MIB,
    "2 GiB" to 2048UL * MIB,
    "Never" to ULong.MAX_VALUE,
)

@Composable
fun SettingsScreen(ring: RingInfo, vm: AppViewModel) {
    val context = LocalContext.current
    val route by vm.route.collectAsStateWithLifecycle()
    val storage by vm.storageGranted.collectAsStateWithLifecycle()
    val folders by vm.folders.collectAsStateWithLifecycle()
    val message by vm.message.collectAsStateWithLifecycle()
    val dark by context.bring.prefs.darkTheme.collectAsStateWithLifecycle()
    var offload by remember { mutableStateOf(context.bring.prefs.offloadThresholdBytes ?: (100UL * MIB)) }
    var picking by rememberSaveable { mutableStateOf(false) }
    var confirmForget by remember { mutableStateOf(false) }
    DisposableEffect(Unit) { onDispose { vm.clearMessage() } }

    Column(
        Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .imePadding()
            .padding(12.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        message?.let { ErrorLine(it) }

        Panel(title = "Ring") {
            KeyValue("Device", ring.deviceSlug)
            KeyValue("Ring ID", ring.ringId.take(16), code = true)
            KeyValue("LAN", ring.lanUrl, code = true)
            KeyValue("WAN", ring.wanUrl ?: "— (LAN only)", code = ring.wanUrl != null)
            KeyValue("TLS pin", ring.tlsCertSha256.chunked(4).take(8).joinToString(" ") + " …", code = true)
        }

        Panel(title = "Connection", trailing = { Tag(route.label, routeColor(route)) }) {
            Text(
                "LAN when on unmetered Wi-Fi and the Ring answers within 1.5 s; " +
                    "otherwise WAN if configured. Background sync runs on LAN only.",
                style = MaterialTheme.typography.bodyMedium,
                color = BringTheme.colors.dim,
            )
            SharpButton("Check again", onClick = vm::recheckRoute)
        }

        Panel(
            title = "Storage access",
            trailing = {
                Tag(
                    if (storage) "GRANTED" else "REQUIRED",
                    if (storage) BringTheme.colors.synced else BringTheme.colors.warn,
                )
            },
        ) {
            Text(
                "All-files access lets the engine read watched folders by path and " +
                    "delete offloaded copies.",
                style = MaterialTheme.typography.bodyMedium,
                color = BringTheme.colors.dim,
            )
            if (!storage) {
                SharpButton("Grant all-files access", primary = true, onClick = {
                    try {
                        context.startActivity(StorageAccess.requestIntent(context))
                    } catch (_: ActivityNotFoundException) {
                        context.startActivity(StorageAccess.fallbackIntent())
                    }
                })
            }
            NotificationRow()
        }

        Panel(title = "Watched folders") {
            if (folders.isEmpty()) {
                Text("None yet.", style = MaterialTheme.typography.bodyMedium, color = BringTheme.colors.dim)
            }
            folders.forEach { f ->
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(FolderGlyph, null, Modifier.size(24.dp), tint = MaterialTheme.colorScheme.primary)
                    Column(Modifier.weight(1f).padding(start = 12.dp)) {
                        Text(
                            f.localDir.removePrefix(StorageAccess.root().path + "/"),
                            style = MaterialTheme.typography.bodyLarge,
                        )
                        Text("on the Ring: ${f.ringFolder}", style = MetaStyle, color = BringTheme.colors.dim)
                    }
                    IconButton(onClick = { vm.removeFolder(f.id) }) {
                        Icon(Icons.Filled.Delete, "Stop watching", tint = MaterialTheme.colorScheme.error)
                    }
                }
            }
            val watched = folders.map { it.localDir }.toSet()
            val suggestions = remember(storage, watched) {
                if (storage) StorageAccess.suggestions().map { it.path }.filter { it !in watched } else emptyList()
            }
            if (suggestions.isNotEmpty()) {
                Text("Quick add", style = MaterialTheme.typography.bodyMedium, color = BringTheme.colors.dim)
                FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
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
                    "Grant all-files access above to pick folders.",
                    style = MetaStyle,
                    color = BringTheme.colors.dim,
                )
            }
        }

        Panel(title = "Offload") {
            Text(
                "After a verified upload, delete the local copy of files larger than:",
                style = MaterialTheme.typography.bodyMedium,
                color = BringTheme.colors.dim,
            )
            FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                OFFLOAD_CHOICES.forEach { (label, bytes) ->
                    SharpButton(label, primary = offload == bytes, onClick = {
                        offload = bytes
                        vm.setOffloadThreshold(bytes)
                    })
                }
            }
        }

        Panel(title = "Appearance") {
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                SharpButton("Dark", primary = dark, onClick = { vm.setDarkTheme(true) })
                SharpButton("Light", primary = !dark, onClick = { vm.setDarkTheme(false) })
            }
        }

        Panel(title = "Pairing") {
            Text(
                "Forgetting the Ring drops its index on this device. Local files are not touched.",
                style = MaterialTheme.typography.bodyMedium,
                color = BringTheme.colors.dim,
            )
            SharpButton("Forget Ring", danger = true, onClick = { confirmForget = true })
        }

        Text(versionLine(context), style = MetaStyle, color = BringTheme.colors.dim)
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

    if (confirmForget) {
        AlertDialog(
            onDismissRequest = { confirmForget = false },
            shape = Sharp,
            title = { Text("Forget this Ring?") },
            text = { Text("You will need `bring pair` to connect again.") },
            confirmButton = {
                SharpButton("Forget", danger = true, primary = true, onClick = {
                    confirmForget = false
                    vm.forget()
                })
            },
            dismissButton = { SharpButton("Cancel", onClick = { confirmForget = false }) },
        )
    }
}

/** POST_NOTIFICATIONS (API 33+): upload progress and the sync service. */
@SuppressLint("InlinedApi") // the request button only shows on API 33+
@Composable
private fun NotificationRow() {
    val context = LocalContext.current
    fun granted() = Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
        ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) ==
        PackageManager.PERMISSION_GRANTED
    var ok by remember { mutableStateOf(granted()) }
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { ok = it }
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
        Text("Notifications", style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
        if (ok) {
            Tag("ON", BringTheme.colors.synced)
        } else {
            SharpButton("Allow", onClick = { launcher.launch(Manifest.permission.POST_NOTIFICATIONS) })
        }
    }
}
