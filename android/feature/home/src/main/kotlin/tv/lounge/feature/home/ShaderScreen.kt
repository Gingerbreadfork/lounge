package tv.lounge.feature.home

import android.content.Context
import android.graphics.BitmapShader
import android.graphics.RuntimeShader
import android.graphics.Shader
import android.os.Build
import android.view.SurfaceView
import androidx.annotation.RequiresApi
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import tv.lounge.design.BlueNoise
import tv.lounge.design.LoungeTheme
import tv.lounge.platform.PanelSize

// Lounge's finishing pass, drawn over every scene at the size of the surface's buffer: the room's
// vignette, the house lights, a ceiling on highlights so nothing glares at night, and one level of
// blue noise a pixel against banding.
private const val FINISH_SKSL = """
uniform shader scene;
uniform shader blue;
uniform float2 resolution;
uniform float brightness;

half4 main(float2 coord) {
    half3 c = scene.eval(coord).rgb;
    float2 v = coord / resolution - 0.5;
    v.x *= resolution.x / resolution.y;
    c *= half(brightness * (1.0 - 0.32 * smoothstep(0.55, 1.1, length(v))));
    half m = max(c.r, max(c.g, c.b));
    if (m > 0.72) {
        c *= (0.72 + (m - 0.72) * 0.45) / m;
    }
    c += blue.eval(coord).r;
    return half4(c, 1.0);
}
"""

/**
 * An abstract screensaver: the program drawn on the GPU behind the clock and its name. It comes
 * up out of black once its first frame is there, and the house lights are the [room]'s.
 */
@Composable
internal fun ShaderScreensaver(style: ShaderStyle, room: AmbientRoom, onUnusable: (String) -> Unit, preview: Boolean = false) {
    val motion = LoungeTheme.motion
    var shown by remember(style) { mutableStateOf(false) }
    val cover = animateFloatAsState(if (shown) 0f else 1f, tween(if (motion.reducedMotion) 0 else 1_200), label = "sceneCover")
    // Whatever went wrong last time, this run decides afresh.
    LaunchedEffect(style) { SceneMemory.seen.remove(style.id) }
    Box(Modifier.fillMaxSize().background(Color.Black)) {
        if (Build.VERSION.SDK_INT >= 33) {
            SaverSurface(style, room.lights, motion.reducedMotion, preview, onUnusable, onShown = { shown = true })
            // The surface is behind the window, so black drawn here covers it until it has a frame.
            Box(Modifier.fillMaxSize().drawBehind { if (cover.value > 0.001f) drawRect(Color.Black, alpha = cover.value) })
        } else {
            LaunchedEffect(Unit) { onUnusable("abstract screensavers need Android 13") }
        }
        AmbientCaption(room, style.name, style.origin)
    }
}

@RequiresApi(33)
@Composable
private fun SaverSurface(style: ShaderStyle, lights: State<Float>, slow: Boolean, preview: Boolean, onUnusable: (String) -> Unit, onShown: () -> Unit) {
    val context = LocalContext.current
    val colors = LoungeTheme.colors
    val panel = remember { PanelSize.of(context) }
    // Each renderer has a finish of its own: one is still drawing as the next starts.
    val finish = remember(style, colors) { SaverFinish() }
    val renderer = remember(finish, slow, preview) {
        SceneRenderer(
            context,
            style,
            colors,
            finish,
            ladderFor = { w, h -> SceneLadder.saver(w, h, panel, style.fine) },
            preview = preview,
            speed = if (slow) 0.35f else 1f,
            onUnusable = onUnusable,
            onShown = onShown,
        )
    }
    LaunchedEffect(finish) { snapshotFlow { lights.value }.collect { finish.brightness = it } }
    // A new scene gets a new surface: the callbacks that start a renderer fire once per surface.
    key(renderer) {
        AndroidView(
            factory = { c: Context ->
                SurfaceView(c).apply {
                    holder.addCallback(renderer)
                    // Over a live background's surface, still under the window.
                    setZOrderMediaOverlay(true)
                }
            },
            modifier = Modifier.fillMaxSize(),
        )
    }
    DisposableEffect(renderer) { onDispose { renderer.close() } }
}

@RequiresApi(33)
private class SaverFinish : SceneFinish {
    @Volatile var brightness: Float = 1f
    override val shader = RuntimeShader(FINISH_SKSL).apply {
        setInputShader("blue", BitmapShader(BlueNoise.bitmap, Shader.TileMode.REPEAT, Shader.TileMode.REPEAT))
    }

    override fun prepare(width: Float, height: Float, scale: Float): Boolean {
        shader.setFloatUniform("resolution", width, height)
        shader.setFloatUniform("brightness", brightness)
        return true
    }
}
