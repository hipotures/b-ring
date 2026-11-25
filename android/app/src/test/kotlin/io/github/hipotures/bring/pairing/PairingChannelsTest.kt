package io.github.hipotures.bring.pairing

import io.github.hipotures.bring.ffi.MoonException
import io.github.hipotures.bring.ffi.SoundReceiver
import io.github.hipotures.bring.ffi.blePairingPayloadUuid
import io.github.hipotures.bring.ffi.blePairingServiceUuid
import io.github.hipotures.bring.ffi.pairingCodeFromBytes
import io.github.hipotures.bring.ffi.soundLimits
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Test
import java.util.UUID

class PairingChannelsTest {
    /** Raw bytes of `bring-core`'s payload test sample (see [PairingCodeTest]). */
    private val sampleBytes = byteArrayOf(1, 1, 2, 3, 4, 5, 6, 7, 8) +
        ByteArray(16) { 0xAB.toByte() } +
        byteArrayOf(192.toByte(), 168.toByte(), 1, 10, 0x1D, 0x13)

    @Test fun bleBytesBecomeTheSameCodeAsTheQr() {
        assertEquals(
            "040G4-0R40M-30E25-BNENT-QAXBN-ENTQA-XBNEN-TQAXB-R2M02-2GX2C-B",
            pairingCodeFromBytes(sampleBytes),
        )
        assertNull(pairingCodeFromBytes(sampleBytes.copyOf(30)))
    }

    @Test fun bleUuidsParse() {
        val service = UUID.fromString(blePairingServiceUuid())
        val payload = UUID.fromString(blePairingPayloadUuid())
        assert(service != payload)
    }

    @Test fun soundReceiverHearsNothingInSilenceAndRejectsOddRates() {
        SoundReceiver(SoundListener.SAMPLE_RATE.toUInt()).use { rx ->
            repeat(30) { assertNull(rx.pushPcm16(ByteArray(9600))) }
            assertEquals(0f, rx.level())
        }
        assertThrows(MoonException.Invalid::class.java) { SoundReceiver(1000u) }
    }

    @Test fun soundSettingsBuildAValidReceiver() {
        val limits = soundLimits()
        assertEquals(listOf(1875u, 18000u), SoundSettings().bandsHz(limits))
        assertEquals(listOf(limits.maxBandHz), SoundSettings(bands = SoundBands.CUSTOM, customHz = 99_999).bandsHz(limits))
        for (b in SoundBands.entries) for (d in SoundDetection.entries) {
            val cfg = SoundSettings(bands = b, detection = d).listenConfig(limits)
            SoundReceiver.withConfig(SoundListener.SAMPLE_RATE.toUInt(), cfg).use { rx ->
                assertEquals(cfg.bandsHz.size, rx.takeScores().size)
            }
        }
        assertEquals("18000–20953 Hz", SoundSettings.bandRange(18_000u, limits))
    }
}
