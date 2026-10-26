package tv.lounge.design

import androidx.compose.foundation.LocalIndication
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFontFamilyResolver
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.tv.material3.LocalContentColor
import androidx.tv.material3.LocalTextStyle

enum class LoungeThemeVariant { Dusk, House, Hearth, Opera, Marquee, GreenRoom, Moonlight, Graphite, Blackout, HighContrast }

fun LoungeThemeVariant.colors(): LoungeColors = when (this) {
    LoungeThemeVariant.Dusk -> LoungeColors.Dusk
    LoungeThemeVariant.House -> LoungeColors.House
    LoungeThemeVariant.Hearth -> LoungeColors.Hearth
    LoungeThemeVariant.Opera -> LoungeColors.Opera
    LoungeThemeVariant.Marquee -> LoungeColors.Marquee
    LoungeThemeVariant.GreenRoom -> LoungeColors.GreenRoom
    LoungeThemeVariant.Moonlight -> LoungeColors.Moonlight
    LoungeThemeVariant.Graphite -> LoungeColors.Graphite
    LoungeThemeVariant.Blackout -> LoungeColors.Blackout
    LoungeThemeVariant.HighContrast -> LoungeColors.HighContrast
}

/** Per-tier visual budget (DESIGN.md §12). */
@Immutable
data class LoungeVisuals(
    val spill: Boolean,
    val glowAlpha: Float,
    val parallax: Boolean,
    val shaderWallpaper: Boolean,
    val previews: Boolean,
    /** A Tier A TV, with room for the faster light clock, more prefetching and a smoother push-in. */
    val tierA: Boolean = false,
) {
    companion object {
        val TierA = LoungeVisuals(spill = true, glowAlpha = 0.34f, parallax = true, shaderWallpaper = true, previews = true, tierA = true)
        val TierB = LoungeVisuals(spill = true, glowAlpha = 0.34f, parallax = true, shaderWallpaper = true, previews = true)
        val Lite = LoungeVisuals(spill = false, glowAlpha = 0.24f, parallax = false, shaderWallpaper = false, previews = false)
    }
}

val LocalLoungeVisuals = staticCompositionLocalOf { LoungeVisuals.TierA }

/** Layout constants at 1080p (DESIGN.md §5). */
object Dimens {
    val safeX: Dp = 96.dp
    val safeY: Dp = 54.dp
    val topBarHeight: Dp = 48.dp
    val gutter: Dp = 24.dp
    val rowGap: Dp = 32.dp
    /** A 40 px header line plus 18 px of clear air, so a focused tile lifts into space, not type. */
    val rowHeaderHeight: Dp = 58.dp
    /** Light room every focusable container reserves on each side; light is never clipped. */
    val glowRoom: Dp = 56.dp
    val tileRadius: Dp = 12.dp
    val stageRadius: Dp = 20.dp
    val sheetWidth: Dp = 560.dp
    val focusLineY: Dp = 334.dp
    val stageArtworkWidth: Dp = 1120.dp
    val stageArtworkHeight: Dp = 630.dp
    val playbillWidth: Dp = 560.dp
    val collapsedStageHeight: Dp = 200.dp
    val stripTileWidth: Dp = 232.dp
    val stripTileHeight: Dp = 130.dp
    val glowRadius: Dp = 32.dp
    val spillRadius: Dp = 1100.dp
}

object LoungeTheme {
    val colors: LoungeColors
        @Composable get() = LocalLoungeColors.current
    val typography: LoungeTypography
        @Composable get() = LocalLoungeTypography.current
    val motion: LoungeMotion
        @Composable get() = LocalLoungeMotion.current
    val visuals: LoungeVisuals
        @Composable get() = LocalLoungeVisuals.current
}

@Composable
fun LoungeTheme(
    variant: LoungeThemeVariant = LoungeThemeVariant.Dusk,
    reducedMotion: Boolean = false,
    visuals: LoungeVisuals = LoungeVisuals.TierA,
    content: @Composable () -> Unit,
) {
    val colors = variant.colors()
    val lit = if (colors.focusRing) visuals.copy(glowAlpha = (visuals.glowAlpha * 2f).coerceAtMost(1f)) else visuals
    val type = if (colors.focusRing) LoungeTypography.Bold else LoungeTypography.Default
    val motion = if (reducedMotion) LoungeMotion.Reduced else LoungeMotion.Default
    val designDensity = designDensity()
    val fontResolver = LoungeFonts.resolver(LocalContext.current)
    CompositionLocalProvider(
        LocalDensity provides designDensity,
        LocalFontFamilyResolver provides fontResolver,
        LocalLoungeColors provides colors,
        LocalLoungeTypography provides type,
        LocalLoungeMotion provides motion,
        LocalLoungeVisuals provides lit,
        LocalContentColor provides colors.limelight,
        LocalTextStyle provides type.body,
        LocalIndication provides NoIndication,
    ) {
        // The velvet fill stays even under the light field: the field is a scaled offscreen layer,
        // and without an opaque fill beneath it the GPU leaves last frame's pixels where nothing drew.
        Box(Modifier.fillMaxSize().background(colors.velvet)) { content() }
    }
}

/**
 * Every number in DESIGN.md is a pixel on a 1920-wide canvas. This density makes one design pixel
 * one physical pixel at 1080p and two at 4K. Font scale is kept.
 */
@Composable
private fun designDensity(): Density {
    val system = LocalDensity.current
    val widthPx = LocalWindowInfo.current.containerSize.width
    val scale = if (widthPx > 0) widthPx / 1920f else system.density / 2f
    return Density(density = scale, fontScale = system.fontScale)
}

/** Reads the system's reduced-motion preference (animator duration scale of 0). */
@Composable
fun systemReducedMotion(): Boolean {
    val context = LocalContext.current
    val scale = android.provider.Settings.Global.getFloat(
        context.contentResolver,
        android.provider.Settings.Global.ANIMATOR_DURATION_SCALE,
        1f,
    )
    return scale == 0f
}

/** Focus is expressed by light, never by Material ripples or rings. */
private object NoIndication : androidx.compose.foundation.IndicationNodeFactory {
    override fun create(interactionSource: androidx.compose.foundation.interaction.InteractionSource): androidx.compose.ui.Modifier.Node =
        object : androidx.compose.ui.Modifier.Node() {}

    override fun equals(other: Any?): Boolean = other === this
    override fun hashCode(): Int = -1
}

@Suppress("unused")
val Transparent: Color = Color.Transparent
