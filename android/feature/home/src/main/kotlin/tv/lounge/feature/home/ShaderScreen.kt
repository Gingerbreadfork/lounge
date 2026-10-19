package tv.lounge.feature.home

import android.app.Activity
import android.content.ContextWrapper
import android.graphics.RuntimeShader
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.service.dreams.DreamService
import android.util.Log
import android.view.FrameMetrics
import android.view.Window
import androidx.annotation.RequiresApi
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.ShaderBrush
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.layout.layout
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.unit.Constraints
import java.util.Calendar
import kotlin.math.ceil
import kotlinx.coroutines.delay
import tv.lounge.design.LoungeColors
import tv.lounge.design.LoungeTheme

private const val TAG = "Lounge"
private const val FRAME_NS = 30_000_000L
private const val WINDOW = 45
private const val SLOW_MS = 45f
private const val BACKED_UP_MS = 8f
private const val GPU_BUDGET_MS = 24f
private const val START_DIVISOR = 3f
private const val MAX_DIVISOR = 6f
private const val SETTLE_NS = 2_500_000_000L
private const val RENDERER = 2

// Lounge's finishing pass, drawn together with every scene: the room's vignette, the house lights,
// a ceiling on highlights so nothing glares at night, and dither so dark gradients do not band.
private const val FINISH_SKSL = """
uniform shader scene;
uniform float2 resolution;
uniform float brightness;

float hash(float2 p) {
    float3 p3 = fract(float3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

half4 main(float2 coord) {
    half3 c = scene.eval(coord).rgb;
    float2 v = coord / resolution - 0.5;
    v.x *= resolution.x / resolution.y;
    c *= half(brightness * (1.0 - 0.32 * smoothstep(0.55, 1.1, length(v))));
    half m = max(c.r, max(c.g, c.b));
    if (m > 0.72) {
        c *= (0.72 + (m - 0.72) * 0.45) / m;
    }
    c += half3((hash(coord) - 0.5) * (2.0 / 255.0));
    return half4(c, 1.0);
}
"""

/**
 * An abstract screensaver: the program drawn on the GPU behind the clock and its name. The house
 * lights go down a step after half an hour, as they do for pictures.
 */
@Composable
fun ShaderScreensaver(style: ShaderStyle, onUnusable: (String) -> Unit, preview: Boolean = false) {
    val motion = LoungeTheme.motion
    val clock = rememberAmbientClock()
    val drift = rememberAmbientDrift()
    val lights = remember { mutableFloatStateOf(1f) }
    LaunchedEffect(Unit) {
        delay(LATE_AFTER_MS)
        repeat(LATE_DIM_STEPS) { i ->
            lights.floatValue = 1f - 0.2f * (i + 1) / LATE_DIM_STEPS
            delay(LATE_DIM_MS / LATE_DIM_STEPS)
        }
    }
    Box(Modifier.fillMaxSize().background(Color.Black)) {
        if (Build.VERSION.SDK_INT >= 33) {
            ShaderSurface(style, lights, motion.reducedMotion, !preview, onUnusable)
        } else {
            LaunchedEffect(Unit) { onUnusable("abstract screensavers need Android 13") }
        }
        AmbientCaption(clock, style.name, style.origin, drift)
    }
}

/** A program compiled and checked against everything that will be set on it. */
@RequiresApi(33)
private class Program(source: String) {
    val shader = RuntimeShader(source)
    private val decls = ShaderSource.uniforms(source).associateBy { it.name }

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
    }
}

/** Compiles the program and sets everything once, so a mistake shows now and not in the middle of a draw. */
@RequiresApi(33)
private fun compile(style: ShaderStyle, colors: LoungeColors): Result<Program> = runCatching {
    ShaderSource.problem(style.source)?.let { throw IllegalArgumentException(it) }
    Program(style.source).apply {
        provide(640f, 360f, 0f, 12f, colors)
        style.values.forEach { value(it) }
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

/**
 * The program and Lounge's finishing pass at a fraction of the screen's resolution, upscaled.
 * It draws thirty times a second; when the GPU cannot keep up the fraction gets smaller, and a
 * program too heavy even then is given up on.
 */
@RequiresApi(33)
@Composable
private fun ShaderSurface(style: ShaderStyle, lights: State<Float>, slow: Boolean, keepResolution: Boolean, onUnusable: (String) -> Unit) {
    val colors = LoungeTheme.colors
    val compiled = remember(style) { compile(style, colors) }
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
    val memoryKey = remember(style) { "$RENDERER:${style.id}:${style.source.hashCode()}" }
    val divisor = remember(memoryKey) { mutableFloatStateOf(memory.getFloat(memoryKey, START_DIVISOR).coerceIn(START_DIVISOR, MAX_DIVISOR)) }
    val finish = remember { RuntimeShader(FINISH_SKSL) }
    val finishBrush = remember(finish) { ShaderBrush(finish) }
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
        val started = withFrameNanos { it }
        var last = started
        var settle = started + SETTLE_NS
        val gaps = FloatArray(WINDOW)
        val lateness = FloatArray(WINDOW)
        var n = 0
        var windows = 0
        var reportAt = 1
        while (true) {
            withFrameNanos { now ->
                if (now - last >= FRAME_NS) {
                    if (now > settle) {
                        lateness[n] = (System.nanoTime() - now) / 1_000_000f
                        gaps[n++] = (now - last) / 1_000_000f
                    }
                    last = now
                    time.floatValue = (now - started) / 1_000_000_000f * speed
                }
            }
            if (n < WINDOW) continue
            n = 0
            windows++
            val median = gaps.sorted()[WINDOW / 2]
            val late = lateness.sorted()[WINDOW / 2]
            val gpuMs = gpu.median()
            if (windows == reportAt || windows % 60 == 0) {
                Log.i(TAG, "screensaver: ${style.id} at 1/${divisor.floatValue.toInt()}, $median ms a frame, $late ms late, GPU ${gpuMs ?: "unknown"} ms")
            }
            // Frames starting late after vsync mean the GPU is full; the GPU budget keeps it well short of that.
            if (median > SLOW_MS || late > BACKED_UP_MS || (gpuMs != null && gpuMs > GPU_BUDGET_MS)) {
                if (divisor.floatValue >= MAX_DIVISOR) {
                    onUnusable("it is too heavy for this TV")
                    return@LaunchedEffect
                }
                divisor.floatValue += 1f
                if (keepResolution) memory.edit().putFloat(memoryKey, divisor.floatValue).apply()
                gpu.clear()
                settle = last + SETTLE_NS
                reportAt = windows + 1
            }
        }
    }

    Box(Modifier.fillMaxSize()) {
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
            // The scene is bound again each frame: a child keeps the uniforms it had when bound.
            finish.setInputShader("scene", program.shader)
            finish.setFloatUniform("resolution", size.width, size.height)
            finish.setFloatUniform("brightness", lights.value)
            drawRect(finishBrush)
        }
    }
}
