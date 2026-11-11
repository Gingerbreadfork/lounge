package tv.lounge.design

import java.time.LocalTime
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class WordsTest {
    private val minute = 60_000L

    @Test
    fun clocksFollowTheTwentyFourHourSetting() {
        assertEquals("21:42", Words.clock(LocalTime.of(21, 42), is24Hour = true))
        assertEquals("07:05", Words.clock(LocalTime.of(7, 5), is24Hour = true))
        assertEquals("9:42 pm", Words.clock(LocalTime.of(21, 42), is24Hour = false))
        assertEquals("12:00 pm", Words.clock(LocalTime.NOON, is24Hour = false))
        assertEquals("12:30 am", Words.clock(LocalTime.of(0, 30), is24Hour = false))
        assertEquals("9:42", Words.clock(LocalTime.of(21, 42), is24Hour = false, period = false))
    }

    @Test
    fun agoReadsAsItDoesOnThePhone() {
        assertEquals("just now", Words.ago(30_000))
        assertEquals("just now", Words.ago(-5 * minute))
        assertEquals("1 minute ago", Words.ago(minute))
        assertEquals("3 minutes ago", Words.ago(3 * minute))
        assertEquals("1 hour ago", Words.ago(61 * minute))
        assertEquals("5 hours ago", Words.ago(5 * 60 * minute))
        assertEquals("1 day ago", Words.ago(30 * 60 * minute))
        assertEquals("3 days ago", Words.ago(3 * 24 * 60 * minute))
    }

    @Test
    fun countsAgreeWithTheirNumber() {
        assertEquals("1 channel", Words.counted(1, "channel", "channels"))
        assertEquals("0 channels", Words.counted(0, "channel", "channels"))
        assertEquals("12 channels", Words.counted(12, "channel", "channels"))
    }

    @Test
    fun longerToastsStayLonger() {
        assertEquals(4_000L, toastReadingMillis("Hidden from every row"))
        val sentence = "Nothing on this TV can open Alien: Earth. Is its app still installed? Check, then try again."
        assertTrue(toastReadingMillis(sentence) > 4_000L)
        assertEquals(10_000L, toastReadingMillis("x".repeat(1_000)))
    }

    @Test
    fun theRimOnAPrimaryButtonStandsOffItsFill() {
        /** [rim] as it lands on [fill] at the rim's strength. */
        fun drawn(fill: androidx.compose.ui.graphics.Color, rim: androidx.compose.ui.graphics.Color) = androidx.compose.ui.graphics.Color(
            red = fill.red + (rim.red - fill.red) * RIM_ALPHA,
            green = fill.green + (rim.green - fill.green) * RIM_ALPHA,
            blue = fill.blue + (rim.blue - fill.blue) * RIM_ALPHA,
        )
        // On each room's lamp, which fills a primary button with no artwork behind it.
        val rooms = with(LoungeColors) { listOf(Dusk, House, Hearth, Opera, Marquee, GreenRoom, Moonlight, Graphite, Blackout, HighContrast) }
        for (colors in rooms) {
            val fill = colors.houselight
            assertTrue(fill.toString(), contrastRatio(fill, drawn(fill, textOn(fill, colors))) >= 3f)
        }
        // On any artwork's light it is never fainter than a limelight rim would be.
        val colors = LoungeColors.Dusk
        for (argb in listOf(0xFF4A6FA5, 0xFF7A4FD0, 0xFFB03A48, 0xFF2E8B57, 0xFF3366FF, 0xFFC4D9F2)) {
            val fill = androidx.compose.ui.graphics.Color(argb)
            assertTrue("%08x".format(argb), contrastRatio(fill, drawn(fill, textOn(fill, colors))) >= contrastRatio(fill, drawn(fill, colors.limelight)))
        }
    }

    @Test
    fun textOnALightIsWhicheverReadsBetter() {
        val colors = LoungeColors.Dusk
        for (argb in listOf(0xFF4A6FA5, 0xFF7A4FD0, 0xFFB03A48, 0xFFFFA86B, 0xFF2E8B57, 0xFF3366FF)) {
            val light = androidx.compose.ui.graphics.Color(argb)
            val chosen = contrastRatio(light, textOn(light, colors))
            val other = contrastRatio(light, if (textOn(light, colors) == colors.velvet) colors.limelight else colors.velvet)
            assertTrue("%08x".format(argb), chosen >= other)
        }
    }
}
