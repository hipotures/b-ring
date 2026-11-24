package io.github.hipotures.bring

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import io.github.hipotures.bring.ffi.RingInfo
import io.github.hipotures.bring.pairing.PairingScreen
import io.github.hipotures.bring.ui.BringTheme
import io.github.hipotures.bring.ui.KeyValue
import io.github.hipotures.bring.ui.Panel
import io.github.hipotures.bring.ui.SharpButton

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            val dark by bring.prefs.darkTheme.collectAsStateWithLifecycle()
            BringTheme(dark) {
                Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
                    Box(Modifier.safeDrawingPadding()) { App() }
                }
            }
        }
    }
}

@Composable
private fun App(vm: AppViewModel = viewModel()) {
    when (val p = vm.pairing.collectAsStateWithLifecycle().value) {
        AppViewModel.Pairing.Loading -> {}
        AppViewModel.Pairing.Unpaired -> PairingScreen(onPaired = vm::onPaired)
        is AppViewModel.Pairing.Paired -> PairedHome(p.ring, onForget = vm::forget)
    }
}

@Composable
private fun PairedHome(ring: RingInfo, onForget: () -> Unit) {
    Column(Modifier.padding(16.dp)) {
        Panel(title = "Ring") {
            KeyValue("device", ring.deviceSlug)
            KeyValue("lan", ring.lanUrl)
            KeyValue("wan", ring.wanUrl ?: "—")
            SharpButton("Forget Ring", onClick = onForget, danger = true)
        }
    }
}
