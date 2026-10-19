package tv.lounge.design

import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color

/** The house palette (DESIGN.md §3.1). Dynamic artwork light is never paint; see [Light]. */
@Immutable
data class LoungeColors(
    val velvet: Color,
    val curtain: Color,
    val curtainEdge: Color,
    val limelight: Color,
    val dust: Color,
    val houselight: Color,
    val ember: Color,
    val signal: Color,
    val scrim: Color,
    /** Multiplier applied to light-spill alpha; halved on the OLED theme. */
    val spillScale: Float,
    /** The backdrop's far corner: the light field runs `velvet` into this toward the bottom right. */
    val horizon: Color = velvet,
) {
    /** A tile's light. The core's fallback is tungsten; here it becomes this theme's own lamp. */
    fun light(argb: UInt): Color = if (argb == CORE_FALLBACK_LIGHT) houselight else argb(argb)

    companion object {
        private const val CORE_FALLBACK_LIGHT = 0xFFF2B263u

        /** Dusk (DESIGN.md §3.1): indigo walls after sunset, a coral lamp. The default. */
        val Dusk = LoungeColors(
            velvet = Color(0xFF120E27),
            curtain = Color(0xFF211A3E),
            curtainEdge = Color(0xFF342A58),
            limelight = Color(0xFFF6EFE7),
            dust = Color(0xFFA8A0C2),
            houselight = Color(0xFFFFA86B),
            ember = Color(0xFFFF5468),
            signal = Color(0xFF7CDFB3),
            scrim = Color(0x66120E27),
            spillScale = 1f,
            horizon = Color(0xFF1C1130),
        )

        /** House: the warm plum room under tungsten light. */
        val House = LoungeColors(
            velvet = Color(0xFF1A1016),
            curtain = Color(0xFF2A1B24),
            curtainEdge = Color(0xFF3B2A34),
            limelight = Color(0xFFF6EFE7),
            dust = Color(0xFFA89AA3),
            houselight = Color(0xFFF2B263),
            ember = Color(0xFFE35D5B),
            signal = Color(0xFF7FD3A8),
            scrim = Color(0x661A1016),
            spillScale = 1f,
        )

        /** Hearth: umber walls, the room lit by a fire. */
        val Hearth = LoungeColors(
            velvet = Color(0xFF17100D),
            curtain = Color(0xFF291C17),
            curtainEdge = Color(0xFF3D2B23),
            limelight = Color(0xFFF6EFE7),
            dust = Color(0xFFB3A196),
            houselight = Color(0xFFFF7E4D),
            ember = Color(0xFFFF4A52),
            signal = Color(0xFF8ED9A0),
            scrim = Color(0x6617100D),
            spillScale = 1f,
            horizon = Color(0xFF22130D),
        )

        /** Opera: red velvet and gilt. */
        val Opera = LoungeColors(
            velvet = Color(0xFF1E0B12),
            curtain = Color(0xFF311520),
            curtainEdge = Color(0xFF48222F),
            limelight = Color(0xFFF6EFE7),
            dust = Color(0xFFB89DA6),
            houselight = Color(0xFFF3D08C),
            ember = Color(0xFFFF5C63),
            signal = Color(0xFF86DBAE),
            scrim = Color(0x661E0B12),
            spillScale = 1f,
            horizon = Color(0xFF26101A),
        )

        /** Marquee: navy night, incandescent bulbs. */
        val Marquee = LoungeColors(
            velvet = Color(0xFF0C1226),
            curtain = Color(0xFF172040),
            curtainEdge = Color(0xFF26305A),
            limelight = Color(0xFFF6EFE7),
            dust = Color(0xFFA1A9C4),
            houselight = Color(0xFFFFC85C),
            ember = Color(0xFFFF5A6A),
            signal = Color(0xFF7FE0B6),
            scrim = Color(0x660C1226),
            spillScale = 1f,
            horizon = Color(0xFF141A30),
        )

        /** Green room: deep green, a brass lamp. */
        val GreenRoom = LoungeColors(
            velvet = Color(0xFF0E1814),
            curtain = Color(0xFF192822),
            curtainEdge = Color(0xFF283B33),
            limelight = Color(0xFFF6EFE7),
            dust = Color(0xFFA0B2A8),
            houselight = Color(0xFFE9C47F),
            ember = Color(0xFFFF6260),
            signal = Color(0xFF8FE8BC),
            scrim = Color(0x660E1814),
            spillScale = 1f,
            horizon = Color(0xFF131F1C),
        )

        /** Moonlight: cool slate, the moon through the window. */
        val Moonlight = LoungeColors(
            velvet = Color(0xFF10161F),
            curtain = Color(0xFF1B2430),
            curtainEdge = Color(0xFF2B3747),
            limelight = Color(0xFFF6EFE7),
            dust = Color(0xFFA3ADBB),
            houselight = Color(0xFFC4D9F2),
            ember = Color(0xFFFF6773),
            signal = Color(0xFF86DEB1),
            scrim = Color(0x6610161F),
            spillScale = 1f,
            horizon = Color(0xFF151C27),
        )

        /** Graphite: no colour at all, a warm white lamp. */
        val Graphite = LoungeColors(
            velvet = Color(0xFF141414),
            curtain = Color(0xFF222222),
            curtainEdge = Color(0xFF343434),
            limelight = Color(0xFFF6EFE7),
            dust = Color(0xFFA6A3A0),
            houselight = Color(0xFFE8DFD4),
            ember = Color(0xFFF25C5C),
            signal = Color(0xFF86D9AC),
            scrim = Color(0x66141414),
            spillScale = 1f,
            horizon = Color(0xFF191919),
        )

        val Blackout = House.copy(
            velvet = Color(0xFF000000),
            curtain = Color(0xFF161016),
            scrim = Color(0x66000000),
            spillScale = 0.5f,
        )

        val HighContrast = House.copy(
            velvet = Color(0xFF000000),
            dust = Color(0xFFD5CBD1),
            spillScale = 1f,
        )
    }
}

val LocalLoungeColors = staticCompositionLocalOf { LoungeColors.Dusk }

fun Color.toArgbInt(): Int = (this.value shr 32).toInt()

fun argb(value: UInt): Color = Color(value.toLong() and 0xFFFFFFFFL)
