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
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material3.Icon
import io.github.hipotures.bring.ui.ActionIcon
import io.github.hipotures.bring.ui.FolderGlyph
import io.github.hipotures.bring.ui.Group
import io.github.hipotures.bring.ui.PlateMark
import io.github.hipotures.bring.ui.RoutePlate
import io.github.hipotures.bring.ui.RowDivider
import io.github.hipotures.bring.ui.Segmented
import io.github.hipotures.bring.ui.StatusPlate
import io.github.hipotures.bring.ui.ThemeChoice
import themechanics.compose.Tm
import themechanics.compose.TmDimens
import themechanics.compose.TmTone
import themechanics.compose.tmHardShadow
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.hipotures.bring.AppViewModel
import io.github.hipotures.bring.bring
import io.github.hipotures.bring.ffi.RingInfo
import io.github.hipotures.bring.storage.StorageAccess
import io.github.hipotures.bring.versionLine
import io.github.hipotures.bring.ui.ErrorLine
import io.github.hipotures.bring.ui.KeyValue
import io.github.hipotures.bring.ui.Panel
import io.github.hipotures.bring.ui.SharpButton
import io.github.hipotures.bring.ui.Tag

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
    val theme by context.bring.prefs.theme.collectAsStateWithLifecycle()
    var offload by remember { mutableStateOf(context.bring.prefs.offloadThresholdBytes ?: (100UL * MIB)) }
    var picking by rememberSaveable { mutableStateOf(false) }
    var confirmForget by remember { mutableStateOf(false) }
    DisposableEffect(Unit) { onDispose { vm.clearMessage() } }

    Column(
        Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .imePadding()
            .padding(TmDimens.panelPad),
        verticalArrangement = Arrangement.spacedBy(TmDimens.gapPanel),
    ) {
        message?.let { ErrorLine(it) }

        Panel(title = "Ring") {
            KeyValue("Device", ring.deviceSlug)
            KeyValue("Ring ID", ring.ringId.take(16))
            KeyValue("LAN", ring.lanUrl)
            KeyValue("WAN", ring.wanUrl ?: "— (LAN only)")
            KeyValue("TLS pin", ring.tlsCertSha256.chunked(4).take(8).joinToString(" ") + " …")
        }

        Panel(title = "Connection", trailing = { RoutePlate(route) }) {
            Text(
                "LAN when on unmetered Wi-Fi and the Ring answers within 1.5 s; " +
                    "otherwise WAN if configured. Background sync runs on LAN only.",
                style = MaterialTheme.typography.bodyMedium,
                color = Tm.colors.text2,
            )
            SharpButton("Check again", onClick = vm::recheckRoute)
        }

        Panel(
            title = "Storage access",
            trailing = {
                StatusPlate(
                    if (storage) "Granted" else "Required",
                    if (storage) PlateMark.COMPLETE else PlateMark.WARNING,
                    small = true,
                )
            },
        ) {
            Text(
                "All-files access lets the engine read watched folders by path and " +
                    "delete offloaded copies.",
                style = MaterialTheme.typography.bodyMedium,
                color = Tm.colors.text2,
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
                                Text("→ Ring: ${f.ringFolder}", style = Tm.text.code, color = Tm.colors.text2)
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
                    "Grant all-files access above to pick folders.",
                    style = Tm.text.code,
                    color = Tm.colors.text2,
                )
            }
        }

        Panel(title = "Offload") {
            Text(
                "After a verified upload, delete the local copy of files larger than:",
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

        Panel(title = "Appearance") {
            Segmented(ThemeChoice.entries, theme, vm::setTheme, label = { it.label })
            Text(
                "System follows the phone's light or dark mode. Coffee is a warm dark theme.",
                style = MaterialTheme.typography.bodySmall,
                color = Tm.colors.text2,
            )
        }

        Panel(title = "Pairing") {
            Text(
                "Forgetting the Ring drops its index on this device. Local files are not touched.",
                style = MaterialTheme.typography.bodyMedium,
                color = Tm.colors.text2,
            )
            SharpButton("Forget Ring", danger = true, onClick = { confirmForget = true })
        }

        Text(versionLine(context), style = Tm.text.code, color = Tm.colors.text2)
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
            // Overlay: 2 dp line1 frame and a hard offset shadow, no soft elevation.
            // The padding keeps the offset shadow inside the dialog window.
            modifier = Modifier
                .padding(end = TmDimens.shadowOverlay, bottom = TmDimens.shadowOverlay)
                .tmHardShadow(Tm.colors.shadowOverlay, TmDimens.shadowOverlay)
                .border(TmDimens.bw1, Tm.colors.line1, MaterialTheme.shapes.extraLarge),
            containerColor = Tm.colors.surface,
            tonalElevation = 0.dp,
            title = { Text("Forget this Ring?") },
            text = {
                Text(
                    buildAnnotatedString {
                        append("You will need ")
                        withStyle(SpanStyle(fontFamily = Tm.text.code.fontFamily)) { append("bring pair") }
                        append(" to connect again.")
                    },
                )
            },
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
            Tag("On", TmTone.Success)
        } else {
            SharpButton("Allow", onClick = { launcher.launch(Manifest.permission.POST_NOTIFICATIONS) })
        }
    }
}
