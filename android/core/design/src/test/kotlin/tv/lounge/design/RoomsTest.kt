package tv.lounge.design

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class RoomsTest {
    @Test
    fun everyRoomClearsTheSameBar() {
        // DESIGN.md §3.1: the text, the lamp and the two semantic colours, each on the room's wall.
        for (room in LoungeThemeVariant.entries) {
            val c = room.colors()
            fun check(what: String, color: androidx.compose.ui.graphics.Color, at: Float) {
                val ratio = contrastRatio(color, c.velvet)
                assertTrue("$room $what is $ratio, under $at", ratio >= at)
            }
            check("limelight", c.limelight, 15.8f)
            check("dust", c.dust, 6.9f)
            check("houselight", c.houselight, 7.4f)
            check("ember", c.ember, 5.2f)
            check("signal", c.signal, 10.4f)
        }
    }

    @Test
    fun everyRoomKeepsTheSameWhite() {
        val white = LoungeColors.Dusk.limelight
        for (room in LoungeThemeVariant.entries) assertEquals(room.toString(), white, room.colors().limelight)
    }
}
