package tv.lounge.design

import android.graphics.RuntimeShader
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
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.ImageShader
import androidx.compose.ui.graphics.ShaderBrush
import androidx.compose.ui.graphics.TileMode
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.layout
import androidx.compose.ui.unit.Constraints
import java.util.Calendar
import kotlinx.coroutines.delay

// The lit backdrop in one pass: slow clouds over the ground, the focused tile's spill, the
// playbill's light from the artwork edge, and a little grain against banding.
private const val LIGHT_FIELD_SKSL = """
uniform float2 size;
uniform float time;
uniform float amp;
layout(color) uniform half4 base;
layout(color) uniform half4 horizon;
layout(color) uniform half4 tint;
uniform float2 spill;
uniform float spillR;
uniform float spillAlpha;
layout(color) uniform half4 spillColor;
uniform float2 bill;
uniform float2 billRadii;
uniform float billAlpha;
layout(color) uniform half4 billColor;

float hash(float2 p) {
    return fract(sin(dot(p, float2(127.1, 311.7))) * 43758.5453);
}

float noise(float2 p) {
    float2 i = floor(p);
    float2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    float a = hash(i);
    float b = hash(i + float2(1.0, 0.0));
    float c = hash(i + float2(0.0, 1.0));
    float d = hash(i + float2(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

half4 main(float2 frag) {
    float2 uv = frag / size;
    float t = time * 0.018;
    float clouds = noise(uv * float2(2.6, 1.6) + float2(t, -t * 0.6));
    float grain = (hash(frag) - 0.5) * 0.012;
    half3 ground = mix(base.rgb, horizon.rgb, half(clamp((uv.x + uv.y) * 0.5, 0.0, 1.0)));
    half3 col = ground + tint.rgb * half((clouds - 0.5) * 0.09 * amp) + half(grain);
    float ds = length(frag - spill) / spillR;
    float s = pow(clamp(1.0 - ds / 0.7, 0.0, 1.0), 1.8) * spillAlpha;
    col = mix(col, spillColor.rgb, half(s));
    float2 e = (frag - bill) / billRadii;
    float db = length(e);
    float bl = pow(clamp(1.0 - db, 0.0, 1.0), 1.6) * billAlpha;
    col = mix(col, billColor.rgb, half(bl));
    return half4(col, 1.0);
}
"""

/**
 * The playbill's light, published by the Stage (screen pixels). [strength] is read while drawing,
 * so the light fades with the playbill as the Stage lowers.
 */
data class BillLight(val center: Offset, val rx: Float, val ry: Float, val color: Color, val strength: () -> Float = { 1f })

private const val FIELD_SLEEPS_AFTER_MS = 60_000L

/**
 * Everything lit behind the home screen, in one draw. With the shader on, the spill and the
 * playbill's light are part of the same pass; without it the bitmap spill is drawn alone.
 */
@Composable
fun LightField(state: LightState, modifier: Modifier = Modifier) {
    val visuals = LoungeTheme.visuals
    if (visuals.shaderWallpaper && Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        LightFieldImpl(state, modifier)
        Grain(modifier)
    } else {
        BackdropSpill(state, modifier)
    }
}

/**
 * Grain at the screen's own resolution over the field. The field is drawn small and scaled up,
 * which smooths its own grain away, and its slow dark gradients then show as moving contour
 * lines on an 8-bit panel; two levels of noise per pixel break the contours up.
 */
@Composable
private fun Grain(modifier: Modifier) {
    val brush = remember { ShaderBrush(ImageShader(GrainTexture.bitmap, TileMode.Repeated, TileMode.Repeated)) }
    Canvas(modifier.fillMaxSize()) { drawRect(brush, blendMode = BlendMode.Plus) }
}

private object GrainTexture {
    val bitmap: ImageBitmap by lazy {
        val n = 128
        val random = java.util.Random(7)
        val px = IntArray(n * n) {
            val v = random.nextInt(3)
            (0xFF shl 24) or (v shl 16) or (v shl 8) or v
        }
        android.graphics.Bitmap.createBitmap(px, n, n, android.graphics.Bitmap.Config.ARGB_8888).asImageBitmap()
    }
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
    val shader = remember { RuntimeShader(LIGHT_FIELD_SKSL) }
    val brush = remember(shader) { ShaderBrush(shader) }

    DisposableEffect(state) {
        state.fieldDrawsLights = true
        onDispose { state.fieldDrawsLights = false }
    }

    // The clouds step a few times a second and each step recomposites the screen, so the clock
    // stops under an opaque screen and a minute after the last key.
    val time = remember { mutableFloatStateOf(0f) }
    val stepMs = if (visuals === LoungeVisuals.TierA) 333L else 500L
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

    // The spill moves with focus over `travel` and takes the tile's colour over `curtain`,
    // exactly as the bitmap spill does; the first light is placed, not swept into.
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
    // The spill goes and comes back over `settle` when nothing holds the light, not in a frame.
    val activeLevel = animateFloatAsState(if (state.active) 1f else 0f, if (state.active) motion.snap() else motion.settle(), label = "spillActive")
    val spillAlpha = 0.20f * colors.spillScale
    val ampBase = colors.spillScale * (if (night) 1f else 0.7f)
    val billAlpha = 0.22f * colors.spillScale

    val scale = 3f
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
            // An offscreen layer: the shader runs over the small layer only when something in
            // it changes; every other frame just composites that texture.
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
        shader.setFloatUniform("size", size.width, size.height)
        shader.setFloatUniform("time", time.floatValue)
        shader.setFloatUniform("amp", ampBase * (0.5f + 0.5f * idleLevel.value))
        shader.setColorUniform("base", android.graphics.Color.valueOf(colors.velvet.red, colors.velvet.green, colors.velvet.blue, 1f))
        shader.setColorUniform("horizon", android.graphics.Color.valueOf(colors.horizon.red, colors.horizon.green, colors.horizon.blue, 1f))
        shader.setColorUniform("tint", android.graphics.Color.valueOf(sc.red, sc.green, sc.blue, 1f))
        shader.setFloatUniform("spill", c.x / scale, c.y / scale)
        shader.setFloatUniform("spillR", Dimens.spillRadius.toPx() / scale)
        shader.setFloatUniform("spillAlpha", spillAlpha * level * activeLevel.value)
        shader.setColorUniform("spillColor", android.graphics.Color.valueOf(sc.red, sc.green, sc.blue, 1f))
        if (bill != null) {
            shader.setFloatUniform("bill", bill.center.x / scale, bill.center.y / scale)
            shader.setFloatUniform("billRadii", (bill.rx / scale).coerceAtLeast(1f), (bill.ry / scale).coerceAtLeast(1f))
            shader.setFloatUniform("billAlpha", billAlpha * level * bill.strength().coerceIn(0f, 1f))
            shader.setColorUniform("billColor", android.graphics.Color.valueOf(bill.color.red, bill.color.green, bill.color.blue, 1f))
        } else {
            shader.setFloatUniform("bill", -10000f, -10000f)
            shader.setFloatUniform("billRadii", 1f, 1f)
            shader.setFloatUniform("billAlpha", 0f)
            shader.setColorUniform("billColor", android.graphics.Color.valueOf(0f, 0f, 0f, 1f))
        }
        drawRect(brush)
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
