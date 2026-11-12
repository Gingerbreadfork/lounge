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
    /** Focus also draws a 3 px `limelight` ring and the glow doubles: the high-contrast theme only. */
    val focusRing: Boolean = false,
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

        /** Cabaret: black violet walls, a pink neon sign. */
        val Cabaret = LoungeColors(
            velvet = Color(0xFF170B1E),
            curtain = Color(0xFF291634),
            curtainEdge = Color(0xFF40244F),
            limelight = Color(0xFFF6EFE7),
            dust = Color(0xFFB9A3C6),
            houselight = Color(0xFFFF8DC7),
            ember = Color(0xFFFF5A4A),
            signal = Color(0xFF7FE6C2),
            scrim = Color(0x66170B1E),
            spillScale = 1f,
            horizon = Color(0xFF1F0C24),
        )

        /**
         * Aquarium: deep petrol walls, lit by the glow of a fish tank. Its green leans to leaf so it
         * stays apart from the lamp.
         */
        val Aquarium = LoungeColors(
            velvet = Color(0xFF071517),
            curtain = Color(0xFF10262A),
            curtainEdge = Color(0xFF1D3B40),
            limelight = Color(0xFFF6EFE7),
            dust = Color(0xFF9DB8BA),
            houselight = Color(0xFF74DCE8),
            ember = Color(0xFFFF6B5E),
            signal = Color(0xFFB5E88A),
            scrim = Color(0x66071517),
            spillScale = 1f,
            horizon = Color(0xFF0A1B20),
        )

        /**
         * Absinthe: dark olive walls, a green-gold glow. Its green leans to teal so it stays apart
         * from the lamp.
         */
        val Absinthe = LoungeColors(
            velvet = Color(0xFF12140A),
            curtain = Color(0xFF22251A),
            curtainEdge = Color(0xFF363A28),
            limelight = Color(0xFFF6EFE7),
            dust = Color(0xFFB0B39A),
            houselight = Color(0xFFD2E27C),
            ember = Color(0xFFFF5E57),
            signal = Color(0xFF6FD9C0),
            scrim = Color(0x6612140A),
            spillScale = 1f,
            horizon = Color(0xFF181A0C),
        )

        /** Ultraviolet: a room near black, lit by a blacklight. */
        val Ultraviolet = LoungeColors(
            velvet = Color(0xFF0D0C12),
            curtain = Color(0xFF1B1A26),
            curtainEdge = Color(0xFF2C2A3D),
            limelight = Color(0xFFF6EFE7),
            dust = Color(0xFFA9A6C4),
            houselight = Color(0xFFB9A2FF),
            ember = Color(0xFFFF5577),
            signal = Color(0xFF7CF0C8),
            scrim = Color(0x660D0C12),
            spillScale = 1f,
            horizon = Color(0xFF120F1C),
        )

        // Both set their horizon too: a copy would keep House's plum one.
        val Blackout = House.copy(
            velvet = Color(0xFF000000),
            horizon = Color(0xFF000000),
            curtain = Color(0xFF161016),
            scrim = Color(0x66000000),
            spillScale = 0.5f,
        )

        val HighContrast = House.copy(
            velvet = Color(0xFF000000),
            horizon = Color(0xFF000000),
            dust = Color(0xFFD5CBD1),
            spillScale = 1f,
            focusRing = true,
        )
    }
}

val LocalLoungeColors = staticCompositionLocalOf { LoungeColors.Dusk }

fun Color.toArgbInt(): Int = (this.value shr 32).toInt()

fun argb(value: UInt): Color = Color(value.toLong() and 0xFFFFFFFFL)
