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
import androidx.compose.material3.LinearProgressIndicator
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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions
import io.github.hipotures.bring.ffi.RingInfo
import io.github.hipotures.bring.ffi.isValidPairingCode
import io.github.hipotures.bring.ui.BringTheme
import io.github.hipotures.bring.ui.ErrorLine
import io.github.hipotures.bring.ui.Mono
import io.github.hipotures.bring.ui.Panel
import io.github.hipotures.bring.ui.Sharp
import io.github.hipotures.bring.ui.SharpButton
import io.github.hipotures.bring.ui.Tag

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
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text("PAIR WITH A RING", style = MaterialTheme.typography.titleLarge, fontFamily = Mono)
        Text(
            "Run `bring pair` on the Ring. Scan its QR code, or type the code printed under it.",
            style = MaterialTheme.typography.bodyMedium,
            color = BringTheme.colors.dim,
        )

        when (val s = state) {
            is PairState.Verify -> VerifyPanel(s, onCancel = vm::cancel)
            PairState.Connecting -> Panel(title = "Connecting") {
                LinearProgressIndicator(Modifier.fillMaxWidth())
                Text("Key exchange with the Ring…", style = MaterialTheme.typography.bodyMedium)
            }
            else -> {
                Panel(
                    title = "1 · Code",
                    trailing = {
                        when {
                            code.isBlank() -> {}
                            valid -> Tag("CHECKSUM OK", BringTheme.colors.synced)
                            else -> Tag("INVALID", BringTheme.colors.warn)
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
                                    .setPrompt("Scan the code shown by `bring pair`"),
                            )
                        },
                        modifier = Modifier.fillMaxWidth(),
                    )
                    OutlinedTextField(
                        value = code,
                        onValueChange = { code = it.uppercase() },
                        label = { Text("Code") },
                        placeholder = { Text("XXXXX-XXXXX-…", fontFamily = Mono) },
                        textStyle = MaterialTheme.typography.bodyLarge.copy(fontFamily = Mono),
                        shape = Sharp,
                        minLines = 2,
                        keyboardOptions = KeyboardOptions(
                            capitalization = KeyboardCapitalization.Characters,
                            autoCorrectEnabled = false,
                            keyboardType = KeyboardType.Ascii,
                        ),
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
                Panel(title = "2 · This device") {
                    OutlinedTextField(
                        value = name,
                        onValueChange = { name = it },
                        label = { Text("Name shown on the Ring") },
                        singleLine = true,
                        shape = Sharp,
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
    }
}

@Composable
private fun VerifyPanel(s: PairState.Verify, onCancel: () -> Unit) {
    Panel(title = "3 · Verify") {
        Text(
            "Check that the Ring shows the same code, then approve there (y).",
            style = MaterialTheme.typography.bodyMedium,
        )
        Text(
            formatSas(s.sas),
            fontFamily = Mono,
            fontSize = 44.sp,
            fontWeight = FontWeight.Bold,
            letterSpacing = 4.sp,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.align(Alignment.CenterHorizontally),
        )
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                s.ringUrl,
                fontFamily = Mono,
                fontSize = 12.sp,
                color = BringTheme.colors.dim,
                modifier = Modifier.weight(1f),
            )
            Tag("WAITING", MaterialTheme.colorScheme.primary)
        }
        LinearProgressIndicator(Modifier.fillMaxWidth())
        s.note?.let { ErrorLine(it) }
        SharpButton("Cancel", onClick = onCancel, danger = true, modifier = Modifier.fillMaxWidth())
    }
}
