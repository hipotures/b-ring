package io.github.hipotures.bring.pairing

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.hipotures.bring.bring
import io.github.hipotures.bring.ffi.SoundLimits
import io.github.hipotures.bring.ffi.soundLimits
import io.github.hipotures.bring.ui.Panel
import io.github.hipotures.bring.ui.Segmented
import io.github.hipotures.bring.ui.SharpButton
import io.github.hipotures.bring.ui.Tag
import io.github.hipotures.bring.ui.tmFieldColors
import themechanics.compose.Tm
import themechanics.compose.TmDimens
import themechanics.compose.TmTone
import java.util.Locale

/** 1875 → "1.9k", 18000 → "18k". */
private fun kHz(hz: UInt) = String.format(Locale.ROOT, "%.1f", hz.toFloat() / 1000f).removeSuffix(".0") + "k"

/**
 * Advanced settings of "Listen": bands, detection threshold, microphone
 * source, voice processing and diagnostics. Shown on the pairing screen
 * (folded) and in the app settings; both edit the same [SoundSettings].
 */
@Composable
fun SoundSettingsPanel(expanded: Boolean = true, onToggle: (() -> Unit)? = null) {
    val prefs = LocalContext.current.bring.prefs
    val s by prefs.sound.collectAsStateWithLifecycle()
    val limits = remember { soundLimits() }
    fun set(next: SoundSettings) = prefs.setSound(next)
    val mono = SpanStyle(fontFamily = Tm.text.code.fontFamily)
    fun AnnotatedString.Builder.code(text: String) = withStyle(mono) { append(text) }

    Panel(
        title = "Sound pairing",
        trailing = { if (!s.isDefault) Tag("Tuned", TmTone.Info) },
        expanded = expanded,
        onToggle = onToggle,
    ) {
        Text(
            buildAnnotatedString {
                append("Listen on the band the Ring plays: ")
                code("bring pair --sound")
                append(" is Low (audible), ")
                code("--sound-band high")
                append(" is High (inaudible to most adults), ")
                code("--sound-band <Hz>")
                append(" is Custom.")
            },
            style = MaterialTheme.typography.bodySmall,
            color = Tm.colors.text2,
        )

        Label("Listen on")
        Segmented(
            SoundBands.entries,
            s.bands,
            { set(s.copy(bands = it)) },
            label = { it.label },
            sub = {
                when (it) {
                    SoundBands.BOTH -> "${kHz(limits.lowBandHz).removeSuffix("k")}+${kHz(limits.highBandHz)}"
                    SoundBands.LOW -> kHz(limits.lowBandHz)
                    SoundBands.HIGH -> kHz(limits.highBandHz)
                    SoundBands.CUSTOM -> kHz(s.customHz.toUInt())
                }
            },
        )
        if (s.bands == SoundBands.CUSTOM) CustomBandField(s, limits, ::set)

        Label("Detection")
        Segmented(
            SoundDetection.entries,
            s.detection,
            { set(s.copy(detection = it)) },
            label = { it.label },
            sub = { String.format(Locale.ROOT, "%.2f", it.threshold(limits)) },
        )
        Hint("Sensitive catches weaker or distant sound; every result is still checksummed.")

        Label("Microphone source")
        Segmented(MicSource.entries, s.source, { set(s.copy(source = it)) }, label = { it.label })
        Hint("Auto uses the raw (unprocessed) microphone where the phone offers it.")

        Label("Voice processing")
        Segmented(
            listOf(true, false),
            s.rawProcessing,
            { set(s.copy(rawProcessing = it)) },
            label = { if (it) "Off" else "Phone default" },
        )
        Hint("Echo cancellation, noise suppression and gain control smear tones; keep them off.")

        Label("Diagnostics")
        Segmented(
            listOf(false, true),
            s.diagnostics,
            { set(s.copy(diagnostics = it)) },
            label = { if (it) "On" else "Off" },
        )
        Hint("Shows how strongly each band hears a frame start, and keeps listening after a code arrives.")

        if (!s.isDefault) {
            SharpButton("Reset to defaults", onClick = { set(SoundSettings()) })
        }
    }
}

@Composable
private fun CustomBandField(s: SoundSettings, limits: SoundLimits, set: (SoundSettings) -> Unit) {
    var draft by remember { mutableStateOf(s.customHz.toString()) }
    val parsed = draft.toIntOrNull()
    val valid = parsed != null && parsed.toUInt() in limits.minBandHz..limits.maxBandHz
    OutlinedTextField(
        value = draft,
        onValueChange = { text ->
            draft = text.filter(Char::isDigit).take(5)
            draft.toIntOrNull()
                ?.takeIf { it.toUInt() in limits.minBandHz..limits.maxBandHz }
                ?.let { set(s.copy(customHz = it)) }
        },
        label = { Text("Lowest tone, Hz") },
        singleLine = true,
        isError = !valid,
        supportingText = {
            Text(
                if (valid) {
                    SoundSettings.bandRange(parsed!!.toUInt(), limits) + " (rounded to 46.875 Hz steps)"
                } else {
                    "${limits.minBandHz}–${limits.maxBandHz}"
                },
                style = Tm.text.code,
            )
        },
        textStyle = MaterialTheme.typography.bodyLarge.copy(fontFamily = Tm.text.code.fontFamily),
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
        colors = tmFieldColors(),
        modifier = Modifier.fillMaxWidth(),
    )
}

/**
 * Per-band detection score while listening: ~0.06 is noise, the threshold
 * starts a decode, 1.0 is a clean frame start. Bar = recent peak, number =
 * best so far.
 */
@Composable
fun BandScores(bands: List<BandScore>, source: String, frames: Int) {
    val limits = remember { soundLimits() }
    val threshold = LocalContext.current.bring.prefs.sound.value.detection.threshold(limits)
    Column(verticalArrangement = Arrangement.spacedBy(TmDimens.space1)) {
        Label("Diagnostics · mic: $source · frames decoded: $frames")
        bands.forEach { b ->
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(TmDimens.space2)) {
                Text(
                    SoundSettings.bandRange(b.lowHz, limits),
                    style = Tm.text.numeric,
                    color = Tm.colors.text2,
                    modifier = Modifier.width(132.dp),
                )
                ScoreBar(b.score, threshold, Modifier.weight(1f))
                Text(
                    String.format(Locale.ROOT, "%.2f", b.best),
                    style = Tm.text.numeric,
                    color = if (b.best >= threshold) Tm.colors.success else Tm.colors.text2,
                )
            }
        }
    }
}

/**
 * Sweep results, one line per mic setting: frames decoded, time to the
 * first one, peak input level and best score per band. The winner is green.
 */
@Composable
fun SweepTable(rows: List<SweepResult>) {
    val winner = SweepResult.best(rows)
    Column(verticalArrangement = Arrangement.spacedBy(TmDimens.space1)) {
        Label("Sweep · frames · first · level · best score per band")
        rows.forEach { r ->
            val text = r.error?.let { "${r.label}: $it" } ?: buildString {
                append(r.label.padEnd(18))
                append(" ${r.frames}fr")
                append(r.firstMs?.let { String.format(Locale.ROOT, " %4.1fs", it / 1000f) } ?: "    –")
                append(String.format(Locale.ROOT, " L%.2f", r.peakLevel))
                r.bands.forEach { b -> append(" ${kHz(b.lowHz)}:" + String.format(Locale.ROOT, "%.2f", b.best)) }
            }
            Text(
                text,
                style = Tm.text.code,
                color = when {
                    r === winner -> Tm.colors.success
                    r.error != null -> Tm.colors.warning
                    else -> Tm.colors.text
                },
            )
            if (r.error == null && r.sourceName.isNotEmpty() && r.sourceName != r.source.name.lowercase()) {
                Text("  (${r.sourceName})", style = Tm.text.code, color = Tm.colors.text2)
            }
        }
        if (winner != null) {
            Hint("Best: ${winner.label}. Set it under Sound pairing if Auto does worse.")
        }
    }
}

/** Square bar on a `line3` track with a tick at the threshold. */
@Composable
private fun ScoreBar(score: Float, threshold: Float, modifier: Modifier) {
    Box(
        modifier
            .height(8.dp)
            .border(TmDimens.bw3, Tm.colors.line3)
            .background(Tm.colors.sunken),
    ) {
        Box(
            Modifier
                .fillMaxWidth(score.coerceIn(0f, 1f))
                .fillMaxHeight()
                .background(if (score >= threshold) Tm.colors.success else Tm.colors.accent),
        )
        Row(Modifier.fillMaxWidth().fillMaxHeight()) {
            Box(Modifier.weight(threshold.coerceIn(0.01f, 0.99f)))
            Box(Modifier.width(TmDimens.bw2).fillMaxHeight().background(Tm.colors.ink))
            Box(Modifier.weight(1f - threshold.coerceIn(0.01f, 0.99f)))
        }
    }
}

@Composable
private fun Label(text: String) = Text(text.uppercase(), style = Tm.text.eyebrow, color = Tm.colors.text2)

@Composable
private fun Hint(text: String) = Text(text, style = MaterialTheme.typography.bodySmall, color = Tm.colors.text2)
