package tv.lounge.design

import android.graphics.BitmapShader
import android.graphics.RuntimeShader
import android.graphics.Shader
import android.os.Build
import androidx.annotation.RequiresApi
import androidx.compose.animation.Animatable
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.VectorConverter
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import kotlinx.coroutines.flow.collectLatest
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.isSpecified
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.ShaderBrush
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.layout
import androidx.compose.ui.unit.Constraints
import java.util.Calendar
import kotlinx.coroutines.delay

// The ground of the lit backdrop at the screen's own size: slow clouds over velvet running into
// the horizon, and one level of blue noise a pixel against banding. The clouds come from the
// baked noise tile, one fetch a pixel, so a full-size draw is cheap; it happens only when the
// field's clock steps.
private const val FIELD_GROUND_SKSL = """
uniform shader noise;
uniform shader blue;
uniform float2 size;
uniform float time;
uniform float amp;
layout(color) uniform half4 base;
layout(color) uniform half4 horizon;
layout(color) uniform half4 tint;

half4 main(float2 frag) {
    float2 uv = frag / size;
    float t = time * 0.018;
    float clouds = noise.eval((uv * float2(2.6, 1.6) + float2(t, -t * 0.6)) * 16.0).r;
    half3 ground = mix(base.rgb, horizon.rgb, half(clamp((uv.x + uv.y) * 0.5, 0.0, 1.0)));
    half3 c = ground + tint.rgb * half((clouds - 0.5) * 0.09 * amp);
    c += blue.eval(frag).r;
    return half4(c, 1.0);
}
"""

// The lights over the ground: the focused tile's spill and the playbill's light from the artwork
// edge, with their own alpha so blending does the mixing. They move with every frame of a focus
// change, so they are drawn at half size; a smooth falloff loses nothing to that.
private const val FIELD_LIGHTS_SKSL = """
uniform float2 spill;
uniform float spillR;
uniform float spillAlpha;
layout(color) uniform half4 spillColor;
uniform float2 bill;
uniform float2 billRadii;
uniform float billAlpha;
layout(color) uniform half4 billColor;

half4 main(float2 frag) {
    float ds = length(frag - spill) / spillR;
    float s = pow(clamp(1.0 - ds / 0.7, 0.0, 1.0), 1.8) * spillAlpha;
    float2 e = (frag - bill) / billRadii;
    float bl = pow(clamp(1.0 - length(e), 0.0, 1.0), 1.6) * billAlpha;
    half3 c = spillColor.rgb * half(s * (1.0 - bl)) + billColor.rgb * half(bl);
    return half4(c, half(1.0 - (1.0 - s) * (1.0 - bl)));
}
"""

/**
 * The playbill's light, published by the Stage (screen pixels). [strength] is read while drawing,
 * so the light fades with the playbill as the Stage lowers.
 */
data class BillLight(val center: Offset, val rx: Float, val ry: Float, val color: Color, val strength: () -> Float = { 1f })

private const val FIELD_SLEEPS_AFTER_MS = 60_000L

/**
 * Everything lit behind the home screen. With the shader on, the ground is drawn and dithered at
 * the screen's size and the lights at half size over it; without it the bitmap spill is drawn
 * alone.
 */
@Composable
fun LightField(state: LightState, modifier: Modifier = Modifier) {
    val visuals = LoungeTheme.visuals
    if (visuals.shaderWallpaper && Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        LightFieldImpl(state, modifier)
    } else {
        BackdropSpill(state, modifier)
    }
}

/**
 * A 64 × 64 blue-noise tile at half density: one level on half the pixels, wrapping without a
 * seam. Added at the screen's own size to a slow dark gradient before it is quantised: without it
 * the gradient breaks into contour lines that drift on an 8-bit panel, and blue noise has no
 * clumps, so at one level it reads as a smooth surface rather than grain.
 */
object BlueNoise {
    /** Made by void-and-cluster, one bit a pixel. */
    private const val TILE = "q2K4mWl1TJtGLSXqrLKZSnXJawVKjWNxlLtkem1ifSsmSVbOqz6LTPOKTRHQRWRqiHOprh/xVuOutnRy4g2lDFaKJ4VaWrVqmbWouRPxLVuwpabttRdhKC7K3QZkzFbLRzRJbJZymW3YtarjWROrVVqK0xiz3GYwSrYqlswjKc910tT6jd3akqIYjoSyUEaazWexbOVPNXU7SjZTndyVipDZYrQUEfVpz1ObWvPWIS1zLTqHjHKfRCR1QXAnFdT12cVWXeqsoxYvFPuLWOa4q2DfCmIvJQ7I20CkvYRJ5Vsae23G+fxapqlKrBBLQxRZppaD9zY9s8U5MvgbYkCmdTdtK6Cr62oNgWSm9Zoeq9m+i8gOtuBUUkm8P6jCp91npSXAa1lag6wsqldLLpT0NfN3Wrpi1hWpFUCRk7VlJaalPW6SRiv6OVZGkl17fAmk0ujaswEBba87WtLC/teqFMhWrl5ieityl8JFTIlCkqZ4Ouphvy3Wbae2W61BujUFEoFZWTTS6+ncv0NVtiUEPSsiukrLuXuCcWombGWHTLsVqcpNGHoxq00XO+HvKUaI61CpHiNnbXdQ3tSxdJRxqK8gx1tLSw4LCrc4plx623m5lWWpo4TRpaRollyoqYysW09VFu+vNbNTktPiCHJsKlhsWFrnJkmzVU2PKy2TZ0o="

    val bitmap: android.graphics.Bitmap by lazy {
        val n = 64
        val bits = android.util.Base64.decode(TILE, android.util.Base64.DEFAULT)
        val px = IntArray(n * n) { i ->
            val v = (bits[i / 8].toInt() shr (7 - i % 8)) and 1
            (0xFF shl 24) or (v shl 16) or (v shl 8) or v
        }
        android.graphics.Bitmap.createBitmap(px, n, n, android.graphics.Bitmap.Config.ARGB_8888)
    }
    val image: ImageBitmap by lazy { bitmap.asImageBitmap() }
}

private fun isNight(): Boolean {
    val hour = Calendar.getInstance().get(Calendar.HOUR_OF_DAY)
    return hour >= 19 || hour < 6
}

@RequiresApi(Build.VERSION_CODES.TIRAMISU)
@Composable
private fun LightFieldImpl(state: LightState, modifier: Modifier) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    val visuals = LoungeTheme.visuals
    val ground = remember {
        RuntimeShader(FIELD_GROUND_SKSL).apply {
            setInputShader("noise", NoiseTexture.shader)
            setInputShader("blue", BitmapShader(BlueNoise.bitmap, Shader.TileMode.REPEAT, Shader.TileMode.REPEAT))
        }
    }
    val groundBrush = remember(ground) { ShaderBrush(ground) }
    val lights = remember { RuntimeShader(FIELD_LIGHTS_SKSL) }
    val lightsBrush = remember(lights) { ShaderBrush(lights) }

    DisposableEffect(state) {
        state.fieldDrawsLights = true
        onDispose { state.fieldDrawsLights = false }
    }

    // The spill moves with focus over `travel` and takes the tile's colour over `curtain`,
    // exactly as the bitmap spill does; the first light is placed, not swept into.
    val spillCenter = remember { Animatable(Offset(960f, 540f), Offset.VectorConverter) }
    val spillColor = remember { Animatable(colors.houselight) }
    FollowLight(state, spillCenter, motion)
    // The clouds take the light's colour only when their clock steps, so a colour tween never
    // redraws the full-size ground every frame.
    val cloudTint = remember { mutableStateOf(colors.houselight) }
    LaunchedEffect(motion.reducedMotion) {
        var placed = false
        snapshotFlow { state.color }.collectLatest { target ->
            if (motion.reducedMotion || !placed) spillColor.snapTo(target) else spillColor.animateTo(target, motion.curtain())
            placed = true
            if (motion.reducedMotion) cloudTint.value = target
        }
    }

    // The clouds step a few times a second and each step recomposites the screen, so the clock
    // stops under an opaque screen and a minute after the last key.
    val time = remember { mutableFloatStateOf(0f) }
    val stepMs = if (visuals.tierA) 333L else 500L
    var night by remember { mutableStateOf(isNight()) }
    if (!motion.reducedMotion) {
        LaunchedEffect(stepMs) {
            val started = System.nanoTime()
            var paused = 0L
            var pausedAt = 0L
            snapshotFlow { state.paused }.collectLatest { sleeping ->
                if (sleeping) {
                    pausedAt = System.nanoTime()
                    return@collectLatest
                }
                if (pausedAt != 0L) {
                    paused += System.nanoTime() - pausedAt
                    pausedAt = 0L
                }
                while (true) {
                    delay(stepMs)
                    time.floatValue = ((System.nanoTime() - started - paused) / 1_000_000_000.0).toFloat()
                    state.clock = time.floatValue
                    cloudTint.value = spillColor.value
                }
            }
        }
    }
    LaunchedEffect(Unit) {
        while (true) {
            delay(10 * 60_000L)
            night = isNight()
        }
    }
    val idleLevel = rememberIdleLevel(state.idle)
    // The spill goes and comes back over `settle` when nothing holds the light, not in a frame.
    val activeLevel = animateFloatAsState(if (state.active) 1f else 0f, if (state.active) motion.snap() else motion.settle(), label = "spillActive")
    val spillAlpha = 0.20f * colors.spillScale
    val ampBase = colors.spillScale * (if (night) 1f else 0.7f)
    val billAlpha = 0.22f * colors.spillScale

    // An offscreen layer at the screen's size: the ground is drawn again only when its clock
    // steps or the light's colour reaches it; every other frame just composites that texture.
    Canvas(modifier.fillMaxSize().graphicsLayer { compositingStrategy = CompositingStrategy.Offscreen }) {
        val tint = cloudTint.value
        ground.setFloatUniform("size", size.width, size.height)
        ground.setFloatUniform("time", time.floatValue)
        ground.setFloatUniform("amp", ampBase * (0.5f + 0.5f * idleLevel.value))
        ground.setColorUniform("base", android.graphics.Color.valueOf(colors.velvet.red, colors.velvet.green, colors.velvet.blue, 1f))
        ground.setColorUniform("horizon", android.graphics.Color.valueOf(colors.horizon.red, colors.horizon.green, colors.horizon.blue, 1f))
        ground.setColorUniform("tint", android.graphics.Color.valueOf(tint.red, tint.green, tint.blue, 1f))
        drawRect(groundBrush)
    }
    val scale = 2f
    Canvas(
        modifier
            .fillMaxSize()
            .layout { measurable, constraints ->
                // Fixed, not a range: the canvas takes its minimum size and would otherwise be 0 × 0.
                val placeable = measurable.measure(
                    Constraints.fixed(
                        kotlin.math.ceil(constraints.maxWidth / scale).toInt(),
                        kotlin.math.ceil(constraints.maxHeight / scale).toInt(),
                    ),
                )
                layout(constraints.maxWidth, constraints.maxHeight) { placeable.place(0, 0) }
            }
            .graphicsLayer {
                scaleX = scale
                scaleY = scale
                transformOrigin = TransformOrigin(0f, 0f)
                compositingStrategy = CompositingStrategy.Offscreen
            },
    ) {
        val c = spillCenter.value
        val sc = spillColor.value
        val bill = state.bill
        val level = idleLevel.value * state.warmth
        lights.setFloatUniform("spill", c.x / scale, c.y / scale)
        lights.setFloatUniform("spillR", Dimens.spillRadius.toPx() / scale)
        lights.setFloatUniform("spillAlpha", spillAlpha * level * activeLevel.value)
        lights.setColorUniform("spillColor", android.graphics.Color.valueOf(sc.red, sc.green, sc.blue, 1f))
        if (bill != null) {
            lights.setFloatUniform("bill", bill.center.x / scale, bill.center.y / scale)
            lights.setFloatUniform("billRadii", (bill.rx / scale).coerceAtLeast(1f), (bill.ry / scale).coerceAtLeast(1f))
            lights.setFloatUniform("billAlpha", billAlpha * level * bill.strength().coerceIn(0f, 1f))
            lights.setColorUniform("billColor", android.graphics.Color.valueOf(bill.color.red, bill.color.green, bill.color.blue, 1f))
        } else {
            lights.setFloatUniform("bill", -10000f, -10000f)
            lights.setFloatUniform("billRadii", 1f, 1f)
            lights.setFloatUniform("billAlpha", 0f)
            lights.setColorUniform("billColor", android.graphics.Color.valueOf(0f, 0f, 0f, 1f))
        }
        drawRect(lightsBrush)
    }
}

/**
 * Sleep the field's clock after a minute without input: [LightState.paused] follows the input
 * time read through [lastInputAt], so a key press restarts the timer without recomposing the caller.
 */
@Composable
fun FieldSleep(state: LightState, lastInputAt: () -> Long, overlayOpaque: Boolean) {
    LaunchedEffect(overlayOpaque) {
        snapshotFlow { lastInputAt() }.collectLatest {
            state.paused = overlayOpaque
            if (!overlayOpaque) {
                delay(FIELD_SLEEPS_AFTER_MS)
                state.paused = true
            }
        }
    }
}
