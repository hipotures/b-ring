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
import androidx.compose.material.icons.filled.Close
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import io.github.hipotures.bring.storage.subfolders
import io.github.hipotures.bring.storage.volumes
import io.github.hipotures.bring.ui.BringTheme
import io.github.hipotures.bring.ui.FolderGlyph
import io.github.hipotures.bring.ui.Hairline
import io.github.hipotures.bring.ui.MetaStyle
import io.github.hipotures.bring.ui.Sharp
import io.github.hipotures.bring.ui.SharpButton
import io.github.hipotures.bring.ui.Tag
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

    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Column(
            Modifier
                .fillMaxSize()
                .background(MaterialTheme.colorScheme.background)
                .safeDrawingPadding(),
        ) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .background(MaterialTheme.colorScheme.surfaceContainer)
                    .height(60.dp)
                    .padding(start = 16.dp, end = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text("Choose folder", style = MaterialTheme.typography.titleLarge, modifier = Modifier.weight(1f))
                IconButton(onDismiss) { Icon(Icons.Filled.Close, "Close") }
            }
            HorizontalDivider(thickness = Hairline, color = MaterialTheme.colorScheme.outline)

            if (vols.size > 1) {
                Row(
                    Modifier
                        .fillMaxWidth()
                        .horizontalScroll(rememberScrollState())
                        .padding(horizontal = 12.dp, vertical = 6.dp),
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    vols.forEach { v ->
                        SharpButton(v.label, primary = v == volume, onClick = { path = v.root.path })
                    }
                }
            }

            // Breadcrumbs: volume / a / b, each tappable.
            val rel = path.removePrefix(volume.root.path).trim('/')
            Row(
                Modifier
                    .fillMaxWidth()
                    .horizontalScroll(rememberScrollState())
                    .padding(horizontal = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Crumb(volume.label, current = atRoot) { path = volume.root.path }
                var acc = volume.root.path
                if (rel.isNotEmpty()) {
                    rel.split('/').forEach { part ->
                        acc = "$acc/$part"
                        val target = acc
                        Text("›", style = MaterialTheme.typography.bodyLarge, color = BringTheme.colors.dim)
                        Crumb(part, current = target == path) { path = target }
                    }
                }
            }
            HorizontalDivider(thickness = Hairline, color = MaterialTheme.colorScheme.outlineVariant)

            LazyColumn(Modifier.weight(1f)) {
                if (!atRoot) {
                    item {
                        DirRow("Up one level", note = "up") { path = dir.parent ?: volume.root.path }
                        HorizontalDivider(thickness = Hairline, color = MaterialTheme.colorScheme.outlineVariant)
                    }
                }
                val list = children
                when {
                    list == null -> item { Note("Reading…") }
                    list.isEmpty() -> item { Note("No subfolders.") }
                    else -> items(list, key = { it.path }) { f ->
                        DirRow(f.name, watched = f.path in watched) { path = f.path }
                        HorizontalDivider(thickness = Hairline, color = MaterialTheme.colorScheme.outlineVariant)
                    }
                }
            }

            HorizontalDivider(thickness = Hairline, color = MaterialTheme.colorScheme.outline)
            Column(
                Modifier
                    .fillMaxWidth()
                    .background(MaterialTheme.colorScheme.surfaceContainer)
                    .padding(12.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text(path, style = MaterialTheme.typography.bodyMedium, maxLines = 2, overflow = TextOverflow.MiddleEllipsis)
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
        style = MaterialTheme.typography.bodyLarge,
        fontWeight = if (current) FontWeight.SemiBold else FontWeight.Normal,
        color = if (current) MaterialTheme.colorScheme.onBackground else MaterialTheme.colorScheme.primary,
        modifier = Modifier
            .clickable(enabled = !current, onClick = onClick)
            .padding(vertical = 12.dp, horizontal = 8.dp),
    )
}

@Composable
private fun DirRow(name: String, note: String? = null, watched: Boolean = false, onClick: () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .heightIn(min = 56.dp)
            .padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (note == "up") {
            Text("↑", fontSize = 22.sp, color = MaterialTheme.colorScheme.primary, modifier = Modifier.width(28.dp))
        } else {
            Icon(FolderGlyph, null, Modifier.size(28.dp), tint = MaterialTheme.colorScheme.primary)
        }
        Spacer(Modifier.width(14.dp))
        Text(
            name,
            style = MaterialTheme.typography.bodyLarge,
            maxLines = 1,
            overflow = TextOverflow.MiddleEllipsis,
            modifier = Modifier.weight(1f),
        )
        if (watched) Tag("WATCHED", BringTheme.colors.synced)
        Text(" ›", fontSize = 22.sp, color = BringTheme.colors.dim)
    }
}

@Composable
private fun Note(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.bodyMedium,
        color = BringTheme.colors.dim,
        modifier = Modifier.padding(16.dp),
    )
}
