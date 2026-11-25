package io.github.hipotures.bring.pairing

import android.app.Application
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.core.content.ContextCompat
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import androidx.lifecycle.viewmodel.compose.viewModel
import io.github.hipotures.bring.bring
import io.github.hipotures.bring.ffi.soundLimits
import io.github.hipotures.bring.ui.Panel
import io.github.hipotures.bring.ui.PlateMark
import io.github.hipotures.bring.ui.SharpButton
import io.github.hipotures.bring.ui.StatusPlate
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import themechanics.compose.Tm
import themechanics.compose.TmDimens

/** What the microphone hears during a sound test. */
data class SoundLive(
    val level: Float = 0f,
    val source: String = "",
    val bands: List<BandScore> = emptyList(),
    val frames: Int = 0,
)

sealed interface SoundTestState {
    data object Idle : SoundTestState
    /** Continuous listening with the saved settings, until Stop. */
    data class Listening(val live: SoundLive) : SoundTestState
    /** Sweep step [step] of [total] ([label]). */
    data class Sweeping(val step: Int, val total: Int, val label: String, val live: SoundLive) : SoundTestState
    data class Failed(val message: String) : SoundTestState
}

/**
 * Sound reception test for `bring pair --sound` (any band): listens without
 * stopping at a code, or sweeps every mic source × voice processing.
 * Nothing is paired; decoded codes are only counted.
 */
class SoundTestViewModel(app: Application) : AndroidViewModel(app) {
    private val prefs = app.bring.prefs
    private val _state = MutableStateFlow<SoundTestState>(SoundTestState.Idle)
    val state: StateFlow<SoundTestState> = _state.asStateFlow()
    /** Last live reading, kept after Stop so it can be read. */
    private val _last = MutableStateFlow<SoundLive?>(null)
    val last: StateFlow<SoundLive?> = _last.asStateFlow()
    private val _sweep = MutableStateFlow<List<SweepResult>>(emptyList())
    /** Results of the last sweep (partial if stopped). */
    val sweep: StateFlow<List<SweepResult>> = _sweep.asStateFlow()
    private var job: Job? = null

    fun listen() = run {
        _last.value = null
        _state.value = SoundTestState.Listening(SoundLive())
        try {
            SoundListener.listen(getApplication(), prefs.sound.value, onCode = {}) { p ->
                val live = SoundLive(p.level, p.source, p.bands, p.frames)
                _last.value = live
                _state.value = SoundTestState.Listening(live)
            }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            _state.value = SoundTestState.Failed(e.message ?: e.toString())
        }
    }

    /** Tries every mic source × voice processing for [STEP_MS] each. */
    fun sweep() = run {
        val base = prefs.sound.value
        val combos = SweepResult.combos()
        _sweep.value = emptyList()
        combos.forEachIndexed { i, (source, raw) ->
            var row = SweepResult(source, raw)
            _state.value = SoundTestState.Sweeping(i + 1, combos.size, row.label, SoundLive())
            val started = System.currentTimeMillis()
            try {
                withTimeoutOrNull(STEP_MS) {
                    SoundListener.listen(
                        getApplication(),
                        base.copy(source = source, rawProcessing = raw),
                        onCode = {
                            if (row.firstMs == null) row = row.copy(firstMs = System.currentTimeMillis() - started)
                        },
                    ) { p ->
                        row = row.copy(
                            sourceName = p.source,
                            frames = p.frames,
                            peakLevel = maxOf(row.peakLevel, p.level),
                            bands = p.bands,
                        )
                        _state.value = SoundTestState.Sweeping(
                            i + 1, combos.size, row.label, SoundLive(p.level, p.source, p.bands, p.frames),
                        )
                    }
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                row = row.copy(error = e.message ?: e.toString())
            }
            _sweep.value = _sweep.value + row
        }
        _state.value = SoundTestState.Idle
    }

    fun stop() {
        job?.cancel()
        job = null
        _state.value = SoundTestState.Idle
    }

    fun failed(message: String) {
        stop()
        _state.value = SoundTestState.Failed(message)
    }

    private fun run(block: suspend () -> Unit) {
        job?.cancel()
        job = viewModelScope.launch { block() }
    }

    private companion object {
        /** ~5 frames of a repeating beacon (1.86 s each). */
        const val STEP_MS = 10_000L
    }
}

/**
 * "Sound test" in the app settings: run `bring pair --sound` (with any
 * `--sound-band`, `--sound-volume`) on the Ring and watch what the phone hears.
 */
@Composable
fun SoundTestPanel(expanded: Boolean, onToggle: () -> Unit, vm: SoundTestViewModel = viewModel()) {
    val context = LocalContext.current
    val state by vm.state.collectAsStateWithLifecycle()
    val last by vm.last.collectAsStateWithLifecycle()
    val rows by vm.sweep.collectAsStateWithLifecycle()
    // The microphone only runs while the settings screen is shown.
    DisposableEffect(Unit) { onDispose { vm.stop() } }
    var pending by rememberSaveable { mutableStateOf<String?>(null) }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        val action = pending
        pending = null
        when {
            !granted -> vm.failed("Microphone permission denied.")
            action == "sweep" -> vm.sweep()
            else -> vm.listen()
        }
    }
    fun start(action: String) {
        val ok = ContextCompat.checkSelfPermission(context, SoundListener.PERMISSION) == PackageManager.PERMISSION_GRANTED
        when {
            !ok -> {
                pending = action
                ask.launch(SoundListener.PERMISSION)
            }
            action == "sweep" -> vm.sweep()
            else -> vm.listen()
        }
    }

    Panel(
        title = "Sound test",
        trailing = {
            when (state) {
                is SoundTestState.Listening, is SoundTestState.Sweeping ->
                    StatusPlate("Listening", PlateMark.ACTIVE, small = true)
                else -> {}
            }
        },
        expanded = expanded,
        onToggle = onToggle,
    ) {
        Text(
            "Run “bring pair --sound” on the Ring (add --sound-band high, --sound-volume …) and listen here. " +
                "Nothing is paired; the test keeps listening after a code arrives.",
            style = MaterialTheme.typography.bodySmall,
            color = Tm.colors.text2,
        )
        val busy = state is SoundTestState.Listening || state is SoundTestState.Sweeping
        Row(horizontalArrangement = Arrangement.spacedBy(TmDimens.space2)) {
            SharpButton(
                if (state is SoundTestState.Listening) "Stop" else "Listen",
                primary = state !is SoundTestState.Listening,
                enabled = state !is SoundTestState.Sweeping,
                onClick = { if (busy) vm.stop() else start("listen") },
                modifier = Modifier.weight(1f),
            )
            SharpButton(
                if (state is SoundTestState.Sweeping) "Stop sweep" else "Sweep mic (~80 s)",
                enabled = state !is SoundTestState.Listening,
                onClick = { if (busy) vm.stop() else start("sweep") },
                modifier = Modifier.weight(1f),
            )
        }
        // Every line is always there and one line high, so the panel never
        // changes height (that made the settings page jump while scrolled).
        val settings by context.bring.prefs.sound.collectAsStateWithLifecycle()
        val limits = remember { soundLimits() }
        val placeholder = remember(settings) { settings.bandsHz(limits).map { BandScore(it, 0f, 0f) } }
        val live = when (val s = state) {
            is SoundTestState.Listening -> s.live
            is SoundTestState.Sweeping -> s.live
            else -> last ?: SoundLive()
        }
        val (status, statusColor) = when (val s = state) {
            SoundTestState.Idle -> "Idle." to Tm.colors.text2
            is SoundTestState.Listening -> "Listening with the saved settings…" to Tm.colors.text2
            is SoundTestState.Sweeping -> "Sweep ${s.step}/${s.total}: ${s.label}" to Tm.colors.text2
            is SoundTestState.Failed -> s.message to Tm.colors.danger
        }
        Text(
            status,
            style = MaterialTheme.typography.bodyMedium,
            color = statusColor,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        LevelMeter(if (busy) live.level else 0f)
        BandScores(live.bands.ifEmpty { placeholder }, live.source.ifEmpty { "—" })
        Text(
            "Decoded OK: ${live.frames} ${if (live.frames == 1) "frame" else "frames"}",
            style = MaterialTheme.typography.titleMedium,
            color = if (live.frames > 0) Tm.colors.success else Tm.colors.text2,
            maxLines = 1,
        )
        SweepTable(rows, running = (state as? SoundTestState.Sweeping)?.step)
    }
}
