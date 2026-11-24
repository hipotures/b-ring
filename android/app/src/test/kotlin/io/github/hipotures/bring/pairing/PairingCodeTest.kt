package io.github.hipotures.bring.pairing

import io.github.hipotures.bring.ffi.isValidPairingCode
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class PairingCodeTest {
    /** `bring-core`'s payload test sample (192.168.1.10:7443). */
    private val sample = "040G4-0R40M-30E25-BNENT-QAXBN-ENTQA-XBNEN-TQAXB-R2M02-2GX2C-B"

    @Test fun regroupsSloppyInputLikeTheRing() {
        val sloppy = sample.replace("-", "").lowercase().chunked(7).joinToString("  ")
        assertEquals(sample, groupPairingCode(sloppy))
    }

    @Test fun regroupedCodeStillValidatesInTheEngine() {
        assertTrue(isValidPairingCode(sample))
        assertTrue(isValidPairingCode(groupPairingCode(sample.lowercase())))
    }

    @Test fun singleTypoIsCaughtByTheChecksum() {
        val typo = sample.replaceFirst('R', 'S')
        assertFalse(isValidPairingCode(typo))
    }

    @Test fun sasIsSplitForComparison() {
        assertEquals("012 345", formatSas("012345"))
        assertEquals("12345", formatSas("12345"))
    }
}
