package io.github.hipotures.bring.pairing

import android.app.Activity
import android.os.Bundle
import android.view.KeyEvent
import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.LuminanceSource
import com.google.zxing.MultiFormatReader
import com.google.zxing.Reader
import com.google.zxing.Result
import com.google.zxing.common.GlobalHistogramBinarizer
import com.google.zxing.common.HybridBinarizer
import com.journeyapps.barcodescanner.CaptureManager
import com.journeyapps.barcodescanner.DecoratedBarcodeView
import com.journeyapps.barcodescanner.Decoder
import com.journeyapps.barcodescanner.DecoderFactory
import com.journeyapps.barcodescanner.camera.CameraSettings
import java.util.EnumMap

/**
 * QR scanner Activity using [RobustQrDecoderFactory] and continuous autofocus.
 *
 * `zxing-android-embedded`'s stock decoder hardcodes [HybridBinarizer], whose
 * 8×8 local thresholding frequently breaks finder patterns on crisp terminal QR
 * codes shown on dark backgrounds (especially with sub-pixel row hairlines).
 * Trying [GlobalHistogramBinarizer] first fixes terminal screens while keeping
 * [HybridBinarizer] and inverted fallbacks for uneven lighting.
 */
class QrCaptureActivity : Activity() {
    private lateinit var capture: CaptureManager
    private lateinit var scanner: DecoratedBarcodeView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(com.google.zxing.client.android.R.layout.zxing_capture)
        scanner = findViewById(com.google.zxing.client.android.R.id.zxing_barcode_scanner)
        capture = CaptureManager(this, scanner)
        capture.initializeFromIntent(intent, savedInstanceState)
        scanner.setDecoderFactory(RobustQrDecoderFactory())
        scanner.cameraSettings.apply {
            isAutoFocusEnabled = true
            focusMode = CameraSettings.FocusMode.CONTINUOUS
            isMeteringEnabled = true
        }
        capture.decode()
    }

    override fun onResume() {
        super.onResume()
        capture.onResume()
    }

    override fun onPause() {
        super.onPause()
        capture.onPause()
    }

    override fun onDestroy() {
        super.onDestroy()
        capture.onDestroy()
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        capture.onSaveInstanceState(outState)
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        capture.onRequestPermissionsResult(requestCode, permissions, grantResults)
    }

    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean =
        scanner.onKeyDown(keyCode, event) || super.onKeyDown(keyCode, event)
}

/** Decoder that tries global and hybrid binarizers (plus inverted) on every frame. */
class RobustQrDecoder(reader: Reader) : Decoder(reader) {
    override fun decode(source: LuminanceSource): Result? {
        val attempts = arrayOf(
            BinaryBitmap(GlobalHistogramBinarizer(source)),
            BinaryBitmap(HybridBinarizer(source)),
            BinaryBitmap(GlobalHistogramBinarizer(source.invert())),
            BinaryBitmap(HybridBinarizer(source.invert())),
        )
        for (bitmap in attempts) {
            decode(bitmap)?.let { return it }
        }
        return null
    }
}

class RobustQrDecoderFactory : DecoderFactory {
    override fun createDecoder(baseHints: MutableMap<DecodeHintType, *>?): Decoder {
        val hints = EnumMap<DecodeHintType, Any>(DecodeHintType::class.java)
        baseHints?.let { hints.putAll(it) }
        hints[DecodeHintType.POSSIBLE_FORMATS] = listOf(BarcodeFormat.QR_CODE)
        hints[DecodeHintType.TRY_HARDER] = true
        val reader = MultiFormatReader().apply { setHints(hints) }
        return RobustQrDecoder(reader)
    }
}
