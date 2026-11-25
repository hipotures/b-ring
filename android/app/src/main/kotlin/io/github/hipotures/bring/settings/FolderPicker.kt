package io.github.hipotures.bring.settings

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.KeyboardArrowUp
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.DialogWindowProvider
import androidx.core.view.WindowCompat
import io.github.hipotures.bring.storage.subfolders
import io.github.hipotures.bring.storage.volumes
import io.github.hipotures.bring.ui.ActionIcon
import io.github.hipotures.bring.ui.FolderGlyph
import io.github.hipotures.bring.ui.RowDivider
import io.github.hipotures.bring.ui.Segmented
import io.github.hipotures.bring.ui.SharpButton
import io.github.hipotures.bring.ui.Tag
import themechanics.compose.Tm
import themechanics.compose.TmDimens
import themechanics.compose.TmTone
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File

/**
 * Full-screen folder browser over the device's storage (all-files access):
 * tap into folders, then "Watch this folder". Volume roots themselves cannot
 * be picked (that would back up the whole device).
 */
@Composable
fun FolderPicker(watched: Set<String>, onPick: (String) -> Unit, onDismiss: () -> Unit) {
    val context = LocalContext.current
    val vols = remember { volumes(context) }
    var path by rememberSaveable { mutableStateOf(vols.first().root.path) }
    val dir = File(path)
    val volume = vols.firstOrNull { path == it.root.path || path.startsWith(it.root.path + "/") } ?: vols.first()
    val atRoot = path == volume.root.path
    val children by produceState<List<File>?>(null, path) {
        value = null
        value = withContext(Dispatchers.IO) { subfolders(dir) }
    }

    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false)) {
        // The dialog has its own window: match the system bar icons to the theme.
        val window = (LocalView.current.parent as? DialogWindowProvider)?.window
        val dark = Tm.colors.isDark
        SideEffect {
            window?.let {
                WindowCompat.getInsetsController(it, it.decorView).apply {
                    isAppearanceLightStatusBars = !dark
                    isAppearanceLightNavigationBars = !dark
                }
            }
        }
        Column(
            Modifier
                .fillMaxSize()
                .background(Tm.colors.chrome)
                .safeDrawingPadding()
                .background(Tm.colors.canvas),
        ) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .background(Tm.colors.head1)
                    .height(56.dp)
                    .padding(start = TmDimens.regionPad, end = TmDimens.space1),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    "Choose folder",
                    style = MaterialTheme.typography.titleLarge,
                    color = Tm.colors.text,
                    modifier = Modifier.weight(1f),
                )
                ActionIcon(Icons.Outlined.Close, "Close", onDismiss)
            }
            HorizontalDivider(thickness = TmDimens.bw1, color = Tm.colors.line1)

            if (vols.size > 1) {
                Segmented(
                    options = vols,
                    selected = volume,
                    onSelect = { path = it.root.path },
                    label = { it.label },
                    modifier = Modifier.padding(horizontal = TmDimens.panelPad, vertical = TmDimens.space2),
                )
            }

            // Breadcrumbs: volume / a / b, each tappable.
            val rel = path.removePrefix(volume.root.path).trim('/')
            Row(
                Modifier
                    .fillMaxWidth()
                    .horizontalScroll(rememberScrollState())
                    .padding(horizontal = TmDimens.space1),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Crumb(volume.label, current = atRoot) { path = volume.root.path }
                var acc = volume.root.path
                if (rel.isNotEmpty()) {
                    rel.split('/').forEach { part ->
                        acc = "$acc/$part"
                        val target = acc
                        Text("/", style = Tm.text.code, color = Tm.colors.text3)
                        Crumb(part, current = target == path) { path = target }
                    }
                }
            }
            HorizontalDivider(thickness = TmDimens.bw2, color = Tm.colors.line2)

            LazyColumn(Modifier.weight(1f)) {
                if (!atRoot) {
                    item {
                        DirRow("Up one level", up = true) { path = dir.parent ?: volume.root.path }
                        RowDivider()
                    }
                }
                val list = children
                when {
                    list == null -> item { Note("Reading…") }
                    list.isEmpty() -> item { Note("No subfolders.") }
                    else -> items(list, key = { it.path }) { f ->
                        val isWatched = f.path in watched
                        val hasWatchedChild = !isWatched && watched.any { it.startsWith("${f.path}/") }
                        DirRow(f.name, watched = isWatched, hasWatchedChild = hasWatchedChild) { path = f.path }
                        RowDivider()
                    }
                }
            }

            HorizontalDivider(thickness = TmDimens.bw1, color = Tm.colors.line1)
            Column(
                Modifier
                    .fillMaxWidth()
                    .background(Tm.colors.chrome)
                    .padding(horizontal = TmDimens.regionPad, vertical = TmDimens.space2),
                verticalArrangement = Arrangement.spacedBy(TmDimens.space1),
            ) {
                Text(path, style = Tm.text.code, color = Tm.colors.text2, maxLines = 2, overflow = TextOverflow.Ellipsis)
                val already = path in watched
                SharpButton(
                    when {
                        atRoot -> "Open a folder to pick it"
                        already -> "Already watched"
                        else -> "Watch this folder"
                    },
                    enabled = !atRoot && !already,
                    primary = true,
                    onClick = { onPick(path) },
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        }
    }
}

@Composable
private fun Crumb(text: String, current: Boolean, onClick: () -> Unit) {
    Text(
        text,
        style = MaterialTheme.typography.bodyMedium,
        fontWeight = if (current) FontWeight.SemiBold else FontWeight.Normal,
        color = if (current) Tm.colors.text else Tm.colors.link,
        modifier = Modifier
            .clickable(enabled = !current, onClick = onClick)
            .padding(vertical = 14.dp, horizontal = TmDimens.space2),
    )
}

@Composable
private fun DirRow(
    name: String,
    up: Boolean = false,
    watched: Boolean = false,
    hasWatchedChild: Boolean = false,
    onClick: () -> Unit,
) {
    val c = Tm.colors
    Row(
        Modifier
            .fillMaxWidth()
            .background(if (hasWatchedChild) c.successSoft else c.surface)
            .clickable(onClick = onClick)
            .heightIn(min = 56.dp)
            .padding(horizontal = TmDimens.regionPad, vertical = TmDimens.space2),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            if (up) Icons.Outlined.KeyboardArrowUp else FolderGlyph,
            null,
            Modifier.size(TmDimens.iconLg),
            tint = if (hasWatchedChild) c.success else c.text2,
        )
        Spacer(Modifier.width(TmDimens.space4))
        Text(
            name,
            style = MaterialTheme.typography.bodyMedium,
            fontWeight = FontWeight.Medium,
            color = c.text,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.weight(1f),
        )
        if (watched) {
            Tag("Watched", TmTone.Success)
            Spacer(Modifier.width(TmDimens.space2))
        }
        if (!up) Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, null, Modifier.size(TmDimens.iconLg), tint = c.text3)
    }
}

@Composable
private fun Note(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.bodyMedium,
        color = Tm.colors.text2,
        modifier = Modifier.padding(TmDimens.regionPad),
    )
}
