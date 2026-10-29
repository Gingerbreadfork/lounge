package tv.lounge.feature.home

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.requiredSize
import androidx.compose.foundation.layout.wrapContentSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.drawWithCache
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.ImageShader
import androidx.compose.ui.graphics.ShaderBrush
import androidx.compose.ui.graphics.TileMode
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Dp
import kotlin.math.PI
import kotlin.math.exp
import kotlin.math.sin
import tv.lounge.design.BlueNoise
import tv.lounge.design.LoungeTheme

/** How far past the band the drifting layers are drawn, so their edges never slide into view. */
private const val POOLS_OVER = 0.06f
private const val BLOOM_OVER_X = 0.3f
private const val BLOOM_OVER_Y = 0.6f

/**
 * The lowered Stage's light (DESIGN.md §5.2): the picture's own colours eased across the band, its
 * light blooming toward the right, the edges falling away, and the room's dark on the left where
 * the title sits. Each part is drawn once into a layer of its own; [clock] only moves and fades the
 * pools and the bloom as layer properties, so the drift redraws nothing but the dither.
 */
@Composable
internal fun BandLight(glow: List<Color>, light: Color, velvet: Color, clock: () -> Float, width: Dp, height: Dp, modifier: Modifier = Modifier) {
    val dither = remember { ShaderBrush(ImageShader(BlueNoise.image, TileMode.Repeated, TileMode.Repeated)) }
    // A picture without colour, a black and white film, blooms silver: the house lamp stands in
    // for its light elsewhere, but here it would tint the picture's own light.
    val limelight = LoungeTheme.colors.limelight
    val light = remember(glow, light, limelight) {
        val brightest = glow.maxByOrNull { it.red + it.green + it.blue }
        if (brightest != null && glow.all { maxOf(it.red, it.green, it.blue) - minOf(it.red, it.green, it.blue) < 0.05f }) lerp(brightest, limelight, 0.5f) else light
    }
    // In pixels for the layer blocks, which run before their own size is known.
    val (bandW, bandH) = with(LocalDensity.current) { width.toPx() to height.toPx() }
    Box(modifier.requiredSize(width, height)) {
        // The pools, drawn a little wider than the band, sway sideways.
        Spacer(
            Modifier
                .wrapContentSize(Alignment.Center, unbounded = true)
                .requiredSize(width * (1f + POOLS_OVER), height)
                .graphicsLayer {
                    compositingStrategy = CompositingStrategy.Offscreen
                    translationX = bandW * 0.025f * sin(TWO_PI * clock() / 71f)
                }
                .drawWithCache {
                    // Without key art the band is one pool of the picture's light.
                    val pools = Brush.horizontalGradient(if (glow.size >= 2) glow else List(2) { lerp(velvet, light, 0.45f) })
                    onDrawBehind { drawRect(pools) }
                },
        )
        // A soft bloom of the picture's light, wider than tall, right of middle; it wanders and breathes.
        Spacer(
            Modifier
                .wrapContentSize(Alignment.Center, unbounded = true)
                .requiredSize(width * (1f + BLOOM_OVER_X), height * (1f + BLOOM_OVER_Y))
                .graphicsLayer {
                    compositingStrategy = CompositingStrategy.Offscreen
                    val t = clock()
                    translationX = bandW * 0.045f * sin(TWO_PI * t / 53f)
                    translationY = bandH * 0.1f * sin(TWO_PI * t / 37f + 1f)
                    alpha = 0.9f + 0.1f * sin(TWO_PI * t / 29f)
                }
                .drawWithCache {
                    val centre = Offset(size.width / 2f + bandW * 0.28f, size.height / 2f + bandH * 0.08f)
                    val rx = bandW * 0.3f
                    val reach = rx * 2.2f
                    val bloom = Brush.radialGradient(
                        *Array(9) { i ->
                            val t = i / 8f
                            t to light.copy(alpha = 0.4f * exp(-(2.2f * t) * (2.2f * t)))
                        },
                        center = centre,
                        radius = reach,
                    )
                    onDrawBehind {
                        scale(1f, bandH / rx, pivot = centre) { drawCircle(bloom, radius = reach, center = centre) }
                    }
                },
        )
        // What stays still: the edges falling away as if seen through glass, and the room's own
        // dark under the text faintly lit by the picture.
        Spacer(
            Modifier
                .fillMaxSize()
                .graphicsLayer { compositingStrategy = CompositingStrategy.Offscreen }
                .drawWithCache {
                    val edges = Brush.verticalGradient(*Array(9) { i ->
                        val v = i / 8f
                        v to Color.Black.copy(alpha = 0.2f * (1f - sin(PI.toFloat() * v)))
                    })
                    val end = Brush.horizontalGradient(*Array(6) { i ->
                        val u = 0.86f + 0.14f * i / 5f
                        u to Color.Black.copy(alpha = 0.22f * smooth((u - 0.86f) / 0.14f))
                    })
                    val ground = lerp(velvet, light, 0.08f)
                    val scrim = Brush.horizontalGradient(*Array(12) { i ->
                        val u = i / 11f
                        u to ground.copy(alpha = 0.9f * (1f - smooth((u - 0.2f) / 0.52f)))
                    })
                    onDrawBehind {
                        drawRect(edges)
                        drawRect(end)
                        drawRect(scrim)
                    }
                },
        )
        // The dither adds to the finished band: in a layer of its own, Plus would add alpha too
        // and make the layer opaque. The caller composites the band as one layer, so the alpha
        // it adds never reaches the frame's edge.
        Spacer(Modifier.fillMaxSize().drawBehind { drawRect(dither, blendMode = BlendMode.Plus) })
    }
}

private const val TWO_PI = 2f * PI.toFloat()

private fun smooth(x: Float): Float {
    val t = x.coerceIn(0f, 1f)
    return t * t * (3f - 2f * t)
}
