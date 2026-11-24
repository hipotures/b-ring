package io.github.hipotures.bring.pairing

import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.google.zxing.client.android.Intents
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions
import io.github.hipotures.bring.ffi.RingInfo
import io.github.hipotures.bring.ffi.isValidPairingCode
import io.github.hipotures.bring.versionLine
import io.github.hipotures.bring.ui.BusyBar
import io.github.hipotures.bring.ui.ErrorLine
import io.github.hipotures.bring.ui.Panel
import io.github.hipotures.bring.ui.PlateMark
import io.github.hipotures.bring.ui.SharpButton
import io.github.hipotures.bring.ui.StatusPlate
import io.github.hipotures.bring.ui.tmFieldColors
import themechanics.compose.Tm
import themechanics.compose.TmDimens

@Composable
fun PairingScreen(onPaired: (RingInfo) -> Unit, vm: PairingViewModel = viewModel()) {
    val state by vm.state.collectAsStateWithLifecycle()
    var code by rememberSaveable { mutableStateOf("") }
    var name by rememberSaveable { mutableStateOf(Build.MODEL ?: "moon") }
    val valid = remember(code) { code.isNotBlank() && isValidPairingCode(code) }

    // Plain ZXing (no Google Play Services). Devices without a usable camera
    // (Meta Quest 3) use the text field instead.
    val scanner = rememberLauncherForActivityResult(ScanContract()) { result ->
        result.contents?.let { code = groupPairingCode(it) }
    }

    LaunchedEffect(state) {
        (state as? PairState.Done)?.let { onPaired(it.ring) }
    }

    Column(
        Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .imePadding()
            .padding(TmDimens.regionPad),
        verticalArrangement = Arrangement.spacedBy(TmDimens.gapPanel),
    ) {
        Text("Pair with a Ring", style = MaterialTheme.typography.headlineMedium, color = Tm.colors.text)
        Text(
            "Run `bring pair` on the Ring. Scan its QR code, or type the code printed under it.",
            style = MaterialTheme.typography.bodyMedium,
            color = Tm.colors.text2,
        )

        when (val s = state) {
            is PairState.Verify -> VerifyPanel(s, onCancel = vm::cancel)
            PairState.Connecting -> Panel(title = "Connecting") {
                BusyBar()
                Text("Key exchange with the Ring…", style = MaterialTheme.typography.bodyMedium)
            }
            else -> {
                Panel(
                    title = "1. Pairing code",
                    trailing = {
                        when {
                            code.isBlank() -> {}
                            valid -> StatusPlate("Checksum OK", PlateMark.COMPLETE, small = true)
                            else -> StatusPlate("Invalid", PlateMark.ERROR, small = true)
                        }
                    },
                ) {
                    SharpButton(
                        "Scan QR code",
                        onClick = {
                            scanner.launch(
                                ScanOptions()
                                    .setDesiredBarcodeFormats(ScanOptions.QR_CODE)
                                    .setBeepEnabled(false)
                                    .setOrientationLocked(false)
                                    // Terminals with a dark theme show the code inverted.
                                    .addExtra(Intents.Scan.SCAN_TYPE, Intents.Scan.MIXED_SCAN)
                                    .setPrompt("Scan the code shown by `bring pair`"),
                            )
                        },
                        modifier = Modifier.fillMaxWidth(),
                    )
                    OutlinedTextField(
                        value = code,
                        onValueChange = { code = it.uppercase() },
                        label = { Text("Code") },
                        placeholder = { Text("XXXXX-XXXXX-…", fontFamily = Tm.text.code.fontFamily) },
                        textStyle = MaterialTheme.typography.bodyLarge.copy(fontFamily = Tm.text.code.fontFamily),
                        colors = tmFieldColors(),
                        minLines = 2,
                        keyboardOptions = KeyboardOptions(
                            capitalization = KeyboardCapitalization.Characters,
                            autoCorrectEnabled = false,
                            keyboardType = KeyboardType.Ascii,
                        ),
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
                Panel(title = "2. This device") {
                    OutlinedTextField(
                        value = name,
                        onValueChange = { name = it },
                        label = { Text("Name shown on the Ring") },
                        singleLine = true,
                        colors = tmFieldColors(),
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
                if (s is PairState.Failed) ErrorLine(s.message)
                SharpButton(
                    "Pair",
                    onClick = { vm.start(code, name) },
                    enabled = valid,
                    primary = true,
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        }
        Text(versionLine(LocalContext.current), style = Tm.text.code, color = Tm.colors.text2)
    }
}

@Composable
private fun VerifyPanel(s: PairState.Verify, onCancel: () -> Unit) {
    Panel(title = "3. Verify") {
        Text(
            "Check that the Ring shows the same code, then approve there (y).",
            style = MaterialTheme.typography.bodyMedium,
        )
        Text(
            formatSas(s.sas),
            style = MaterialTheme.typography.displayLarge,
            color = Tm.colors.text,
            modifier = Modifier.align(Alignment.CenterHorizontally),
        )
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(TmDimens.space2)) {
            Text(s.ringUrl, style = Tm.text.code, color = Tm.colors.text2, modifier = Modifier.weight(1f))
            StatusPlate("Waiting", PlateMark.ACTIVE, small = true)
        }
        BusyBar()
        s.note?.let { ErrorLine(it) }
        SharpButton("Cancel", onClick = onCancel, modifier = Modifier.fillMaxWidth())
    }
}
