package tv.lounge.feature.home

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.PixelFormat
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import android.os.SystemClock
import android.provider.Settings
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView
import androidx.compose.ui.graphics.toArgb
import androidx.core.content.res.ResourcesCompat
import tv.lounge.core.PlatformException
import tv.lounge.design.LoungeColors
import tv.lounge.platform.HomeAlert
import java.io.ByteArrayOutputStream
import java.io.InputStream
import java.net.HttpURLConnection
import java.net.InetAddress
import java.net.NetworkInterface
import java.net.URL
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlin.concurrent.thread
import kotlin.math.roundToInt

/**
 * Pop-ups from the home, such as a ring at the door: a card in the bottom corner with a picture
 * that is fetched again while it is up. It is drawn over whatever is on screen where the TV
 * lets Lounge draw over other apps, and in Lounge's own window otherwise. The remote cannot
 * reach it, and a pointer's click takes it down.
 */
class AlertPopups(private val context: Context, private val colors: () -> LoungeColors) {
    private val main = Handler(Looper.getMainLooper())
    private var up: Up? = null

    /** Set by the home on the main thread: its window's content while it is on screen, or null. */
    @Volatile var homeWindow: () -> ViewGroup? = { null }

    private class Up(val card: View, val inHome: Boolean, val remove: () -> Unit) {
        @Volatile var gone = false
        var timeUp: Runnable? = null
        var picture: Thread? = null
    }

    /** Whether the TV lets Lounge draw over other apps. */
    fun overApps(): Boolean = Settings.canDrawOverlays(context)

    /**
     * Shows [alert] in place of any pop-up still up, and returns once it is on screen. Throws a
     * sentence when there is nowhere to show it. With the screen off there is nobody to tell.
     */
    fun show(alert: HomeAlert) {
        if (context.getSystemService(PowerManager::class.java)?.isInteractive == false) return
        var shown: Boolean? = null
        if (Looper.myLooper() == main.looper) {
            shown = present(alert)
        } else {
            val done = CountDownLatch(1)
            main.post {
                shown = present(alert)
                done.countDown()
            }
            done.await(SHOW_WAIT_MS, TimeUnit.MILLISECONDS)
        }
        if (shown != false) return
        throw PlatformException.Failed(if (overApps()) "alert: no window took the pop-up" else context.getString(R.string.alerts_not_over_apps))
    }

    /** Takes down a pop-up drawn in the home's window, as when that window goes away. */
    fun homeGone() = main.post { up?.takeIf { it.inHome }?.let(::take) }

    private fun take(shown: Up?) {
        if (shown == null || shown.gone) return
        shown.gone = true
        shown.timeUp?.let(main::removeCallbacks)
        shown.picture?.interrupt()
        runCatching { shown.remove() }
        if (up === shown) up = null
    }

    /** Puts the card up, and says whether anything took it. */
    private fun present(alert: HomeAlert): Boolean {
        take(up)
        val screen = context.resources.displayMetrics
        // Sized by the screen, since this window does not share the home's own scale.
        val width = (screen.widthPixels * 0.25f).roundToInt()
        val edgeX = (screen.widthPixels * 0.05f).roundToInt()
        val edgeY = (screen.heightPixels * 0.05f).roundToInt()
        val (card, picture) = card(alert, width, screen.heightPixels)
        val shown = overOtherApps(card, width, edgeX, edgeY) ?: inHome(card, width, edgeX, edgeY) ?: return false
        up = shown
        card.setOnClickListener { take(shown) }
        // A click is for a pointer. The remote's focus stays where it was.
        card.isFocusable = false
        shown.timeUp = Runnable { take(shown) }.also { main.postDelayed(it, alert.seconds.coerceIn(5, 120) * 1000L) }
        if (picture != null && alert.image != null) shown.picture = follow(alert.image!!, picture, width, shown)
        return true
    }

    private fun overOtherApps(card: View, width: Int, edgeX: Int, edgeY: Int): Up? {
        if (!overApps()) return null
        val windows = context.getSystemService(WindowManager::class.java) ?: return null
        val place = WindowManager.LayoutParams(
            width,
            WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
            // Android draws an overlay that lets touches through part see-through, so this one takes them.
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE,
            PixelFormat.TRANSLUCENT,
        ).apply {
            gravity = Gravity.BOTTOM or Gravity.END
            x = edgeX
            y = edgeY
        }
        if (runCatching { windows.addView(card, place) }.isFailure) return null
        return Up(card, inHome = false) { windows.removeView(card) }
    }

    private fun inHome(card: View, width: Int, edgeX: Int, edgeY: Int): Up? {
        val home = homeWindow() ?: return null
        val place = FrameLayout.LayoutParams(width, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.BOTTOM or Gravity.END).apply { setMargins(0, 0, edgeX, edgeY) }
        if (runCatching { home.addView(card, place) }.isFailure) return null
        return Up(card, inHome = true) { home.removeView(card) }
    }

    /** The card and, when the alert has a picture, the view that shows it. */
    private fun card(alert: HomeAlert, width: Int, screenHeight: Int): Pair<View, ImageView?> {
        val palette = colors()
        val corner = screenHeight * 0.0185f
        val card = LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            background = GradientDrawable().apply {
                setColor(palette.curtain.toArgb())
                cornerRadius = corner
                setStroke((screenHeight / 1080f).roundToInt().coerceAtLeast(1), palette.curtainEdge.toArgb())
            }
            clipToOutline = true
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_YES
            contentDescription = listOfNotNull(alert.title, alert.message).joinToString(". ")
        }
        val picture = alert.image?.let {
            ImageView(context).apply {
                scaleType = ImageView.ScaleType.CENTER_CROP
                setBackgroundColor(palette.velvet.toArgb())
                card.addView(this, LinearLayout.LayoutParams(width, width * 9 / 16))
            }
        }
        val pad = (screenHeight * 0.0185f).roundToInt()
        val words = LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(pad, pad * 3 / 4, pad, pad)
        }
        fun line(text: String, font: Int, size: Float, colour: Int, lines: Int) = TextView(context).apply {
            this.text = text
            typeface = runCatching { ResourcesCompat.getFont(context, font) }.getOrNull() ?: Typeface.DEFAULT
            setTextSize(TypedValue.COMPLEX_UNIT_PX, screenHeight * size)
            setTextColor(colour)
            maxLines = lines
            ellipsize = android.text.TextUtils.TruncateAt.END
            includeFontPadding = false
        }
        words.addView(line(alert.title, tv.lounge.design.R.font.figtree_semibold, 0.026f, palette.limelight.toArgb(), 2))
        alert.message?.let { message ->
            words.addView(
                line(message, tv.lounge.design.R.font.figtree_regular, 0.0185f, palette.dust.toArgb(), 3),
                LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = pad / 3 },
            )
        }
        card.addView(words, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
        return card to picture
    }

    /** Fetches the picture again and again while the pop-up is up, so a camera is seen as it is now. */
    private fun follow(address: String, into: ImageView, width: Int, shown: Up): Thread =
        thread(name = "lounge-alert-picture", isDaemon = true) {
            while (!shown.gone) {
                fetch(address, width, shown)?.let { bitmap -> main.post { if (!shown.gone) into.setImageBitmap(bitmap) } }
                try {
                    Thread.sleep(PICTURE_EVERY_MS)
                } catch (_: InterruptedException) {
                    return@thread
                }
            }
        }

    /** The picture at [address], no larger than the card needs. Null for anything that is not one. */
    private fun fetch(address: String, width: Int, shown: Up): Bitmap? = runCatching {
        val url = URL(address)
        // A picture comes from a camera or the home's hub, never from the TV's own services.
        if (InetAddress.getAllByName(url.host).any { it.isLoopbackAddress || it.isAnyLocalAddress || NetworkInterface.getByInetAddress(it) != null }) return@runCatching null
        val connection = (url.openConnection() as HttpURLConnection).apply {
            connectTimeout = 4_000
            readTimeout = 4_000
            instanceFollowRedirects = false
        }
        try {
            if (connection.responseCode !in 200..299 || connection.contentLengthLong > MAX_PICTURE_BYTES) return@runCatching null
            val bytes = connection.inputStream.use { read(it, shown) } ?: return@runCatching null
            val size = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size, size)
            if (size.outWidth <= 0 || size.outHeight <= 0) return@runCatching null
            // A camera's frame is far larger than the card, and decoded whole it could fill the TV's memory.
            var sample = 1
            while (size.outWidth / (sample * 2) >= width) sample *= 2
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size, BitmapFactory.Options().apply { inSampleSize = sample })
        } finally {
            connection.disconnect()
        }
    }.getOrNull()

    /** Everything [stream] sends, or null when it is too much, too slow or no longer wanted. */
    private fun read(stream: InputStream, shown: Up): ByteArray? {
        val deadline = SystemClock.elapsedRealtime() + FETCH_MS
        val all = ByteArrayOutputStream()
        val chunk = ByteArray(16 * 1024)
        while (true) {
            val n = stream.read(chunk)
            if (n < 0) return all.toByteArray()
            all.write(chunk, 0, n)
            if (all.size() > MAX_PICTURE_BYTES || shown.gone || SystemClock.elapsedRealtime() > deadline) return null
        }
    }

    private companion object {
        const val PICTURE_EVERY_MS = 1_500L
        const val MAX_PICTURE_BYTES = 6 * 1024 * 1024
        /** How long one picture may take to arrive. */
        const val FETCH_MS = 8_000L
        /** How long the caller waits to hear that the card is up. */
        const val SHOW_WAIT_MS = 3_000L
    }
}
