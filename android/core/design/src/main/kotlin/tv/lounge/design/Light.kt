package tv.lounge.design

import androidx.compose.animation.Animatable
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.VectorConverter
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.ui.geometry.isSpecified
import androidx.compose.ui.geometry.isUnspecified
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import kotlin.math.roundToInt
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.State
import kotlinx.coroutines.flow.collectLatest
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.ColorMatrix
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.layout.layout
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/**
 * The focused element emits light (DESIGN.md §3.2, §6.1). [LightState] holds where it is and what
 * colour; the backdrop spill reads it.
 */
@Stable
class LightState {
    /** The focused element's layout node; its centre is read live, so the light rides along when it scrolls. */
    private var source: LayoutCoordinates? = null
    /** Bumped when a different element takes the light: the spill glides to it over `travel`. */
    var sourceId: Int by mutableIntStateOf(0)
        private set
    /** Bumped when the current source moved: the spill snaps to it. */
    var moved: Int by mutableIntStateOf(0)
        private set
    val center: Offset
        get() = source?.takeIf { it.isAttached }?.boundsInRoot()?.center ?: Offset.Unspecified
    var color: Color by mutableStateOf(Color(0xFFFFA86B))
        private set
    var active: Boolean by mutableStateOf(false)
        private set
    var idle: Boolean by mutableStateOf(false)
    /** The playbill's light while the Stage is expanded, for the light field to draw. */
    var bill: BillLight? by mutableStateOf(null)
    /** True while a [LightField] is drawing the spills, so the Stage publishes instead of painting. */
    var fieldDrawsLights: Boolean by mutableStateOf(false)
    /** 0 → 1 while the lamp warms up after a cold start; every light is multiplied by it. */
    var warmth: Float by mutableFloatStateOf(1f)
    /** True while something opaque covers the backdrop, so the field stops stepping. */
    var paused: Boolean by mutableStateOf(false)
    /**
     * The light field's clock in seconds, stepped a few times a second and still while the field
     * sleeps, so anything else that drifts moves in the field's own frames.
     */
    var clock: Float by mutableFloatStateOf(0f)

    private var lastCenter: Offset = Offset.Unspecified

    fun set(source: LayoutCoordinates, color: Color) {
        val changed = this.source !== source
        this.source = source
        this.color = color
        this.active = true
        val now = center
        if (changed) {
            lastCenter = now
            sourceId++
        } else if (now.isSpecified && (lastCenter.isUnspecified || (now - lastCenter).getDistanceSquared() > 0.25f)) {
            // Placement callbacks arrive on every layout pass; only a real move is worth a frame.
            lastCenter = now
            moved++
        }
    }

    /** The source was placed again (a row scrolled, the Stage lowered): the light follows at once. */
    fun sourceMoved() {
        if (active) moved++
    }

    fun clear() {
        active = false
    }
}

val LocalLightState = staticCompositionLocalOf { LightState() }

/** The idle factor on every light: down over `dim` when the room goes quiet, back over `settle` on any key. */
@Composable
fun rememberIdleLevel(idle: Boolean): State<Float> {
    val motion = LoungeTheme.motion
    return animateFloatAsState(if (idle) 0.5f else 1f, if (idle) motion.dim() else motion.settle(), label = "idleLevel")
}

/**
 * Backdrop spill: a radial gradient centred on the focused tile, moving over `travel`
 * and tweening colour. Draw this once, behind everything, filling the screen.
 */
@Composable
fun BackdropSpill(state: LightState, modifier: Modifier = Modifier) {
    val colors = LoungeTheme.colors
    val visuals = LoungeTheme.visuals
    val motion = LoungeTheme.motion
    if (!visuals.spill) return

    val animatedCenter = remember { Animatable(Offset(960f, 540f), Offset.VectorConverter) }
    val animatedColor = remember { Animatable(colors.houselight) }
    FollowLight(state, animatedCenter, motion)
    LaunchedEffect(motion.reducedMotion) {
        var placed = false
        snapshotFlow { state.color }.collectLatest { target ->
            if (motion.reducedMotion || !placed) animatedColor.snapTo(target) else animatedColor.animateTo(target, motion.curtain())
            placed = true
        }
    }
    val idleLevel = rememberIdleLevel(state.idle)
    val baseAlpha = 0.16f * colors.spillScale
    Canvas(modifier.fillMaxSize()) {
        val radius = Dimens.spillRadius.toPx()
        drawSpill(SpillMask.sharp, animatedCenter.value, radius, radius, animatedColor.value, baseAlpha * idleLevel.value * state.warmth)
    }
}

/**
 * Moves [center] with the light. A new source is reached over `travel`; the same source moving
 * under focus is followed at once. The first light is placed, not swept into.
 */
@Composable
fun FollowLight(state: LightState, center: Animatable<Offset, *>, motion: LoungeMotion) {
    LaunchedEffect(state, motion.reducedMotion) {
        var lastSource = -1
        snapshotFlow { Triple(state.sourceId, state.moved, state.active) }.collectLatest { (source, _, active) ->
            if (!active) return@collectLatest
            val target = state.center
            if (!target.isSpecified) return@collectLatest
            val glide = source != lastSource && lastSource != -1 && !motion.reducedMotion
            lastSource = source
            if (glide) center.animateTo(target, motion.travel()) else center.snapTo(target)
        }
    }
}

/**
 * Radial falloffs rasterised once at 256 px and drawn scaled. One texture fetch per pixel is far
 * cheaper on a TV GPU than a gradient shader.
 */
object SpillMask {
    private const val SIZE = 256

    fun warmUp() {
        sharp
        soft
    }

    /** To zero at 70 % of the radius with a soft shoulder: the backdrop spill. */
    val sharp: ImageBitmap by lazy { build { r -> Math.pow((1f - r / 0.7f).coerceIn(0f, 1f).toDouble(), 1.8).toFloat() } }

    /** Long tail to zero at the full radius: the playbill's light from the artwork. */
    val soft: ImageBitmap by lazy { build { r -> Math.pow((1f - r).coerceIn(0f, 1f).toDouble(), 1.6).toFloat() } }

    private fun build(falloff: (Float) -> Float): ImageBitmap {
        val pixels = IntArray(SIZE * SIZE)
        val half = SIZE / 2f
        for (y in 0 until SIZE) {
            for (x in 0 until SIZE) {
                val dx = (x + 0.5f - half) / half
                val dy = (y + 0.5f - half) / half
                val a = (falloff(kotlin.math.sqrt(dx * dx + dy * dy)) * 255f).roundToInt().coerceIn(0, 255)
                pixels[y * SIZE + x] = (a shl 24) or 0xFFFFFF
            }
        }
        return android.graphics.Bitmap.createBitmap(pixels, SIZE, SIZE, android.graphics.Bitmap.Config.ARGB_8888).asImageBitmap()
    }
}

/** Draw a tinted light ellipse of radii [rx] × [ry] centred on [center]. */
fun DrawScope.drawSpill(mask: ImageBitmap, center: Offset, rx: Float, ry: Float, color: Color, alpha: Float) {
    if (alpha <= 0.002f) return
    drawImage(
        image = mask,
        dstOffset = IntOffset((center.x - rx).roundToInt(), (center.y - ry).roundToInt()),
        dstSize = IntSize((2 * rx).roundToInt().coerceAtLeast(1), (2 * ry).roundToInt().coerceAtLeast(1)),
        alpha = alpha,
        colorFilter = ColorFilter.tint(color, BlendMode.SrcIn),
        filterQuality = FilterQuality.Low,
    )
}

/**
 * Glow: `light` at the tier's alpha with a 48 px blur, drawn behind and outside the element. The
 * blurred mask is rasterised once per size and cached; each frame draws one tinted quad.
 */
fun Modifier.lightGlow(color: Color, intensity: Float, radius: Dp = Dimens.glowRadius, cornerRadius: Dp = Dimens.tileRadius): Modifier {
    if (intensity <= 0.001f) return this
    return lightGlow(color, { intensity }, radius, cornerRadius)
}

/** As above, reading the intensity when drawing, so an animated glow never recomposes its owner. */
fun Modifier.lightGlow(color: Color, intensity: () -> Float, radius: Dp = Dimens.glowRadius, cornerRadius: Dp = Dimens.tileRadius): Modifier {
    return this.drawBehind {
        val i = intensity()
        if (i <= 0.001f) return@drawBehind
        val blur = radius.toPx()
        val mask = GlowMasks.get(size.width.roundToInt(), size.height.roundToInt(), cornerRadius.toPx(), blur)
        val pad = GlowMasks.pad(blur).toFloat()
        drawImage(
            image = mask,
            topLeft = Offset(-pad, -pad),
            alpha = i,
            colorFilter = ColorFilter.tint(color, BlendMode.SrcIn),
        )
    }
}

/**
 * Room for light. Scrolling containers clip to their bounds and would box a glow in, so the
 * container gets [room] of extra height as content padding and this reports the height without it,
 * placing the container [room] higher (DESIGN.md §5.3).
 */
fun Modifier.glowRoom(room: Dp = Dimens.glowRoom): Modifier = layout { measurable, constraints ->
    val px = room.roundToPx()
    val placeable = measurable.measure(
        constraints.copy(
            minHeight = 0,
            maxHeight = if (constraints.hasBoundedHeight) constraints.maxHeight + 2 * px else constraints.maxHeight,
        ),
    )
    layout(placeable.width, (placeable.height - 2 * px).coerceAtLeast(0)) { placeable.place(0, -px) }
}

/** Blurred white rounded-rect masks, keyed by size; tinted at draw time so colours never multiply the cache. */
object GlowMasks {
    private val cache = android.util.LruCache<String, ImageBitmap>(32)

    /**
     * The masks the first screen can need at [scale] px per design unit, so the first focus never
     * rasterises a blur in the frame.
     */
    fun warmUp(scale: Float) {
        val r = Dimens.tileRadius.value * scale
        val b = Dimens.glowRadius.value * scale
        get((Dimens.stripTileWidth.value * scale).roundToInt(), (Dimens.stripTileHeight.value * scale).roundToInt(), r, b)
        get((640 * scale).roundToInt(), (360 * scale).roundToInt(), 20 * scale, 60 * scale)
        for ((w, h) in listOf(384 to 216, 288 to 162, 208 to 312, 320 to 80)) {
            get((w * scale).roundToInt(), (h * scale).roundToInt(), r, b)
        }
    }

    /** The margin around the shape: twice the blur, where the light has faded to nothing. */
    fun pad(blur: Float): Int = (blur * 2f).roundToInt()

    fun get(width: Int, height: Int, corner: Float, blur: Float): ImageBitmap {
        val key = "$width x $height r$corner b$blur"
        cache.get(key)?.let { return it }
        val pad = pad(blur)
        val bmp = android.graphics.Bitmap.createBitmap(
            (width + pad * 2).coerceAtLeast(1),
            (height + pad * 2).coerceAtLeast(1),
            android.graphics.Bitmap.Config.ARGB_8888,
        )
        val paint = android.graphics.Paint(android.graphics.Paint.ANTI_ALIAS_FLAG).apply {
            this.color = android.graphics.Color.WHITE
            maskFilter = android.graphics.BlurMaskFilter(blur, android.graphics.BlurMaskFilter.Blur.NORMAL)
        }
        android.graphics.Canvas(bmp).drawRoundRect(pad.toFloat(), pad.toFloat(), (pad + width).toFloat(), (pad + height).toFloat(), corner, corner, paint)
        val image = bmp.asImageBitmap()
        cache.put(key, image)
        return image
    }
}

/**
 * The high-contrast theme's focus ring: 3 px of `limelight` just inside the element's edge, over
 * its content (DESIGN.md §11). Nothing on the other themes. Goes after the element's clip.
 */
fun Modifier.focusRing(colors: LoungeColors, shown: () -> Boolean, cornerRadius: Dp): Modifier {
    if (!colors.focusRing) return this
    return drawWithContent {
        drawContent()
        if (!shown()) return@drawWithContent
        val stroke = 3.dp.toPx()
        drawRoundRect(
            colors.limelight,
            topLeft = Offset(stroke / 2f, stroke / 2f),
            size = androidx.compose.ui.geometry.Size(size.width - stroke, size.height - stroke),
            cornerRadius = androidx.compose.ui.geometry.CornerRadius((cornerRadius.toPx() - stroke / 2f).coerceAtLeast(0f)),
            style = androidx.compose.ui.graphics.drawscope.Stroke(stroke),
        )
    }
}

/**
 * The rim on a focused tile: 2 px of `limelight` at 62 % just inside the edge, over the content,
 * so the lit tile also reads at a glance from across the room (DESIGN.md §6.1). On the
 * high-contrast theme it is that theme's full 3 px ring. [shown] is 0 to 1, read while drawing.
 */
fun Modifier.focusRim(colors: LoungeColors, shown: () -> Float, cornerRadius: Dp): Modifier = drawWithContent {
    drawContent()
    val a = shown()
    if (a <= 0.001f) return@drawWithContent
    val stroke = (if (colors.focusRing) 3.dp else 2.dp).toPx()
    drawRoundRect(
        colors.limelight.copy(alpha = if (colors.focusRing) 1f else 0.62f * a),
        topLeft = Offset(stroke / 2f, stroke / 2f),
        size = androidx.compose.ui.geometry.Size(size.width - stroke, size.height - stroke),
        cornerRadius = androidx.compose.ui.geometry.CornerRadius((cornerRadius.toPx() - stroke / 2f).coerceAtLeast(0f)),
        style = androidx.compose.ui.graphics.drawscope.Stroke(stroke),
    )
}

/** House-lights brightness states (DESIGN.md §6.1), as a colour matrix so blacks stay black. */
enum class Lights(val brightness: Float, val saturation: Float) {
    Focused(1f, 1f),
    Neighbour(0.86f, 1f),
    Rested(0.78f, 0.92f),
    OtherRow(0.70f, 0.88f),
    /** The room while an anchored menu is open: everything but the menu's own tile. */
    Dimmed(0.60f, 0.84f),
    ;

    fun dimmed(idle: Boolean): Pair<Float, Float> =
        if (idle) (brightness - 0.12f).coerceAtLeast(0.4f) to (saturation - 0.05f).coerceAtLeast(0.5f) else brightness to saturation
}

fun lightsFilter(brightness: Float, saturation: Float): ColorFilter? {
    if (brightness >= 0.999f && saturation >= 0.999f) return null
    val sat = ColorMatrix().apply { setToSaturation(saturation) }
    val bright = ColorMatrix(
        floatArrayOf(
            brightness, 0f, 0f, 0f, 0f,
            0f, brightness, 0f, 0f, 0f,
            0f, 0f, brightness, 0f, 0f,
            0f, 0f, 0f, 1f, 0f,
        ),
    )
    bright.timesAssign(sat)
    return ColorFilter.colorMatrix(bright)
}

/** Text colour that reads on `light`: limelight or velvet (DESIGN.md §3.2). */
fun textOn(light: Color, colors: LoungeColors): Color =
    if (contrastRatio(light, colors.velvet) >= 4.5) colors.velvet else colors.limelight

fun contrastRatio(a: Color, b: Color): Float {
    fun lum(c: Color): Float {
        fun lin(v: Float) = if (v <= 0.03928f) v / 12.92f else Math.pow(((v + 0.055f) / 1.055f).toDouble(), 2.4).toFloat()
        return 0.2126f * lin(c.red) + 0.7152f * lin(c.green) + 0.0722f * lin(c.blue)
    }
    val la = lum(a)
    val lb = lum(b)
    val hi = maxOf(la, lb)
    val lo = minOf(la, lb)
    return (hi + 0.05f) / (lo + 0.05f)
}
