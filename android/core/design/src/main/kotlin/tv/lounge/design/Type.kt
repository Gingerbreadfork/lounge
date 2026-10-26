package tv.lounge.design

import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.text.PlatformTextStyle
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontSynthesis
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.font.createFontFamilyResolver
import androidx.compose.ui.text.style.LineHeightStyle
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp

/**
 * Two families (DESIGN.md §4): Bricolage Grotesque for display, Figtree for text.
 * Sizes are in sp at 1080p; the system density scales them for 4K-rendered UIs.
 */
object LoungeFonts {
    @Volatile private var shared: FontFamily.Resolver? = null

    /** The process-wide resolver; [warmUp] fills its cache so first-frame text never parses a font. */
    fun resolver(context: android.content.Context): FontFamily.Resolver =
        shared ?: synchronized(this) { shared ?: createFontFamilyResolver(context.applicationContext).also { shared = it } }

    /**
     * Resolves every typeface the typography uses, off the main thread, so the first frame never
     * creates a font instance.
     */
    fun warmUp(context: android.content.Context) {
        val r = resolver(context)
        val t = LoungeTypography.Default
        for (style in listOf(t.displayXl, t.displayL, t.headline, t.clock, t.title, t.body, t.meta, t.caption)) {
            runCatching {
                r.resolve(style.fontFamily, style.fontWeight ?: FontWeight.Normal, style.fontStyle ?: FontStyle.Normal, FontSynthesis.All).value
            }
        }
    }

    /**
     * Static instances cut from the variable fonts at build time (tools/fonts), one per style; far
     * cheaper to create than a variable-font instance.
     */
    fun bricolage(weight: Int, opticalSize: Float): FontFamily {
        val res = when {
            opticalSize >= 90f -> R.font.bricolage_display_xl
            opticalSize >= 45f -> R.font.bricolage_display_l
            opticalSize >= 38f -> R.font.bricolage_clock
            else -> R.font.bricolage_headline
        }
        return FontFamily(Font(resId = res, weight = FontWeight(weight)))
    }

    fun figtree(weight: Int): FontFamily {
        val res = when {
            weight >= 600 -> R.font.figtree_semibold
            weight >= 500 -> R.font.figtree_medium
            else -> R.font.figtree_regular
        }
        return FontFamily(Font(resId = res, weight = FontWeight(weight)))
    }
}

private val noPadding = PlatformTextStyle(includeFontPadding = false)
private val centred = LineHeightStyle(LineHeightStyle.Alignment.Center, LineHeightStyle.Trim.None)

@Immutable
data class LoungeTypography(
    val displayXl: TextStyle,
    val displayL: TextStyle,
    val headline: TextStyle,
    val clock: TextStyle,
    val title: TextStyle,
    val body: TextStyle,
    val meta: TextStyle,
    val caption: TextStyle,
) {
    companion object {
        val Default = LoungeTypography(
            displayXl = TextStyle(
                fontFamily = LoungeFonts.bricolage(700, 96f),
                fontWeight = FontWeight(700),
                fontSize = 72.sp,
                lineHeight = 76.sp,
                letterSpacing = (-0.02).em,
                platformStyle = noPadding,
                lineHeightStyle = centred,
            ),
            displayL = TextStyle(
                fontFamily = LoungeFonts.bricolage(650, 48f),
                fontWeight = FontWeight(650),
                fontSize = 48.sp,
                lineHeight = 54.sp,
                letterSpacing = (-0.015).em,
                platformStyle = noPadding,
                lineHeightStyle = centred,
            ),
            headline = TextStyle(
                fontFamily = LoungeFonts.bricolage(600, 34f),
                fontWeight = FontWeight(600),
                fontSize = 34.sp,
                lineHeight = 40.sp,
                letterSpacing = (-0.01).em,
                platformStyle = noPadding,
                lineHeightStyle = centred,
            ),
            clock = TextStyle(
                fontFamily = LoungeFonts.bricolage(500, 40f),
                fontWeight = FontWeight(500),
                fontSize = 40.sp,
                lineHeight = 40.sp,
                platformStyle = noPadding,
                lineHeightStyle = centred,
                fontFeatureSettings = "tnum",
            ),
            title = TextStyle(
                fontFamily = LoungeFonts.figtree(600),
                fontWeight = FontWeight(600),
                fontSize = 28.sp,
                lineHeight = 34.sp,
                platformStyle = noPadding,
                lineHeightStyle = centred,
            ),
            body = TextStyle(
                fontFamily = LoungeFonts.figtree(400),
                fontWeight = FontWeight(400),
                fontSize = 26.sp,
                lineHeight = 34.sp,
                platformStyle = noPadding,
                lineHeightStyle = centred,
            ),
            meta = TextStyle(
                fontFamily = LoungeFonts.figtree(500),
                fontWeight = FontWeight(500),
                fontSize = 24.sp,
                lineHeight = 30.sp,
                platformStyle = noPadding,
                lineHeightStyle = centred,
                fontFeatureSettings = "tnum",
            ),
            caption = TextStyle(
                fontFamily = LoungeFonts.figtree(500),
                fontWeight = FontWeight(500),
                fontSize = 22.sp,
                lineHeight = 26.sp,
                letterSpacing = 0.01.em,
                platformStyle = noPadding,
                lineHeightStyle = centred,
            ),
        )

        /** High contrast: the running text one cut heavier, from the Figtree cuts that ship. */
        val Bold = Default.copy(
            body = Default.body.copy(fontFamily = LoungeFonts.figtree(500), fontWeight = FontWeight(500)),
            meta = Default.meta.copy(fontFamily = LoungeFonts.figtree(600), fontWeight = FontWeight(600)),
            caption = Default.caption.copy(fontFamily = LoungeFonts.figtree(600), fontWeight = FontWeight(600)),
        )
    }
}

val LocalLoungeTypography = staticCompositionLocalOf { LoungeTypography.Default }
