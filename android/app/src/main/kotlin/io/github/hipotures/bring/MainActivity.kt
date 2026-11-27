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
import androidx.compose.ui.platform.LocalContext
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
import io.github.hipotures.bring.settings.AppSettingsScreen
import io.github.hipotures.bring.settings.FolderPicker
import io.github.hipotures.bring.settings.RingSettingsScreen
import io.github.hipotures.bring.sync.BackgroundAccess
import io.github.hipotures.bring.ui.BringTheme
import io.github.hipotures.bring.ui.RingBar
import io.github.hipotures.bring.ui.RingSwitcher
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

private enum class Screen { HOME, VIEW, SETTINGS, RING_SETTINGS, ACTIVITY, ADD_RING }

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
    var pickingFolder by rememberSaveable { mutableStateOf(false) }
    val route by vm.route.collectAsStateWithLifecycle()
    val routes by vm.routes.collectAsStateWithLifecycle()
    val sync by vm.sync.collectAsStateWithLifecycle()
    val storage by vm.storageGranted.collectAsStateWithLifecycle()
    val folders by vm.folders.collectAsStateWithLifecycle()
    val browse by vm.browse.collectAsStateWithLifecycle()
    val rounds by vm.rounds.collectAsStateWithLifecycle()
    val lastChecked by vm.lastChecked.collectAsStateWithLifecycle()
    val wantActivity by openActivity.collectAsStateWithLifecycle()
    val active = p.active
    val multiRing = p.rings.size > 1
    LaunchedEffect(wantActivity) {
        if (wantActivity) {
            screen = Screen.ACTIVITY
            openActivity.value = false
        }
    }
    LaunchedEffect(screen) {
        if (screen == Screen.HOME) vm.loadEntries()
    }
    // Once per install, right after the first pairing (or the first start of a paired app):
    // without the battery exemption Samsung cuts background network and sync waits for the app.
    val context = LocalContext.current
    LaunchedEffect(Unit) {
        val prefs = context.bring.prefs
        if (!prefs.backgroundAsked && !BackgroundAccess.granted(context)) {
            prefs.backgroundAsked = true
            BackgroundAccess.request(context)
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
                    title = "B-Ring",
                    route = null,
                    sync = sync,
                    showAppIcon = true,
                    onAddRing = { screen = Screen.ADD_RING },
                    onSettings = { screen = Screen.SETTINGS },
                )
                RingSwitcher(
                    rings = p.rings,
                    activeRingId = p.activeRingId,
                    routes = routes,
                    onSelect = { id ->
                        vm.selectRing(id)
                        dir = ""
                    },
                )
                RingBar(
                    route = route,
                    sync = sync,
                    onSync = vm::syncNow,
                    onRingSettings = { screen = Screen.RING_SETTINGS },
                    onActivity = { screen = Screen.ACTIVITY },
                    lastRound = rounds.firstOrNull(),
                    lastChecked = lastChecked,
                    queued = browse?.queued?.files?.toInt() ?: 0,
                    multiRing = multiRing,
                )
                BrowserScreen(
                    browse = browse,
                    dir = dir,
                    onDir = { dir = it },
                    onShow = vm::showDir,
                    onOpen = {
                        viewing = it.id
                        screen = Screen.VIEW
                    },
                    header = {
                        SetupPanel(
                            storage,
                            folders.size,
                            onOpenSettings = {
                                screen = if (!storage) Screen.SETTINGS else Screen.RING_SETTINGS
                            },
                        )
                    },
                    uploading = sync.path.takeIf { sync.running && sync.ringId == active.ringId },
                    onFreeLocal = vm::freeLocal,
                    ringName = active.name,
                    watchedFolders = folders.filter { it.enabled }.map { it.ringFolder },
                    onAddFolder = {
                        if (storage) pickingFolder = true else screen = Screen.SETTINGS
                    },
                )
                if (pickingFolder) {
                    FolderPicker(
                        watched = folders.map { it.localDir }.toSet(),
                        onPick = {
                            pickingFolder = false
                            vm.addFolder(it)
                        },
                        onDismiss = { pickingFolder = false },
                    )
                }
            }
            Screen.VIEW -> {
                TopStrip("", route, sync, onBack = { screen = Screen.HOME })
                viewing?.let {
                    ViewerScreen(
                        it,
                        route,
                        uploading = sync.path.takeIf { sync.running && sync.ringId == active.ringId },
                    )
                }
            }
            Screen.ACTIVITY -> {
                TopStrip("Sync activity", route = null, sync = sync, onBack = { screen = Screen.HOME })
                ActivityScreen(
                    rounds,
                    lastChecked,
                    p.rings,
                    browse?.queued,
                    activeRingName = p.rings.firstOrNull { it.ringId == p.activeRingId }?.name.orEmpty(),
                )
            }
            Screen.SETTINGS -> {
                TopStrip("Settings", route = null, sync = sync, onBack = { screen = Screen.HOME })
                AppSettingsScreen(vm = vm)
            }
            Screen.RING_SETTINGS -> {
                TopStrip("Ring · ${active.name}", route = route, sync = sync, onBack = { screen = Screen.HOME })
                RingSettingsScreen(
                    rings = p.rings,
                    ring = active,
                    vm = vm,
                    onOpenPermissions = { screen = Screen.SETTINGS },
                )
            }
            Screen.ADD_RING -> {
                TopStrip("Add Ring", route = null, sync = sync, onBack = { screen = Screen.HOME })
                PairingScreen(onPaired = {
                    vm.onPaired(it)
                    dir = ""
                    screen = Screen.HOME
                })
            }
        }
    }
}
