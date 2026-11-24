package io.github.hipotures.bring.settings

import android.Manifest
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
import androidx.compose.runtime.Composable
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
    var customDir by rememberSaveable { mutableStateOf("") }
    var confirmForget by remember { mutableStateOf(false) }

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
            KeyValue("device", ring.deviceSlug)
            KeyValue("ring id", ring.ringId.take(16))
            KeyValue("lan", ring.lanUrl)
            KeyValue("wan", ring.wanUrl ?: "— (LAN only)")
            KeyValue("tls pin", ring.tlsCertSha256.chunked(4).take(8).joinToString(" ") + " …")
        }

        Panel(title = "Connection", trailing = { Tag(route.label, routeColor(route)) }) {
            Text(
                "LAN when on unmetered Wi-Fi and the Ring answers within 500 ms; " +
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
                    Column(Modifier.weight(1f)) {
                        Text(f.localDir, style = MetaStyle)
                        Text("→ ${f.ringFolder}", style = MetaStyle, color = MaterialTheme.colorScheme.primary)
                    }
                    SharpButton("Remove", onClick = { vm.removeFolder(f.id) }, danger = true)
                }
            }
            val watched = folders.map { it.localDir }.toSet()
            val suggestions = remember(storage, watched) {
                if (storage) StorageAccess.suggestions().map { it.path }.filter { it !in watched } else emptyList()
            }
            if (suggestions.isNotEmpty()) {
                Text("ADD", style = MetaStyle, color = BringTheme.colors.dim)
                FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    suggestions.forEach { path ->
                        SharpButton("+ ${path.substringAfterLast('/')}", onClick = { vm.addFolder(path) })
                    }
                }
            }
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                OutlinedTextField(
                    value = customDir,
                    onValueChange = { customDir = it },
                    label = { Text("Other folder (absolute path)") },
                    singleLine = true,
                    shape = Sharp,
                    textStyle = MetaStyle,
                    modifier = Modifier.weight(1f),
                )
                SharpButton("Add", enabled = customDir.startsWith("/"), onClick = {
                    vm.addFolder(customDir.trim())
                    customDir = ""
                })
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
@Composable
private fun NotificationRow() {
    val context = LocalContext.current
    fun granted() = Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
        ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) ==
        PackageManager.PERMISSION_GRANTED
    var ok by remember { mutableStateOf(granted()) }
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { ok = it }
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
        Text("NOTIFICATIONS", style = MetaStyle, color = BringTheme.colors.dim, modifier = Modifier.weight(1f))
        if (ok) {
            Tag("ON", BringTheme.colors.synced)
        } else {
            SharpButton("Allow", onClick = { launcher.launch(Manifest.permission.POST_NOTIFICATIONS) })
        }
    }
}
