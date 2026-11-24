package io.github.hipotures.bring.pairing

import com.google.zxing.BarcodeFormat
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.qrcode.QRCodeWriter
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Test

class QrDecoderTest {
    @Test fun decodesTerminalStyleQrWithHairlinesOnDarkBackground() {
        val text = "04B26-NGBK9-GGEBS-S3496-7X5KV-EHT5T-91WFJ-4D6XH-R2M02-S0X2C-3"
        val matrix = QRCodeWriter().encode(text, BarcodeFormat.QR_CODE, 0, 0)
        val quiet = 4
        val modW = 10
        val modH = 12 // slightly taller modules like monospace terminal cells
        val qrW = (matrix.width + 2 * quiet) * modW
        val qrH = (matrix.height + 2 * quiet) * modH
        val frameW = qrW + 300
        val frameH = qrH + 200
        val ox = 20
        val oy = 80
        // Dark terminal background (luminance ~25)
        val yuv = ByteArray(frameW * frameH) { 25 }
        for (my in -quiet until matrix.height + quiet) {
            for (mx in -quiet until matrix.width + quiet) {
                val dark = mx in 0 until matrix.width && my in 0 until matrix.height && matrix.get(mx, my)
                val lum: Byte = if (dark) 35 else 245.toByte()
                val px0 = ox + (mx + quiet) * modW
                val py0 = oy + (my + quiet) * modH
                for (dy in 0 until modH) {
                    // Simulate 1px grey anti-aliasing seam between terminal half-block rows:
                    // local HybridBinarizer thresholds it to white (breaking finder patterns),
                    // while GlobalHistogramBinarizer keeps it black.
                    val rowLum = if (dy == 0 && (my + quiet) % 2 == 0 && dark) 110.toByte() else lum
                    val rowOff = (py0 + dy) * frameW + px0
                    for (dx in 0 until modW) yuv[rowOff + dx] = rowLum
                }
            }
        }
        val source = PlanarYUVLuminanceSource(yuv, frameW, frameH, 0, 0, frameW, frameH, false)
        val decoder = RobustQrDecoderFactory().createDecoder(null)
        val result = decoder.decode(source)
        assertNotNull("RobustQrDecoder must decode terminal-style QR on dark background", result)
        assertEquals(text, result!!.text)
    }
}
