package io.github.hipotures.bring.home

import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import io.github.hipotures.bring.ui.Panel
import io.github.hipotures.bring.ui.SharpButton
import themechanics.compose.Tm
import themechanics.compose.TmDimens

/** Shown until storage access is granted and something is watched. */
@Composable
fun SetupPanel(storageGranted: Boolean, folderCount: Int, onOpenSettings: () -> Unit) {
    if (storageGranted && folderCount > 0) return
    Panel(
        modifier = Modifier.padding(
            start = TmDimens.panelPad,
            end = TmDimens.panelPad,
            top = TmDimens.space2,
        ),
        title = "Setup",
    ) {
        if (!storageGranted) {
            Text("• Grant all-files access", style = MaterialTheme.typography.bodyMedium)
        }
        if (folderCount == 0) {
            Text("• Choose folders to back up (e.g. DCIM)", style = MaterialTheme.typography.bodyMedium)
        }
        Text(
            "Files on the Ring are listed either way.",
            style = MaterialTheme.typography.bodyMedium,
            color = Tm.colors.text2,
        )
        SharpButton("Open settings", primary = true, onClick = onOpenSettings)
    }
}
