package tv.lounge.feature.home

import android.app.Activity
import android.content.ContextWrapper
import android.graphics.RenderEffect
import android.graphics.RuntimeShader
import android.os.Handler
import android.os.Looper
import android.service.dreams.DreamService
import android.util.Log
import android.view.FrameMetrics
import android.view.Window
import androidx.annotation.RequiresApi
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.ShaderBrush
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.asComposeRenderEffect
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.layout.layout
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.IntSize
import java.util.Calendar
import kotlin.math.ceil
import kotlinx.coroutines.delay
import tv.lounge.design.LoungeColors
import tv.lounge.design.LoungeTheme
import androidx.compose.runtime.produceState
import androidx.compose.runtime.getValue
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

private const val TAG = "Lounge"
private const val FRAME_NS = 30_000_000L
private const val WINDOW = 45
private const val SLOW_MS = 45f
private const val BACKED_UP_MS = 8f
private const val SETTLE_NS = 2_500_000_000L
/** Frames this slow, three in a row, mean no step down will save the scene, so it ends at once. */
private const val HOPELESS_MS = 500f
private const val RENDERER = 4

/**
 * How a scene is allowed to cost: where it starts, how far it may step down before it is given
 * up on, and the GPU time a frame may take. [memoryPrefix] keeps the screensaver's and the home's
 * memories of a scene apart.
 */
internal class SceneQuality(val memoryPrefix: String, val startDivisor: Float, val maxDivisor: Float, val gpuBudgetMs: Float, val keepResolution: Boolean)

/** A program compiled and checked against everything that will be set on it. */
@RequiresApi(33)
internal class Program(source: String) {
    val shader = RuntimeShader(source)
    private val decls = ShaderSource.uniforms(source).associateBy { it.name }
    private val noise = ShaderSource.wantsNoise(source)

    fun float(name: String, vararg values: Float) {
        val d = decls[name] ?: return
        require(!d.color && !d.isInt && d.size == values.size) { "uniform $name should be a float with ${values.size} components" }
        shader.setFloatUniform(name, values)
    }

    /** A colour, taken as a colour or as three or four plain floats. */
    fun color(name: String, color: Color) {
        val d = decls[name] ?: return
        when {
            d.color -> shader.setColorUniform(name, color.toArgb())
            !d.isInt && d.size == 4 -> shader.setFloatUniform(name, color.red, color.green, color.blue, color.alpha)
            !d.isInt && d.size == 3 -> shader.setFloatUniform(name, color.red, color.green, color.blue)
            else -> throw IllegalArgumentException("uniform $name should be layout(color) uniform half4")
        }
    }

    /** A value the style gives for one of the program's own uniforms. */
    fun value(v: ShaderValue) {
        if (v.name in ShaderSource.PROVIDED) return
        val d = decls[v.name] ?: return
        val c = v.color
        when {
            c != null -> color(v.name, Color(c))
            d.color -> throw IllegalArgumentException("uniform ${v.name} needs a colour like \"#FFA86B\"")
            d.size != v.floats.size -> throw IllegalArgumentException("uniform ${v.name} needs ${d.size} numbers, not ${v.floats.size}")
            d.isInt -> shader.setIntUniform(v.name, IntArray(v.floats.size) { v.floats[it].toInt() })
            else -> shader.setFloatUniform(v.name, v.floats)
        }
    }

    fun provide(width: Float, height: Float, time: Float, hour: Float, colors: LoungeColors) {
        float("resolution", width, height)
        float("time", time)
        float("hour", hour)
        color("velvet", colors.velvet)
        color("horizon", colors.horizon)
        color("curtain", colors.curtain)
        color("houselight", colors.houselight)
        color("ember", colors.ember)
        color("signal", colors.signal)
        color("dust", colors.dust)
        if (noise) shader.setInputShader(ShaderSource.NOISE, NoiseTexture.shader)
    }
}

/** Compiles the program and sets everything once, so a mistake shows now and not in the middle of a draw. */
@RequiresApi(33)
internal fun compileScene(style: ShaderStyle, colors: LoungeColors): Result<Program> = runCatching {
    ShaderSource.problem(style.source)?.let { throw IllegalArgumentException(it) }
    Program(style.source).apply {
        provide(640f, 360f, 0f, 12f, colors)
        style.values.forEach { value(it) }
    }
}

/**
 * A scene drawn at a fraction of the screen's resolution and scaled up, with the finishing pass
 * over it at the screen's own size, thirty times a second. When the GPU cannot keep up the
 * fraction gets smaller, and a scene too heavy even at [SceneQuality.maxDivisor] is given up on.
 * While [paused] the clock stops and nothing is drawn. [prepareFinish] sets the finishing pass's
 * uniforms for the screen's size, and runs again when [finishKey] changes.
 */
@RequiresApi(33)
@Composable
internal fun SceneSurface(
    style: ShaderStyle,
    quality: SceneQuality,
    finish: RuntimeShader,
    slow: Boolean,
    paused: () -> Boolean,
    onUnusable: (String) -> Unit,
    finishKey: Any?,
    prepareFinish: (width: Float, height: Float) -> Unit,
) {
    val colors = LoungeTheme.colors
    // A pushed or plugin program can take long enough to compile to stall the screen.
    val compiling by produceState<Result<Program>?>(null, style) { value = withContext(Dispatchers.Default) { compileScene(style, colors) } }
    val compiled = compiling ?: return
    val program = compiled.getOrNull()
    if (program == null) {
        LaunchedEffect(style.id) { onUnusable(compiled.exceptionOrNull()?.message ?: "the program does not compile") }
        return
    }
    val context = LocalContext.current
    val memory = remember { context.getSharedPreferences("screensavers", android.content.Context.MODE_PRIVATE) }
    val time = remember { mutableFloatStateOf(0f) }
    val hour = remember { mutableFloatStateOf(localHour()) }
    // Where this program last held its frame rate on this TV, so it starts there; a new program
    // or a new renderer starts afresh.
    val start = quality.startDivisor
    val memoryKey = remember(style) { "$RENDERER:${quality.memoryPrefix}:${style.id}:${style.source.hashCode()}" }
    val divisor = remember(memoryKey) { mutableFloatStateOf(memory.getFloat(memoryKey, start).coerceIn(start, quality.maxDivisor)) }
    val gpu = remember { GpuTimes() }
    val view = LocalView.current
    DisposableEffect(view) {
        val window = windowOf(view.context)
        val listener = Window.OnFrameMetricsAvailableListener { _, metrics, _ -> gpu.add(metrics.getMetric(FrameMetrics.GPU_DURATION) / 1_000_000f) }
        window?.addOnFrameMetricsAvailableListener(listener, Handler(Looper.getMainLooper()))
        onDispose { window?.removeOnFrameMetricsAvailableListener(listener) }
    }

    LaunchedEffect(Unit) {
        while (true) {
            delay(5 * 60_000L)
            hour.floatValue = localHour()
        }
    }
    LaunchedEffect(program) {
        val speed = if (slow) 0.35f else 1f
        var started = withFrameNanos { it }
        var last = started
        var settle = started + SETTLE_NS
        val gaps = FloatArray(WINDOW)
        val lateness = FloatArray(WINDOW)
        var n = 0
        var windows = 0
        var reportAt = 1
        var pausedAt = 0L
        var hopeless = 0
        while (true) {
            if (paused()) {
                if (pausedAt == 0L) pausedAt = System.nanoTime()
                delay(200)
                continue
            }
            if (pausedAt != 0L) {
                // The scene carries on from where it stopped, and the measurements start afresh.
                val away = System.nanoTime() - pausedAt
                started += away
                last += away
                settle = System.nanoTime() + SETTLE_NS
                pausedAt = 0L
                n = 0
                gpu.clear()
            }
            withFrameNanos { now ->
                if (now - last >= FRAME_NS) {
                    if (now > settle) {
                        lateness[n] = (System.nanoTime() - now) / 1_000_000f
                        gaps[n++] = (now - last) / 1_000_000f
                        hopeless = if (gaps[n - 1] > HOPELESS_MS) hopeless + 1 else 0
                    }
                    last = now
                    time.floatValue = (now - started) / 1_000_000_000f * speed
                }
            }
            if (hopeless >= 3) {
                onUnusable("it is too heavy for this TV")
                return@LaunchedEffect
            }
            if (n < WINDOW) continue
            n = 0
            windows++
            val median = gaps.sorted()[WINDOW / 2]
            val late = lateness.sorted()[WINDOW / 2]
            val gpuMs = gpu.median()
            if (windows == reportAt || windows % 60 == 0) {
                Log.i(TAG, "scene: ${quality.memoryPrefix} ${style.id} at 1/${divisor.floatValue.toInt()}, $median ms a frame, $late ms late, GPU ${gpuMs ?: "unknown"} ms")
            }
            // Frames starting late after vsync mean the GPU is full; the GPU budget keeps it well short of that.
            if (median > SLOW_MS || late > BACKED_UP_MS || (gpuMs != null && gpuMs > quality.gpuBudgetMs)) {
                if (divisor.floatValue >= quality.maxDivisor) {
                    onUnusable("it is too heavy for this TV")
                    return@LaunchedEffect
                }
                divisor.floatValue += 1f
                if (quality.keepResolution) memory.edit().putFloat(memoryKey, divisor.floatValue).apply()
                gpu.clear()
                settle = last + SETTLE_NS
                reportAt = windows + 1
            }
        }
    }

    val sceneBrush = remember(program) { ShaderBrush(program.shader) }
    var full by remember { mutableStateOf(IntSize.Zero) }
    // An effect keeps the uniforms it was made with, so it is made again when they change.
    val effect = remember(finish, full, finishKey) {
        if (full == IntSize.Zero) {
            null
        } else {
            prepareFinish(full.width.toFloat(), full.height.toFloat())
            RenderEffect.createRuntimeShaderEffect(finish, "scene").asComposeRenderEffect()
        }
    }
    Box(
        Modifier
            .fillMaxSize()
            .onSizeChanged { full = it }
            .graphicsLayer {
                renderEffect = effect
                compositingStrategy = CompositingStrategy.Offscreen
            },
    ) {
        Canvas(
            Modifier
                .fillMaxSize()
                .layout { measurable, constraints ->
                    val d = divisor.floatValue
                    val placeable = measurable.measure(
                        Constraints.fixed(ceil(constraints.maxWidth / d).toInt(), ceil(constraints.maxHeight / d).toInt()),
                    )
                    layout(constraints.maxWidth, constraints.maxHeight) { placeable.place(0, 0) }
                }
                .graphicsLayer {
                    scaleX = divisor.floatValue
                    scaleY = divisor.floatValue
                    transformOrigin = TransformOrigin(0f, 0f)
                    compositingStrategy = CompositingStrategy.Offscreen
                },
        ) {
            program.provide(size.width, size.height, time.floatValue, hour.floatValue, colors)
            drawRect(sceneBrush)
        }
    }
}

/** The GPU time of a window's recent frames, from FrameMetrics. */
@RequiresApi(33)
private class GpuTimes {
    private val values = FloatArray(60)
    private var count = 0
    private var next = 0

    @Synchronized fun add(ms: Float) {
        values[next] = ms
        next = (next + 1) % values.size
        if (count < values.size) count++
    }

    @Synchronized fun median(): Float? = if (count < 20) null else values.copyOf(count).sorted()[count / 2]

    @Synchronized fun clear() {
        count = 0
        next = 0
    }
}

private fun windowOf(context: android.content.Context): Window? {
    var c: android.content.Context? = context
    while (c != null) {
        when (c) {
            is Activity -> return c.window
            is DreamService -> return c.window
            is ContextWrapper -> c = c.baseContext
            else -> return null
        }
    }
    return null
}

private fun localHour(): Float {
    val c = Calendar.getInstance()
    return c.get(Calendar.HOUR_OF_DAY) + c.get(Calendar.MINUTE) / 60f
}
