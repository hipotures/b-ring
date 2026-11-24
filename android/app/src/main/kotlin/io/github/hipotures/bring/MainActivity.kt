package io.github.hipotures.bring

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import io.github.hipotures.bring.browser.BrowserScreen
import io.github.hipotures.bring.browser.parentOf
import io.github.hipotures.bring.home.SetupPanel
import io.github.hipotures.bring.pairing.PairingScreen
import io.github.hipotures.bring.settings.SettingsScreen
import io.github.hipotures.bring.ui.BringTheme
import io.github.hipotures.bring.ui.ErrorLine
import io.github.hipotures.bring.ui.TopStrip
import io.github.hipotures.bring.viewer.ViewerScreen

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

private enum class Screen { HOME, VIEW, SETTINGS }

@Composable
private fun App(vm: AppViewModel = viewModel()) {
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) { vm.onResume() }
    val pairing by vm.pairing.collectAsStateWithLifecycle()
    when (val p = pairing) {
        AppViewModel.Pairing.Loading -> {}
        AppViewModel.Pairing.Unpaired -> PairingScreen(onPaired = vm::onPaired)
        is AppViewModel.Pairing.Paired -> Paired(p, vm)
    }
}

@Composable
private fun Paired(p: AppViewModel.Pairing.Paired, vm: AppViewModel) {
    var screen by rememberSaveable { mutableStateOf(Screen.HOME) }
    var dir by rememberSaveable { mutableStateOf("") }
    var viewing by rememberSaveable { mutableStateOf<Long?>(null) }
    val route by vm.route.collectAsStateWithLifecycle()
    val sync by vm.sync.collectAsStateWithLifecycle()
    val storage by vm.storageGranted.collectAsStateWithLifecycle()
    val folders by vm.folders.collectAsStateWithLifecycle()
    val entries by vm.entries.collectAsStateWithLifecycle()
    val syncError by vm.syncError.collectAsStateWithLifecycle()

    BackHandler(enabled = screen != Screen.HOME || dir.isNotEmpty()) {
        when {
            screen != Screen.HOME -> screen = Screen.HOME
            else -> dir = parentOf(dir)
        }
    }
    Column {
        when (screen) {
            Screen.HOME -> {
                TopStrip(
                    title = dir.substringAfterLast('/'),
                    route = route,
                    sync = sync,
                    onBack = if (dir.isEmpty()) null else ({ dir = parentOf(dir) }),
                    onSync = vm::syncNow,
                    onSettings = { screen = Screen.SETTINGS },
                )
                BrowserScreen(
                    entries = entries,
                    dir = dir,
                    onDir = { dir = it },
                    onOpen = {
                        viewing = it.id
                        screen = Screen.VIEW
                    },
                    header = {
                        Column {
                            syncError?.let { ErrorLine(it) }
                            SetupPanel(storage, folders.size, onOpenSettings = { screen = Screen.SETTINGS })
                        }
                    },
                )
            }
            Screen.VIEW -> {
                TopStrip("", route, sync, onBack = { screen = Screen.HOME })
                viewing?.let { ViewerScreen(it, route) }
            }
            Screen.SETTINGS -> {
                TopStrip("Settings", route, sync, onBack = { screen = Screen.HOME })
                SettingsScreen(p.ring, vm)
            }
        }
    }
}
