package tv.lounge.feature.home

import android.graphics.BitmapShader
import android.graphics.RuntimeShader
import android.graphics.Shader
import android.os.Build
import androidx.annotation.RequiresApi
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import kotlinx.coroutines.delay
import tv.lounge.design.BlueNoise
import tv.lounge.design.LoungeTheme

// Lounge's finishing pass, drawn at the screen's own size over every scene: the room's vignette,
// the house lights, a ceiling on highlights so nothing glares at night, and one level of blue
// noise a pixel against banding.
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
            SaverSurface(style, lights, motion.reducedMotion, !preview, onUnusable)
        } else {
            LaunchedEffect(Unit) { onUnusable("abstract screensavers need Android 13") }
        }
        AmbientCaption(clock, style.name, style.origin, drift)
    }
}

@RequiresApi(33)
@Composable
private fun SaverSurface(style: ShaderStyle, lights: State<Float>, slow: Boolean, keepResolution: Boolean, onUnusable: (String) -> Unit) {
    val finish = remember {
        RuntimeShader(FINISH_SKSL).apply { setInputShader("blue", BitmapShader(BlueNoise.bitmap, Shader.TileMode.REPEAT, Shader.TileMode.REPEAT)) }
    }
    // A scene with fine detail starts at full size on a TV with the GPU for it (DESIGN.md §5.5).
    val full = style.fine && LoungeTheme.visuals.tierA
    val quality = remember(keepResolution, full) { SceneQuality("saver", startDivisor = if (full) 1f else 2f, maxDivisor = 6f, gpuBudgetMs = 24f, keepResolution = keepResolution) }
    SceneSurface(style, quality, finish, slow, paused = { false }, onUnusable = onUnusable, finishKey = lights.value) { width, height ->
        finish.setFloatUniform("resolution", width, height)
        finish.setFloatUniform("brightness", lights.value)
    }
}
