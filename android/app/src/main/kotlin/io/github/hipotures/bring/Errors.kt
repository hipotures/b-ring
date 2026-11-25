package io.github.hipotures.bring

import io.github.hipotures.bring.ffi.MoonException

/** A short, human-readable line for an engine (or other) failure. */
fun Throwable.describe(): String = when (this) {
    is MoonException.NotPaired -> "Not paired with a Ring"
    is MoonException.Network -> "Network: $detail"
    is MoonException.Server ->
        "Ring answered $status${if (code.isEmpty()) "" else " ($code)"}: $detail"
    is MoonException.Pairing -> "Pairing failed: $detail"
    is MoonException.PairingPending -> "Waiting for approval on the Ring"
    is MoonException.PairingRejected -> "Pairing was rejected on the Ring"
    is MoonException.PairingExpired -> "Pairing session expired; run “bring pair” again"
    is MoonException.Storage -> "Storage: $detail"
    is MoonException.Invalid -> detail
    is MoonException.Cancelled -> "Cancelled"
    else -> message ?: javaClass.simpleName
}
