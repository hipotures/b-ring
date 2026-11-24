package io.github.hipotures.bring.pairing

import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.util.AttributeSet
import com.journeyapps.barcodescanner.ViewfinderView
import io.github.hipotures.bring.R

/**
 * ZXing's viewfinder with a 2 dp frame (The Mechanics `line1`) around the
 * scan area. Colors come from res/values(-night)/colors.xml: the scanner is
 * a separate Activity, outside Compose and the in-app theme choice.
 */
class FramedViewfinderView(context: Context, attrs: AttributeSet?) : ViewfinderView(context, attrs) {
    private val frame = Paint().apply {
        style = Paint.Style.STROKE
        strokeWidth = 2f * resources.displayMetrics.density
        color = context.getColor(R.color.tm_scanner_frame)
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        val r = framingRect ?: return
        val inset = frame.strokeWidth / 2
        canvas.drawRect(r.left - inset, r.top - inset, r.right + inset, r.bottom + inset, frame)
    }
}
