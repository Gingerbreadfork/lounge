package tv.lounge.feature.home

import android.content.Context
import android.graphics.BitmapShader
import android.graphics.RuntimeShader
import android.graphics.Shader
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
import kotlinx.coroutines.flow.collectLatest
import tv.lounge.design.BlueNoise
import tv.lounge.design.Dimens
import tv.lounge.design.FollowLight
import tv.lounge.design.LightState
import tv.lounge.design.LoungeTheme
import tv.lounge.design.rememberIdleLevel

// The room's light over a live scene, drawn at the scene's own size: the scene dimmed and held
// under a ceiling so the tiles stay the brightest things in the room, the focused tile's spill and
// the playbill's light as the still field draws them, and one level of blue noise a pixel against
// banding. The compositor scales the result to the screen.
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

    // Each renderer has a finish of its own: one is still drawing as the next starts.
    val finish = remember(style, colors) { LiveFinish() }
    val renderer = remember(finish) {
        SceneRenderer(context, style, colors, finish, { w, h -> SceneLadder.live(w, h) }, preview = false, speed = 1f, onUnusable = onUnusable)
    }
    LaunchedEffect(finish) {
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
        }.collect { finish.lights = it }
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

/** The live background's finish: the lights of the frame being drawn, in the buffer's own pixels. */
@RequiresApi(33)
private class LiveFinish : SceneFinish {
    @Volatile var lights: Lights? = null
    override val shader = RuntimeShader(LIVE_FINISH_SKSL).apply {
        setInputShader("blue", BitmapShader(BlueNoise.bitmap, Shader.TileMode.REPEAT, Shader.TileMode.REPEAT))
    }

    override fun prepare(width: Float, height: Float, scale: Float): Boolean {
        val l = lights ?: return false
        val k = scale
        shader.setFloatUniform("level", l.level)
        shader.setFloatUniform("spill", l.spill.x * k, l.spill.y * k)
        shader.setFloatUniform("spillR", l.spillRadius * k)
        shader.setFloatUniform("spillAlpha", l.spillAlpha)
        shader.setColorUniform("spillColor", android.graphics.Color.valueOf(l.spillColor.red, l.spillColor.green, l.spillColor.blue, 1f))
        val bill = l.bill
        if (bill != null) {
            shader.setFloatUniform("bill", bill.x * k, bill.y * k)
            shader.setFloatUniform("billRadii", (l.billRadii.x * k).coerceAtLeast(1f), (l.billRadii.y * k).coerceAtLeast(1f))
            shader.setFloatUniform("billAlpha", l.billAlpha)
            shader.setColorUniform("billColor", android.graphics.Color.valueOf(l.billColor.red, l.billColor.green, l.billColor.blue, 1f))
        } else {
            shader.setFloatUniform("bill", -10000f, -10000f)
            shader.setFloatUniform("billRadii", 1f, 1f)
            shader.setFloatUniform("billAlpha", 0f)
            shader.setColorUniform("billColor", android.graphics.Color.valueOf(0f, 0f, 0f, 1f))
        }
        return true
    }
}
