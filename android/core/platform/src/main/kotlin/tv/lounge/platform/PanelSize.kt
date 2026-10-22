package tv.lounge.platform

import android.content.Context
import android.hardware.display.DisplayManager
import android.view.Display

/**
 * The panel's own pixels. Most TVs draw their apps at 1080p on a 2160p panel; a screensaver can
 * still fill the panel through a surface of its own size, and its pictures are prepared for it.
 */
data class PanelSize(val width: Int, val height: Int) {
    companion object {
        fun of(context: Context): PanelSize {
            val display = (context.getSystemService(Context.DISPLAY_SERVICE) as? DisplayManager)?.getDisplay(Display.DEFAULT_DISPLAY)
                ?: return PanelSize(0, 0)
            val mode = display.mode
            return PanelSize(mode.physicalWidth, mode.physicalHeight)
        }

        /** The window's pixels: what the TV draws apps at. */
        fun window(context: Context): PanelSize {
            val m = context.resources.displayMetrics
            return PanelSize(m.widthPixels, m.heightPixels)
        }
    }
}
