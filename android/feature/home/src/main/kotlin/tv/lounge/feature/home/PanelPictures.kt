package tv.lounge.feature.home

import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.ColorMatrix
import android.graphics.ColorMatrixColorFilter
import android.graphics.ImageDecoder
import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.RectF
import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.viewinterop.AndroidView
import java.io.File
import java.util.concurrent.Executors
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import tv.lounge.platform.PanelSize

private const val FADE_MS = 2_400L
/** The push-in over a slide: 6 %, as the Compose screensaver does it. */
private const val PUSH_IN = 0.06f

/**
 * The screensaver's pictures on a surface of the panel's own size. Most TVs draw their apps at
 * 1080p on a 2160p panel; a surface with a fixed buffer of the panel's pixels is composed at
 * those pixels, so a 2160p picture shows every one of them. The crossfade, the stepped push-in,
 * the drift and the house lights are drawn here; the clock and the captions stay in Compose over it.
 */
@Composable
fun PanelPictures(
    slide: AmbientSlide?,
    slideIndex: Int,
    brightness: Float,
    drift: IntOffset,
    panel: PanelSize,
    window: PanelSize,
    stepMs: Long,
    reducedMotion: Boolean,
    modifier: Modifier = Modifier,
) {
    val renderer = remember(panel) { PictureRenderer(panel, window, stepMs, reducedMotion) }
    AndroidView(
        factory = { context: Context ->
            SurfaceView(context).apply {
                holder.setFixedSize(panel.width, panel.height)
                holder.addCallback(renderer)
            }
        },
        modifier = modifier,
    )
    LaunchedEffect(renderer, slide?.art) { renderer.show(slide, slideIndex) }
    LaunchedEffect(renderer, brightness) { renderer.setBrightness(brightness) }
    LaunchedEffect(renderer, drift) { renderer.setDrift(drift) }
    DisposableEffect(renderer) { onDispose { renderer.close() } }
}

/** One picture on the wall: its bitmap, when it arrived, and where its push-in is anchored. */
private class Hung(val slide: AmbientSlide, val bitmap: Bitmap, val shownAt: Long, val originX: Float, val originY: Float)

/**
 * Draws on one thread of its own: decodes arrive on it, the surface comes and goes on it, and
 * every frame is drawn on it, so nothing is shared.
 */
private class PictureRenderer(private val panel: PanelSize, window: PanelSize, private val stepMs: Long, private val reducedMotion: Boolean) : SurfaceHolder.Callback {
    private val thread = Executors.newSingleThreadExecutor { r -> Thread(r, "lounge-pictures") }
    private val dispatcher = thread.asCoroutineDispatcher()
    private val scope = CoroutineScope(SupervisorJob() + dispatcher)
    /** Design pixels to panel pixels: 2 on a 2160p panel drawn at 1080p. */
    private val k = if (window.width > 0) panel.width.toFloat() / window.width else 1f
    private var holder: SurfaceHolder? = null
    private var loop: Job? = null
    private var current: Hung? = null
    private var previous: Hung? = null
    private var fadeStart = 0L
    private var brightness = 0.6f
    private var drift = IntOffset.Zero
    private var wanted: AmbientSlide? = null
    private val paint = Paint(Paint.FILTER_BITMAP_FLAG or Paint.ANTI_ALIAS_FLAG)
    private val shadow = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.BLACK }
    private val matrix = Matrix()

    fun show(slide: AmbientSlide?, index: Int) {
        scope.launch {
            wanted = slide
            if (slide == null) {
                previous = current?.also { fadeStart = now() }
                current = null
                return@launch
            }
            if (current?.slide?.art == slide.art) return@launch
            val bitmap = withContext(Dispatchers.IO) { decode(slide.art) } ?: return@launch
            // A later slide may have been asked for while this one decoded.
            if (wanted?.art != slide.art) {
                bitmap.recycle()
                return@launch
            }
            previous?.bitmap?.recycle()
            previous = current
            val (ox, oy) = if (slide.framed) 0.5f to 0.5f else if (index % 2 == 0) 0.35f to 0.4f else 0.65f to 0.55f
            current = Hung(slide, bitmap, now(), ox, oy)
            fadeStart = now()
            draw()
        }
    }

    fun setBrightness(value: Float) {
        scope.launch {
            brightness = value
            draw()
        }
    }

    fun setDrift(value: IntOffset) {
        scope.launch {
            drift = value
            draw()
        }
    }

    fun close() {
        scope.cancel()
        thread.shutdown()
    }

    override fun surfaceCreated(h: SurfaceHolder) {
        scope.launch {
            holder = h
            loop = launch {
                while (true) {
                    draw()
                    delay(if (fading()) 16L else stepMs)
                }
            }
        }
    }

    override fun surfaceChanged(h: SurfaceHolder, format: Int, width: Int, height: Int) {
        scope.launch { draw() }
    }

    override fun surfaceDestroyed(h: SurfaceHolder) {
        // The surface is gone once this returns; the drawing thread must be past its last frame.
        val done = java.util.concurrent.CountDownLatch(1)
        scope.launch {
            loop?.cancel()
            loop = null
            holder = null
            done.countDown()
        }
        done.await()
    }

    private fun now() = System.currentTimeMillis()

    private fun fading() = previous != null && !reducedMotion && now() - fadeStart < FADE_MS

    private fun draw() {
        val h = holder ?: return
        val canvas = try {
            h.lockHardwareCanvas()
        } catch (_: IllegalStateException) {
            null
        } ?: return
        try {
            canvas.drawColor(Color.BLACK)
            val t = if (reducedMotion) 1f else ((now() - fadeStart).toFloat() / FADE_MS).coerceIn(0f, 1f)
            val old = previous
            if (old != null && t < 1f) {
                drawHung(canvas, old, 1f)
            } else if (old != null) {
                old.bitmap.recycle()
                previous = null
            }
            current?.let { drawHung(canvas, it, if (old != null) eased(t) else 1f) }
        } finally {
            h.unlockCanvasAndPost(canvas)
        }
    }

    private fun eased(t: Float): Float = 1f - (1f - t) * (1f - t) * (1f - t)

    /** The picture at its push-in and drift, lit by the house lights, framed if it is a painting. */
    private fun drawHung(canvas: Canvas, hung: Hung, alpha: Float) {
        val bitmap = hung.bitmap
        val pw = panel.width.toFloat()
        val ph = panel.height.toFloat()
        // The push-in moves in steps too small to see, and holds between them.
        val elapsed = if (reducedMotion) 0L else (now() - hung.shownAt) / stepMs * stepMs
        val scale = 1f + PUSH_IN * (elapsed.toFloat() / SLIDE_MS).coerceIn(0f, 1f)
        val dx = drift.x * k
        val dy = drift.y * k
        paint.alpha = (alpha * 255f).toInt().coerceIn(0, 255)
        paint.colorFilter = houseLights(brightness)
        if (hung.slide.framed) {
            // Whole, in its own shape, with air around it and a shadow stepping out and fading.
            val padX = 160f * k
            val padY = 100f * k
            val fit = minOf((pw - 2 * padX) / bitmap.width, (ph - 2 * padY) / bitmap.height)
            val w = bitmap.width * fit * scale
            val hgt = bitmap.height * fit * scale
            val left = (pw - w) / 2f + dx
            val top = (ph - hgt) / 2f + dy
            val step = 5f * k
            for (i in 1..8) {
                shadow.alpha = (0.16f * (1f - (i - 1) / 8f) * 255f * alpha).toInt()
                val grow = step * i
                canvas.drawRect(RectF(left - grow, top - grow + step * 2, left + w + grow, top + hgt + grow), shadow)
            }
            matrix.reset()
            matrix.setScale(fit * scale, fit * scale)
            matrix.postTranslate(left, top)
            canvas.drawBitmap(bitmap, matrix, paint)
        } else {
            val cover = maxOf(pw / bitmap.width, ph / bitmap.height)
            val w = bitmap.width * cover
            val hgt = bitmap.height * cover
            matrix.reset()
            matrix.setScale(cover, cover)
            matrix.postTranslate((pw - w) / 2f, (ph - hgt) / 2f)
            matrix.postScale(scale, scale, pw * hung.originX, ph * hung.originY)
            matrix.postTranslate(dx, dy)
            canvas.drawBitmap(bitmap, matrix, paint)
        }
    }

    private var lightsFor = -1f
    private var lights: ColorMatrixColorFilter? = null

    /** The house lights on the picture: saturation first, then brightness, so blacks stay black. */
    private fun houseLights(brightness: Float): ColorMatrixColorFilter {
        if (lightsFor != brightness || lights == null) {
            val m = ColorMatrix().apply { setSaturation(0.95f) }
            val b = brightness
            m.postConcat(ColorMatrix(floatArrayOf(b, 0f, 0f, 0f, 0f, 0f, b, 0f, 0f, 0f, 0f, 0f, b, 0f, 0f, 0f, 0f, 0f, 1f, 0f)))
            lights = ColorMatrixColorFilter(m)
            lightsFor = brightness
        }
        return lights!!
    }

    /** The prepared file as a hardware bitmap: it lives in graphics memory, not the heap. */
    private fun decode(path: String): Bitmap? = try {
        ImageDecoder.decodeBitmap(ImageDecoder.createSource(File(path))) { decoder, _, _ ->
            decoder.allocator = ImageDecoder.ALLOCATOR_HARDWARE
            decoder.isMutableRequired = false
        }
    } catch (e: Exception) {
        android.util.Log.w("Lounge", "screensaver: could not decode $path: $e")
        null
    }
}
