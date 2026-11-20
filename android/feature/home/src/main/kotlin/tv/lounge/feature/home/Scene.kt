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
import android.os.HandlerThread
import android.os.Looper
import android.util.Log
import android.view.Choreographer
import android.view.SurfaceHolder
import androidx.annotation.RequiresApi
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import java.util.Calendar
import kotlin.math.ceil
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.android.asCoroutineDispatcher
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlin.coroutines.resume
import tv.lounge.design.LoungeColors
import tv.lounge.design.NoiseTexture
import tv.lounge.platform.PanelSize

private const val TAG = "Lounge"
private const val WINDOW = 45
private const val SETTLE_MS = 2_500L
/**
 * Nothing is measured until the app has run this long: its own start keeps the GPU busy, and a
 * step down taken then would be remembered.
 */
private const val WARM_MS = 15_000L
/** Frames this slow, three in a row, mean no step down will save the scene, so it ends at once. */
private const val HOPELESS_MS = 500f
/** Frames in a row at more than twice their interval before a step is taken without waiting for a window. */
private const val STALLED_FRAMES = 6
/** A try at a better rung waits for twice as many starts after each one that fails, up to this many doublings. */
private const val MAX_FAILS = 6
/** A try at a better rung is kept only if it holds within this share of the budget, so it is not kept on the edge. */
private const val TRY_MARGIN = 0.85f
/** Bumped when the renderer changes what a frame costs, so remembered steps start over. */
private const val RENDERER = 7

/** One size and rate a scene can be drawn at: the scene's own pixels, the buffer the finish fills, and the interval between frames. */
internal data class SceneRung(val sceneWidth: Int, val sceneHeight: Int, val outWidth: Int, val outHeight: Int, val frameMs: Long) {
    val fps: Int get() = (1000L / frameMs).toInt()
}

/**
 * The steps a scene takes as it proves too heavy, best first: the frame rate drops before the
 * picture shrinks. A scene new to this TV starts at [base]; the rungs before it are only reached
 * by holding them. [budgetMs] is what a frame may cost at 30 frames a second.
 */
internal class SceneLadder(val use: String, val rungs: List<SceneRung>, val base: Int, val budgetMs: Float, val remembersUnusable: Boolean) {
    fun budget(rung: SceneRung): Float = budgetMs * rung.frameMs / 33f

    companion object {
        private val RATES = listOf(33L, 50L)

        private fun rungs(sizes: List<IntArray>): List<SceneRung> = sizes.flatMap { s -> RATES.map { SceneRung(s[0], s[1], s[2], s[3], it) } }

        private fun part(of: Int, divisor: Int): Int = ceil(of / divisor.toFloat()).toInt().coerceAtLeast(1)

        /**
         * The screensaver's: the scene from half the window down to a sixth, finished at the
         * window's size. A scene with fine detail may also hold the window's own size, and the
         * panel's where that is larger.
         */
        fun saver(width: Int, height: Int, panel: PanelSize, fine: Boolean): SceneLadder {
            val sizes = ArrayList<IntArray>()
            if (fine && panel.height > height && panel.width > width) sizes += intArrayOf(panel.width, panel.height, panel.width, panel.height)
            if (fine) sizes += intArrayOf(width, height, width, height)
            val base = sizes.size * RATES.size
            for (d in 2..6) sizes += intArrayOf(part(width, d), part(height, d), width, height)
            return SceneLadder("saver", rungs(sizes), base, budgetMs = 24f, remembersUnusable = true)
        }

        /**
         * The live background's: the scene at a half or a third of the window, finished at that
         * size in a buffer the compositor scales, so the home keeps the GPU.
         */
        fun live(width: Int, height: Int): SceneLadder {
            val sizes = (2..3).map { d -> intArrayOf(part(width, d), part(height, d), part(width, d), part(height, d)) }
            return SceneLadder("live", rungs(sizes), 0, budgetMs = 24f, remembersUnusable = false)
        }
    }
}

/**
 * Where a scene was left on this TV: the rung it holds, the tries at a better one that have
 * failed, the starts since the last try, and why it cannot be shown if it cannot.
 */
internal data class SceneRecord(val step: Int, val fails: Int = 0, val starts: Int = 0, val problem: String? = null) {
    fun encode(): String = "$step:$fails:$starts:${problem.orEmpty()}"

    companion object {
        const val HEAVY = "heavy"
        const val BROKEN = "broken"

        fun decode(text: String?): SceneRecord? {
            val parts = text?.split(':') ?: return null
            if (parts.size != 4) return null
            val step = parts[0].toIntOrNull() ?: return null
            val fails = parts[1].toIntOrNull() ?: return null
            val starts = parts[2].toIntOrNull() ?: return null
            return SceneRecord(step.coerceAtLeast(0), fails.coerceIn(0, MAX_FAILS), starts.coerceAtLeast(0), parts[3].takeIf { it == HEAVY || it == BROKEN })
        }
    }
}

/** What a measurement changed. */
internal enum class StepChange { None, Kept, Stepped, Unusable }

/**
 * Chooses the rung a scene runs at from what its frames cost. Two slow windows in a row step it
 * down, and the step is remembered. So that one bad evening is not kept for good, a later start
 * tries the rung above again: two windows with room to spare keep it, one without goes back, and
 * each failure doubles the starts before the next try.
 */
internal class SceneSteps(private val ladder: SceneLadder, saved: SceneRecord?) {
    private val size = ladder.rungs.size
    /** What to remember; it changes with [StepChange.Kept], [StepChange.Stepped] and [StepChange.Unusable]. */
    var record: SceneRecord
        private set
    var step: Int
        private set
    /** Whether the scene is on a rung it has yet to hold. */
    var trying: Boolean = false
        private set
    private var slowWindows = 0
    private var cleanWindows = 0

    init {
        val held = saved?.step?.coerceIn(0, size)
        when {
            saved == null || held == null -> {
                record = SceneRecord(ladder.base)
                step = ladder.base
            }
            saved.problem == SceneRecord.BROKEN -> {
                record = saved
                step = size
            }
            held > 0 && saved.starts + 1 >= (1 shl saved.fails.coerceIn(0, MAX_FAILS)) -> {
                record = saved.copy(step = held, starts = 0)
                step = held - 1
                trying = true
            }
            else -> {
                // With no rung above there is nothing to wait for.
                record = saved.copy(step = held, starts = if (held > 0) (saved.starts + 1).coerceAtMost(1 shl MAX_FAILS) else 0)
                step = held
            }
        }
    }

    val unusable: Boolean get() = step >= size
    val rung: SceneRung get() = ladder.rungs[step.coerceAtMost(size - 1)]

    /** The median cost of a window of frames. */
    fun window(medianMs: Float): StepChange {
        val slow = medianMs > ladder.budget(rung)
        if (trying) {
            if (medianMs > TRY_MARGIN * ladder.budget(rung)) return back(medianMs)
            if (++cleanWindows < 2) return StepChange.None
            trying = false
            record = SceneRecord(step)
            return StepChange.Kept
        }
        slowWindows = if (slow) slowWindows + 1 else 0
        return if (slowWindows >= 2) down(medianMs) else StepChange.None
    }

    /** Several frames in a row at more than twice their interval, each costing about [costMs]. */
    fun stalled(costMs: Float): StepChange = if (trying) back(costMs) else down(costMs)

    /** Frames so slow that no rung will hold. */
    fun hopeless(costMs: Float): StepChange {
        if (trying) return back(costMs)
        step = size
        return gone()
    }

    /** The program cannot be drawn at all. */
    fun broken(): StepChange {
        trying = false
        step = size
        record = SceneRecord(size, problem = SceneRecord.BROKEN)
        return StepChange.Unusable
    }

    /** Gives up a try: far over budget, the next one waits much longer. */
    private fun back(costMs: Float): StepChange {
        trying = false
        cleanWindows = 0
        val far = costMs > 2f * ladder.budget(rung)
        record = record.copy(fails = (record.fails + if (far) 3 else 1).coerceAtMost(MAX_FAILS), starts = 0)
        step = record.step
        return if (unusable) StepChange.Unusable else StepChange.Stepped
    }

    private fun down(costMs: Float): StepChange {
        slowWindows = 0
        var next = step + 1
        // A slower rate cannot save a frame that costs more than the slower interval allows.
        if (next < size) {
            val same = ladder.rungs[next].sceneWidth == rung.sceneWidth && ladder.rungs[next].outWidth == rung.outWidth
            if (same && costMs > ladder.budget(ladder.rungs[next])) next++
        }
        step = next
        return gone()
    }

    private fun gone(): StepChange {
        val fails = (record.fails + 1).coerceAtMost(MAX_FAILS)
        record = when {
            !unusable -> SceneRecord(step, fails)
            ladder.remembersUnusable -> SceneRecord(size, fails, problem = SceneRecord.HEAVY)
            else -> SceneRecord(size - 1, fails)
        }
        return if (unusable) StepChange.Unusable else StepChange.Stepped
    }
}

/** What is remembered about scenes between starts, and what went wrong with one since the app started. */
internal class SceneMemory(context: Context) {
    private val prefs = context.getSharedPreferences(FILE, Context.MODE_PRIVATE)

    /** A rung is a place on one ladder, so what is remembered is kept per ladder. */
    fun key(ladder: SceneLadder, style: ShaderStyle): String {
        val top = ladder.rungs.first()
        return "${prefix(ladder.use, style)}${top.outWidth}x${top.outHeight}/${ladder.rungs.size}/${ladder.base}"
    }

    private fun prefix(use: String, style: ShaderStyle): String = "$RENDERER:$use:${style.id}:${style.source.hashCode()}:"

    fun read(key: String): SceneRecord? = runCatching { SceneRecord.decode(prefs.getString(key, null)) }.getOrNull()

    fun write(key: String, record: SceneRecord) {
        prefs.edit().putString(key, record.encode()).apply()
    }

    /** Drops what an earlier renderer remembered; its steps say nothing about this one. */
    fun prune() {
        if (pruned) return
        pruned = true
        val stale = prefs.all.keys.filter { k -> k.substringBefore(':').toIntOrNull().let { it != null && it != RENDERER } }
        if (stale.isNotEmpty()) prefs.edit().apply { stale.forEach { remove(it) } }.apply()
    }

    /** Why the screensaver cannot show [style], in words for the person, or null when it can. */
    fun problem(style: ShaderStyle): String? {
        seen[style.id]?.let { return it }
        val prefix = prefix("saver", style)
        val kept = runCatching { prefs.all }.getOrNull().orEmpty().entries.firstNotNullOfOrNull { (k, v) ->
            if (k.startsWith(prefix)) SceneRecord.decode(v as? String)?.problem else null
        }
        return words(kept, style.name)
    }

    companion object {
        const val FILE = "screensavers"
        @Volatile private var pruned = false

        /** Problems met since the app started, by style id, as [words] gives them. */
        val seen = mutableStateMapOf<String, String>()

        fun words(problem: String?, name: String): String? = when (problem) {
            SceneRecord.HEAVY -> "$name is too heavy for this TV"
            SceneRecord.BROKEN -> "$name could not be drawn on this TV"
            else -> null
        }
    }
}

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

/** The last pass over a scene, drawn into the surface's buffer: the room's light and the dither. */
@RequiresApi(33)
internal interface SceneFinish {
    /** Takes the scene as its `scene` input. */
    val shader: RuntimeShader

    /**
     * Sets the pass's uniforms for a buffer of [width] by [height], [scale] buffer pixels to one
     * of the window's. False while there is nothing to light the scene with yet.
     */
    fun prepare(width: Float, height: Float, scale: Float): Boolean
}

/**
 * Draws a scene on a surface behind the window, on a thread of its own, so the window does not
 * redraw for it. The scene is drawn at its rung's size into a buffer the GPU can sample, then the
 * finish over it into the surface's buffer, which the compositor scales to the screen. Each frame
 * is waited for, so its time is what it cost, and [SceneSteps] chooses the rung from that. While
 * [paused] the clock stops and nothing is drawn. A [preview] runs as the next start would and
 * remembers nothing.
 */
@RequiresApi(33)
internal class SceneRenderer(
    context: Context,
    private val style: ShaderStyle,
    private val colors: LoungeColors,
    private val finish: SceneFinish,
    private val ladderFor: (width: Int, height: Int) -> SceneLadder,
    private val preview: Boolean,
    private val speed: Float,
    private val onUnusable: (String) -> Unit,
    private val onShown: () -> Unit = {},
) : SurfaceHolder.Callback {
    private val thread = HandlerThread("lounge-scene").apply { start() }
    private val own = Handler(thread.looper)
    private val scope = CoroutineScope(SupervisorJob() + own.asCoroutineDispatcher())
    private val main = Handler(Looper.getMainLooper())
    private val memory = SceneMemory(context)
    @Volatile var paused: Boolean = false
    /** Held while a frame is drawn, so the surface is never used after it is destroyed. */
    private val drawing = java.util.concurrent.locks.ReentrantLock()
    @Volatile private var holder: SurfaceHolder? = null
    private var loop: Job? = null
    private var viewWidth = 0
    private var viewHeight = 0
    private var use = ""
    private var steps: SceneSteps? = null
    private var memoryKey = ""
    /** The buffer size last asked of the surface. */
    private var sizedWidth = 0
    private var sizedHeight = 0
    private var program: Program? = null
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
    private var shown = false
    private var hour = localHour()
    private var hourAt = 0L

    override fun surfaceCreated(h: SurfaceHolder) {
        holder = h
    }

    override fun surfaceChanged(h: SurfaceHolder, format: Int, width: Int, height: Int) {
        scope.launch {
            if (viewWidth == 0) {
                viewWidth = width
                viewHeight = height
                start(ladderFor(width, height))
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
        own.post {
            drawing.lock()
            try {
                releaseSceneTarget()
            } finally {
                drawing.unlock()
            }
        }
        thread.quitSafely()
    }

    /** The time of this thread's next vsync, in nanoseconds. */
    private suspend fun vsync(choreographer: Choreographer): Long = suspendCancellableCoroutine { c ->
        val callback = Choreographer.FrameCallback { at -> c.resume(at) }
        choreographer.postFrameCallback(callback)
        c.invokeOnCancellation { choreographer.removeFrameCallback(callback) }
    }

    private fun giveUp(why: String) {
        if (given) return
        given = true
        Log.w(TAG, "scene: $use ${style.id} given up: $why")
        loop?.cancel()
        loop = null
        main.post { onUnusable(why) }
    }

    private fun keep(s: SceneSteps) {
        if (!preview) memory.write(memoryKey, s.record)
    }

    /** Sizes the surface's buffer for the rung, so the compositor scales it to the screen. */
    private fun sizeSurface(rung: SceneRung) {
        if (sizedWidth == rung.outWidth && sizedHeight == rung.outHeight) return
        sizedWidth = rung.outWidth
        sizedHeight = rung.outHeight
        // The view's own thread: setFixedSize asks for a layout.
        main.post { holder?.setFixedSize(rung.outWidth, rung.outHeight) }
    }

    private fun start(ladder: SceneLadder) {
        use = ladder.use
        memoryKey = memory.key(ladder, style)
        if (!preview) memory.prune()
        val s = SceneSteps(ladder, memory.read(memoryKey))
        steps = s
        if (s.unusable) {
            giveUp(if (s.record.problem == SceneRecord.BROKEN) "it could not be drawn before" else "it was too heavy for this TV before")
            return
        }
        val compiled = compileScene(style, colors)
        program = compiled.getOrNull()
        if (program == null) {
            s.broken()
            keep(s)
            giveUp(compiled.exceptionOrNull()?.message ?: "the program does not compile")
            return
        }
        keep(s)
        sizedWidth = viewWidth
        sizedHeight = viewHeight
        sizeSurface(s.rung)
        Log.i(TAG, "scene: $use ${style.id} starts at ${s.rung.sceneWidth}x${s.rung.sceneHeight}, ${s.rung.fps} fps${if (s.trying) ", trying it" else ""}")
        loop = scope.launch { run(s) }
    }

    private suspend fun run(s: SceneSteps) {
        val choreographer = Choreographer.getInstance()
        var started = vsync(choreographer)
        var last = 0L
        val warm = android.os.Process.getStartUptimeMillis() + WARM_MS - android.os.SystemClock.uptimeMillis()
        var settleUntil = System.currentTimeMillis() + maxOf(SETTLE_MS, warm)
        val frames = FloatArray(WINDOW)
        var n = 0
        var windows = 0
        var pausedAt = 0L
        var reported = false
        var hopeless = 0
        var stalled = 0

        /** Acts on what a measurement changed; false when the scene has ended. */
        fun after(change: StepChange): Boolean {
            if (change == StepChange.None) return true
            keep(s)
            if (change == StepChange.Unusable) {
                giveUp("it is too heavy for this TV")
                return false
            }
            if (change == StepChange.Stepped) {
                sizeSurface(s.rung)
                settleUntil = System.currentTimeMillis() + SETTLE_MS
            }
            n = 0
            hopeless = 0
            stalled = 0
            reported = false
            return true
        }

        while (true) {
            // Without a surface there is nothing to draw on, as when another app is in front.
            if (paused || holder == null) {
                if (pausedAt == 0L) pausedAt = System.nanoTime()
                delay(200)
                continue
            }
            if (pausedAt != 0L) {
                // The scene carries on from where it stopped, and the measurements start afresh.
                started += System.nanoTime() - pausedAt
                pausedAt = 0L
                settleUntil = System.currentTimeMillis() + SETTLE_MS
                n = 0
            }
            val rung = s.rung
            // A frame starts on a vsync, so its buffer reaches the screen on one: the first that
            // comes an interval after the last, with half a vsync of slack.
            val frameNs = rung.frameMs * 1_000_000L - 8_000_000L
            var at = vsync(choreographer)
            while (at - last < frameNs) at = vsync(choreographer)
            last = at
            val t0 = System.nanoTime()
            val drawn = draw(rung, (at - started) / 1_000_000_000f * speed)
            val took = (System.nanoTime() - t0) / 1_000_000f
            // A frame that was not drawn cost nothing and says nothing.
            if (drawn && System.currentTimeMillis() > settleUntil) {
                frames[n++] = took
                hopeless = if (took > HOPELESS_MS) hopeless + 1 else 0
                stalled = if (took > 2f * rung.frameMs) stalled + 1 else 0
                val change = when {
                    hopeless >= 3 -> s.hopeless(took)
                    stalled >= STALLED_FRAMES -> s.stalled(took)
                    n >= WINDOW -> {
                        n = 0
                        windows++
                        val median = frames.sorted()[WINDOW / 2]
                        if (!reported || windows % 60 == 0) {
                            Log.i(TAG, "scene: $use ${style.id} at ${rung.sceneWidth}x${rung.sceneHeight}, ${rung.fps} fps, $median ms a frame${if (s.trying) ", trying it" else ""}")
                            reported = true
                        }
                        s.window(median)
                    }
                    else -> StepChange.None
                }
                if (!after(change)) return
            }
        }
    }

    /** Draws one frame; false when there was no surface or nothing to light it with. */
    private fun draw(rung: SceneRung, time: Float): Boolean {
        drawing.lock()
        try {
            return drawLocked(rung, time)
        } finally {
            drawing.unlock()
        }
    }

    /** Draws the scene at [sw] by [sh] into a buffer the GPU can sample, as a shader scaled by [up]. */
    private fun renderScene(p: Program, sw: Int, sh: Int, time: Float, up: Float): Shader? {
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
        val now = System.currentTimeMillis()
        if (now - hourAt > 60_000L) {
            hour = localHour()
            hourAt = now
        }
        p.provide(sw.toFloat(), sh.toFloat(), time, hour, colors)
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
        sceneMatrix.setScale(up, up)
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

    private fun drawLocked(rung: SceneRung, time: Float): Boolean {
        val h = holder ?: return false
        val p = program ?: return false
        val canvas: Canvas = try {
            h.lockHardwareCanvas()
        } catch (e: Exception) {
            Log.w(TAG, "scene: no canvas: $e")
            null
        } ?: return false
        var posted = false
        try {
            val w = canvas.width
            val hgt = canvas.height
            // The canvas is the rung's own size once the surface is resized, the window's until then.
            if (finish.prepare(w.toFloat(), hgt.toFloat(), w.toFloat() / viewWidth)) {
                val scene = renderScene(p, rung.sceneWidth, rung.sceneHeight, time, w.toFloat() / rung.sceneWidth)
                if (scene != null) {
                    finish.shader.setInputShader("scene", scene)
                    paint.shader = finish.shader
                    canvas.drawRect(0f, 0f, w.toFloat(), hgt.toFloat(), paint)
                    posted = true
                }
            }
            if (!posted) canvas.drawColor(android.graphics.Color.BLACK)
        } catch (e: Exception) {
            Log.w(TAG, "scene: draw failed: $e")
            giveUp("it does not draw: $e")
        } finally {
            h.unlockCanvasAndPost(canvas)
        }
        if (posted && !shown) {
            shown = true
            main.post(onShown)
        }
        return posted
    }
}

/** The local time of day in hours, from 0 to 24. */
internal fun localHour(): Float {
    val c = Calendar.getInstance()
    return c.get(Calendar.HOUR_OF_DAY) + c.get(Calendar.MINUTE) / 60f
}
