package io.github.hipotures.bring.pairing

/**
 * Normalises typed or scanned input the way the engine's decoder reads it
 * (case-insensitive, ignoring spaces and dashes) and regroups it in dash-separated fives,
 * as `bring pair` prints it.
 */
fun groupPairingCode(input: String): String =
    input.filter { it != '-' && !it.isWhitespace() }
        .uppercase()
        .chunked(5)
        .joinToString("-")

/** `123456` -> `123 456`: easier to compare at a glance. */
fun formatSas(sas: String): String =
    if (sas.length == 6) "${sas.substring(0, 3)} ${sas.substring(3)}" else sas
