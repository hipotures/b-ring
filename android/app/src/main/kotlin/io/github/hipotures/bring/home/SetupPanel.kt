package io.github.hipotures.bring.home

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import io.github.hipotures.bring.ui.BringTheme
import io.github.hipotures.bring.ui.Panel
import io.github.hipotures.bring.ui.SharpButton

/** Shown until storage access is granted and something is watched. */
@Composable
fun SetupPanel(storageGranted: Boolean, folderCount: Int, onOpenSettings: () -> Unit) {
    if (storageGranted && folderCount > 0) return
    Panel(title = "Setup") {
        if (!storageGranted) {
            Text("• Grant all-files access", style = MaterialTheme.typography.bodyMedium)
        }
        if (folderCount == 0) {
            Text("• Choose folders to back up (e.g. DCIM)", style = MaterialTheme.typography.bodyMedium)
        }
        Text(
            "Files on the Ring are listed either way.",
            style = MaterialTheme.typography.bodyMedium,
            color = BringTheme.colors.dim,
        )
        SharpButton("Open settings", primary = true, onClick = onOpenSettings)
    }
}
