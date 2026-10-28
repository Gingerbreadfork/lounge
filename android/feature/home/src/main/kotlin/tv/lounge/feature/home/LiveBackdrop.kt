package tv.lounge.feature.home

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapShader
import android.graphics.Canvas
import android.graphics.HardwareRenderer
import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.PixelFormat
import android.graphics.RenderNode
import android.graphics.RuntimeShader
import android.graphics.Shader
import android.hardware.HardwareBuffer
import android.media.Image
import android.media.ImageReader
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.annotation.RequiresApi
import androidx.compose.animation.Animatable
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.VectorConverter
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.key
import androidx.compose.runtime.remember
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.viewinterop.AndroidView
import java.util.concurrent.Executors
import kotlin.math.ceil
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.launch
import tv.lounge.design.BlueNoise
import tv.lounge.design.Dimens
import tv.lounge.design.FollowLight
import tv.lounge.design.LightState
import tv.lounge.design.LoungeColors
import tv.lounge.design.LoungeTheme
import tv.lounge.design.rememberIdleLevel

private const val TAG = "Lounge"
private const val FRAME_MS = 33L
private const val WINDOW = 45
private const val SETTLE_MS = 2_500L
/**
 * Nothing is measured until the app has run this long: its own start keeps the GPU busy, and a
 * step down taken then would be remembered for good.
 */
private const val WARM_MS = 15_000L
private const val SLOW_MS = 40f
/** Frames this slow, three in a row, mean no step down will save the scene, so it ends at once. */
private const val HOPELESS_MS = 500f
private const val START_DIVISOR = 2
private const val MAX_DIVISOR = 3
private const val RENDERER = 4

// The room's light over a live scene, drawn at the screen's own size over the scaled-up scene:
// the scene dimmed and held under a ceiling so the tiles stay the brightest things in the room,
// the focused tile's spill and the playbill's light as the still field draws them, and one level
// of blue noise a pixel against banding.
private const val LIVE_FINISH_SKSL = """
uniform shader scene;
uniform shader blue;
uniform float level;
uniform float2 spill;
uniform float spillR;
uniform float spillAlpha;
layout(color) uniform half4 spillColor;
uniform float2 bill;
uniform float2 billRadii;
uniform float billAlpha;
layout(color) uniform half4 billColor;

half4 main(float2 frag) {
    half3 c = scene.eval(frag).rgb * half(level);
    half m = max(c.r, max(c.g, c.b));
    if (m > 0.5) {
        c *= (0.5 + (m - 0.5) * 0.3) / m;
    }
    float ds = length(frag - spill) / spillR;
    float s = pow(clamp(1.0 - ds / 0.7, 0.0, 1.0), 1.8) * spillAlpha;
    c = mix(c, spillColor.rgb, half(s));
    float2 e = (frag - bill) / billRadii;
    float bl = pow(clamp(1.0 - length(e), 0.0, 1.0), 1.6) * billAlpha;
    c = mix(c, billColor.rgb, half(bl));
    c += blue.eval(frag).r;
    return half4(c, 1.0);
}
"""

/** The lights for one frame, in window pixels. */
private class Lights(
    val level: Float,
    val spill: Offset,
    val spillRadius: Float,
    val spillAlpha: Float,
    val spillColor: Color,
    val bill: Offset?,
    val billRadii: Offset,
    val billAlpha: Float,
    val billColor: Color,
)

/**
 * A live background: one of the abstract scenes behind the room instead of the still light
 * field, with the same lights over it. It is drawn on a surface of its own behind the window, so
 * the home itself does not redraw for it; it freezes with the field's clock, after a minute without
 * input or under an opaque overlay, and gives way to the still field when it is too heavy.
 */
@RequiresApi(33)
@Composable
fun LiveBackdrop(style: ShaderStyle, state: LightState, onUnusable: (String) -> Unit, modifier: Modifier = Modifier) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    val context = LocalContext.current
    val density = LocalDensity.current
    DisposableEffect(state) {
        state.fieldDrawsLights = true
        onDispose { state.fieldDrawsLights = false }
    }
    // The spill moves with focus over `travel` and takes the tile's colour over `curtain`, as the
    // still field's does; the first light is placed, not swept into.
    val spillCenter = remember { Animatable(Offset(960f, 540f), Offset.VectorConverter) }
    val spillColor = remember { Animatable(colors.houselight) }
    FollowLight(state, spillCenter, motion)
    LaunchedEffect(motion.reducedMotion) {
        var placed = false
        snapshotFlow { state.color }.collectLatest { target ->
            if (motion.reducedMotion || !placed) spillColor.snapTo(target) else spillColor.animateTo(target, motion.curtain())
            placed = true
        }
    }
    val idleLevel = rememberIdleLevel(state.idle)
    val activeLevel = animateFloatAsState(if (state.active) 1f else 0f, if (state.active) motion.snap() else motion.settle(), label = "liveSpillActive")
    val spillAlpha = 0.20f * colors.spillScale
    val billAlpha = 0.22f * colors.spillScale
    val spillRadius = with(density) { Dimens.spillRadius.toPx() }

    val renderer = remember(style, colors) { BackdropRenderer(context, style, colors, onUnusable) }
    LaunchedEffect(renderer) {
        snapshotFlow {
            val level = idleLevel.value * state.warmth
            val bill = state.bill
            Lights(
                level = 0.72f * (0.6f + 0.4f * level),
                spill = spillCenter.value,
                spillRadius = spillRadius,
                spillAlpha = spillAlpha * level * activeLevel.value,
                spillColor = spillColor.value,
                bill = bill?.center,
                billRadii = if (bill != null) Offset(bill.rx, bill.ry) else Offset(1f, 1f),
                billAlpha = if (bill != null) billAlpha * level * bill.strength().coerceIn(0f, 1f) else 0f,
                billColor = bill?.color ?: Color.Black,
            )
        }.collect { renderer.lights = it }
    }
    LaunchedEffect(renderer) { snapshotFlow { state.paused }.collect { renderer.paused = it } }
    // A new scene gets a new surface: the callbacks that start a renderer fire once per surface.
    key(renderer) {
        AndroidView(
            factory = { c: Context -> SurfaceView(c).apply { holder.addCallback(renderer) } },
            modifier = modifier.fillMaxSize(),
        )
    }
    DisposableEffect(renderer) { onDispose { renderer.close() } }
}

/**
 * Draws the scene on its own thread, thirty times a second, at a fraction of the window's size
 * in a layer of its own, then the lights and the dither over the scaled-up layer at full size.
 * When frames take too long the fraction gets smaller, and a scene too heavy even at a third is
 * given up on.
 */
@RequiresApi(33)
private class BackdropRenderer(context: Context, private val style: ShaderStyle, private val colors: LoungeColors, private val onUnusable: (String) -> Unit) : SurfaceHolder.Callback {
    private val thread = Executors.newSingleThreadExecutor { r -> Thread(r, "lounge-backdrop") }
    private val scope = CoroutineScope(SupervisorJob() + thread.asCoroutineDispatcher())
    private val main = Handler(Looper.getMainLooper())
    private val memory = context.getSharedPreferences("screensavers", Context.MODE_PRIVATE)
    private val memoryKey = "$RENDERER:live:${style.id}:${style.source.hashCode()}"
    @Volatile var lights: Lights? = null
    @Volatile var paused: Boolean = false
    /** Held while a frame is drawn, so the surface is never used after it is destroyed. */
    private val drawing = java.util.concurrent.locks.ReentrantLock()
    @Volatile private var holder: SurfaceHolder? = null
    private var loop: Job? = null
    private var viewWidth = 0
    private var viewHeight = 0
    private var divisor = runCatching { memory.getInt(memoryKey, START_DIVISOR) }.getOrDefault(START_DIVISOR).coerceIn(START_DIVISOR, MAX_DIVISOR)
    private var program: Program? = null
    private val finish = RuntimeShader(LIVE_FINISH_SKSL)
    private val paint = Paint()
    private val scenePaint = Paint()
    // The scene is drawn small into a buffer the GPU can sample, and the finish reads it scaled.
    private val sceneNode = RenderNode("lounge-scene")
    private var sceneRenderer: HardwareRenderer? = null
    private var sceneReader: ImageReader? = null
    private var sceneWidth = 0
    private var sceneHeight = 0
    private var shownImage: Image? = null
    private var lastImage: Image? = null
    private var sceneShader: Shader? = null
    private val sceneMatrix = Matrix()
    private var given = false

    override fun surfaceCreated(h: SurfaceHolder) {
        holder = h
    }

    override fun surfaceChanged(h: SurfaceHolder, format: Int, width: Int, height: Int) {
        scope.launch {
            if (viewWidth == 0) {
                viewWidth = width
                viewHeight = height
                Log.i(TAG, "backdrop: ${style.id} surface ${width}x${height}, drawing at 1/$divisor")
                start()
            }
        }
    }

    override fun surfaceDestroyed(h: SurfaceHolder) {
        // Waits for at most the frame in flight; the loop finds no holder afterwards.
        drawing.lock()
        try {
            holder = null
        } finally {
            drawing.unlock()
        }
    }

    fun close() {
        scope.cancel()
        thread.execute {
            drawing.lock()
            try {
                releaseSceneTarget()
            } finally {
                drawing.unlock()
            }
        }
        thread.shutdown()
    }

    private fun giveUp(why: String) {
        if (given) return
        given = true
        Log.w(TAG, "backdrop: ${style.id} given up: $why")
        loop?.cancel()
        loop = null
        main.post { onUnusable(why) }
    }

    private fun start() {
        val compiled = compileScene(style, colors)
        program = compiled.getOrNull()
        if (program == null) {
            giveUp(compiled.exceptionOrNull()?.message ?: "the program does not compile")
            return
        }
        finish.setInputShader("blue", BitmapShader(BlueNoise.bitmap, Shader.TileMode.REPEAT, Shader.TileMode.REPEAT))
        loop = scope.launch {
            var started = System.nanoTime()
            val warm = android.os.Process.getStartUptimeMillis() + WARM_MS - android.os.SystemClock.uptimeMillis()
            var settleUntil = System.currentTimeMillis() + maxOf(SETTLE_MS, warm)
            val frames = FloatArray(WINDOW)
            var n = 0
            var windows = 0
            var pausedAt = 0L
            var reported = false
            var hopeless = 0
            while (true) {
                if (paused) {
                    if (pausedAt == 0L) pausedAt = System.nanoTime()
                    delay(200)
                    continue
                }
                if (pausedAt != 0L) {
                    started += System.nanoTime() - pausedAt
                    pausedAt = 0L
                    settleUntil = System.currentTimeMillis() + SETTLE_MS
                    n = 0
                }
                val t0 = System.nanoTime()
                draw((t0 - started) / 1_000_000_000f)
                val took = (System.nanoTime() - t0) / 1_000_000f
                if (System.currentTimeMillis() > settleUntil) {
                    frames[n++] = took
                    hopeless = if (took > HOPELESS_MS) hopeless + 1 else 0
                    if (hopeless >= 3) {
                        giveUp("it is too heavy for this TV")
                        return@launch
                    }
                }
                if (n >= WINDOW) {
                    n = 0
                    windows++
                    val median = frames.sorted()[WINDOW / 2]
                    if (!reported || windows % 60 == 0) {
                        Log.i(TAG, "backdrop: ${style.id} at 1/$divisor, $median ms a frame")
                        reported = true
                    }
                    // A frame that waits for a buffer is a GPU that has fallen behind.
                    if (median > SLOW_MS) {
                        if (divisor >= MAX_DIVISOR) {
                            giveUp("it is too heavy for this TV")
                            return@launch
                        }
                        divisor++
                        memory.edit().putInt(memoryKey, divisor).apply()
                        settleUntil = System.currentTimeMillis() + SETTLE_MS
                        reported = false
                    }
                }
                delay((FRAME_MS - took.toLong()).coerceIn(1L, FRAME_MS))
            }
        }
    }

    private fun draw(time: Float) {
        drawing.lock()
        try {
            drawLocked(time)
        } finally {
            drawing.unlock()
        }
    }

    /** Draws the scene at [sw] by [sh] into a buffer the GPU can sample, as a shader scaled to the window. */
    private fun renderScene(p: Program, sw: Int, sh: Int, time: Float): Shader? {
        if (sw != sceneWidth || sh != sceneHeight) {
            releaseSceneTarget()
            val reader = ImageReader.newInstance(sw, sh, PixelFormat.RGBA_8888, 3, HardwareBuffer.USAGE_GPU_SAMPLED_IMAGE or HardwareBuffer.USAGE_GPU_COLOR_OUTPUT)
            sceneReader = reader
            sceneRenderer = HardwareRenderer().apply {
                isOpaque = true
                setLightSourceGeometry(0f, 0f, 800f, 20f)
                setLightSourceAlpha(0f, 0f)
                setSurface(reader.surface)
                setContentRoot(sceneNode)
            }
            sceneWidth = sw
            sceneHeight = sh
        }
        p.provide(sw.toFloat(), sh.toFloat(), time, 12f, colors)
        scenePaint.shader = p.shader
        sceneNode.setPosition(0, 0, sw, sh)
        val sc = sceneNode.beginRecording(sw, sh)
        try {
            sc.drawRect(0f, 0f, sw.toFloat(), sh.toFloat(), scenePaint)
        } finally {
            sceneNode.endRecording()
        }
        sceneRenderer?.createRenderRequest()?.setWaitForPresent(true)?.syncAndDraw()
        // One frame is drawn per acquire, and two stay held, so the third slot is always this one.
        val image = sceneReader?.acquireNextImage() ?: return sceneShader
        val buffer = image.hardwareBuffer
        val bitmap = buffer?.let { Bitmap.wrapHardwareBuffer(it, null) }
        buffer?.close()
        if (bitmap == null) {
            image.close()
            return sceneShader
        }
        // The frame before stays open until this one is posted, so a buffer is never refilled
        // while the GPU may still read it.
        lastImage?.close()
        lastImage = shownImage
        shownImage = image
        sceneMatrix.setScale(divisor.toFloat(), divisor.toFloat())
        return BitmapShader(bitmap, Shader.TileMode.CLAMP, Shader.TileMode.CLAMP).apply {
            filterMode = BitmapShader.FILTER_MODE_LINEAR
            setLocalMatrix(sceneMatrix)
            sceneShader = this
        }
    }

    private fun releaseSceneTarget() {
        lastImage?.close()
        lastImage = null
        shownImage?.close()
        shownImage = null
        sceneShader = null
        sceneRenderer?.destroy()
        sceneRenderer = null
        sceneReader?.close()
        sceneReader = null
        sceneWidth = 0
        sceneHeight = 0
    }

    private fun drawLocked(time: Float) {
        val h = holder ?: return
        val p = program ?: return
        val l = lights ?: return
        val canvas: Canvas = try {
            h.lockHardwareCanvas()
        } catch (e: Exception) {
            Log.w(TAG, "backdrop: no canvas: $e")
            null
        } ?: return
        try {
            val w = canvas.width
            val hgt = canvas.height
            val d = divisor.toFloat()
            val sw = ceil(w / d).toInt().coerceAtLeast(1)
            val sh = ceil(hgt / d).toInt().coerceAtLeast(1)
            val scene = renderScene(p, sw, sh, time) ?: return
            finish.setInputShader("scene", scene)
            finish.setFloatUniform("level", l.level)
            finish.setFloatUniform("spill", l.spill.x, l.spill.y)
            finish.setFloatUniform("spillR", l.spillRadius)
            finish.setFloatUniform("spillAlpha", l.spillAlpha)
            finish.setColorUniform("spillColor", android.graphics.Color.valueOf(l.spillColor.red, l.spillColor.green, l.spillColor.blue, 1f))
            val bill = l.bill
            if (bill != null) {
                finish.setFloatUniform("bill", bill.x, bill.y)
                finish.setFloatUniform("billRadii", l.billRadii.x.coerceAtLeast(1f), l.billRadii.y.coerceAtLeast(1f))
                finish.setFloatUniform("billAlpha", l.billAlpha)
                finish.setColorUniform("billColor", android.graphics.Color.valueOf(l.billColor.red, l.billColor.green, l.billColor.blue, 1f))
            } else {
                finish.setFloatUniform("bill", -10000f, -10000f)
                finish.setFloatUniform("billRadii", 1f, 1f)
                finish.setFloatUniform("billAlpha", 0f)
                finish.setColorUniform("billColor", android.graphics.Color.valueOf(0f, 0f, 0f, 1f))
            }
            paint.shader = finish
            canvas.drawRect(0f, 0f, w.toFloat(), hgt.toFloat(), paint)
        } catch (e: Exception) {
            Log.w(TAG, "backdrop: draw failed: $e")
            giveUp("it does not draw: $e")
        } finally {
            h.unlockCanvasAndPost(canvas)
        }
    }
}
