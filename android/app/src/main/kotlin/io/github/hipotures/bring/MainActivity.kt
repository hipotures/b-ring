package io.github.hipotures.bring

import android.content.Intent
import android.os.Bundle
import androidx.activity.SystemBarStyle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import io.github.hipotures.bring.activity.ActivityScreen
import io.github.hipotures.bring.browser.BrowserScreen
import io.github.hipotures.bring.browser.parentOf
import io.github.hipotures.bring.home.SetupPanel
import io.github.hipotures.bring.pairing.PairingScreen
import io.github.hipotures.bring.settings.SettingsScreen
import io.github.hipotures.bring.ui.BringTheme
import io.github.hipotures.bring.ui.TopStrip
import io.github.hipotures.bring.viewer.ViewerScreen
import kotlinx.coroutines.flow.MutableStateFlow
import themechanics.compose.Tm

class MainActivity : ComponentActivity() {
    /** Set when opened from the "sync finished" notification. */
    private val openActivity = MutableStateFlow(false)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        openActivity.value = intent?.getBooleanExtra(EXTRA_ACTIVITY, false) == true
        setContent {
            val theme by bring.prefs.theme.collectAsStateWithLifecycle()
            BringTheme(theme) {
                val c = Tm.colors
                // System bar icons must contrast with the app shell (chrome).
                LaunchedEffect(c) {
                    val bars = if (c.isDark) {
                        SystemBarStyle.dark(c.chrome.toArgb())
                    } else {
                        SystemBarStyle.light(c.chrome.toArgb(), c.text.toArgb())
                    }
                    enableEdgeToEdge(statusBarStyle = bars, navigationBarStyle = bars)
                }
                // Behind the system bars: the shell color; the page is canvas.
                Surface(Modifier.fillMaxSize(), color = c.chrome, tonalElevation = 0.dp, shadowElevation = 0.dp) {
                    Box(Modifier.safeDrawingPadding().fillMaxSize().background(c.canvas)) { App(openActivity) }
                }
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        if (intent.getBooleanExtra(EXTRA_ACTIVITY, false)) openActivity.value = true
    }

    companion object {
        const val EXTRA_ACTIVITY = "open_activity"
    }
}

private enum class Screen { HOME, VIEW, SETTINGS, ACTIVITY }

@Composable
private fun App(openActivity: MutableStateFlow<Boolean>, vm: AppViewModel = viewModel()) {
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) { vm.onResume() }
    val pairing by vm.pairing.collectAsStateWithLifecycle()
    when (val p = pairing) {
        AppViewModel.Pairing.Loading -> {}
        AppViewModel.Pairing.Unpaired -> PairingScreen(onPaired = vm::onPaired)
        is AppViewModel.Pairing.Paired -> Paired(p, vm, openActivity)
    }
}

@Composable
private fun Paired(p: AppViewModel.Pairing.Paired, vm: AppViewModel, openActivity: MutableStateFlow<Boolean>) {
    var screen by rememberSaveable { mutableStateOf(Screen.HOME) }
    var dir by rememberSaveable { mutableStateOf("") }
    var viewing by rememberSaveable { mutableStateOf<Long?>(null) }
    val route by vm.route.collectAsStateWithLifecycle()
    val sync by vm.sync.collectAsStateWithLifecycle()
    val storage by vm.storageGranted.collectAsStateWithLifecycle()
    val folders by vm.folders.collectAsStateWithLifecycle()
    val entries by vm.entries.collectAsStateWithLifecycle()
    val rounds by vm.rounds.collectAsStateWithLifecycle()
    val lastChecked by vm.lastChecked.collectAsStateWithLifecycle()
    val wantActivity by openActivity.collectAsStateWithLifecycle()
    LaunchedEffect(wantActivity) {
        if (wantActivity) {
            screen = Screen.ACTIVITY
            openActivity.value = false
        }
    }

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
                    // The path is in the breadcrumbs right below.
                    title = "B-Ring",
                    route = route,
                    sync = sync,
                    onBack = if (dir.isEmpty()) null else ({ dir = parentOf(dir) }),
                    onSync = vm::syncNow,
                    onActivity = { screen = Screen.ACTIVITY },
                    onSettings = { screen = Screen.SETTINGS },
                    lastRound = rounds.firstOrNull(),
                    lastChecked = lastChecked,
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
                        SetupPanel(storage, folders.size, onOpenSettings = { screen = Screen.SETTINGS })
                    },
                    uploading = sync.path.takeIf { sync.running },
                )
            }
            Screen.VIEW -> {
                TopStrip("", route, sync, onBack = { screen = Screen.HOME })
                viewing?.let { ViewerScreen(it, route, uploading = sync.path.takeIf { sync.running }) }
            }
            Screen.ACTIVITY -> {
                TopStrip("Sync activity", route, sync, onBack = { screen = Screen.HOME })
                ActivityScreen(rounds, lastChecked)
            }
            Screen.SETTINGS -> {
                TopStrip("Settings", route, sync, onBack = { screen = Screen.HOME })
                SettingsScreen(p.ring, vm)
            }
        }
    }
}
