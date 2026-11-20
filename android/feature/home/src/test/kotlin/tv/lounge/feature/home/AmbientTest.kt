package tv.lounge.feature.home

import androidx.compose.ui.unit.IntOffset
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class AmbientTest {
    private fun slide(art: String) = AmbientSlide(art, art, "Pictures")

    @Test
    fun picturesCarryOnFromTheLastOneShown() {
        val slides = listOf(slide("a"), slide("b"), slide("c"))
        assertEquals("a", nextSlide(slides, null)?.art)
        assertEquals("c", nextSlide(slides, "b")?.art)
        assertEquals("a", nextSlide(slides, "c")?.art)
        // A picture that has gone starts the round again.
        assertEquals("a", nextSlide(slides, "gone")?.art)
        assertNull(nextSlide(emptyList(), "a"))
    }

    @Test
    fun theMixShowsEverySceneOnceBeforeAnyComesRoundAgain() {
        val ids = listOf("lounge:ink", "lounge:rain", "plugin:x:smoke", "local:a.agsl")
        val random = kotlin.random.Random(7)
        var shown = emptyList<String>()
        val round = List(ids.size) {
            val (pick, now) = ShaderStyles.drawFromMix(ids, shown, random)!!
            shown = now
            pick
        }
        assertEquals(ids.toSet(), round.toSet())
        // The next round does not open with the scene the last one closed on.
        repeat(20) {
            val (pick, now) = ShaderStyles.drawFromMix(ids, shown, random)!!
            assertTrue(pick != shown.last())
            assertEquals(listOf(pick), now)
        }
        assertNull(ShaderStyles.drawFromMix(emptyList(), shown, random))
        assertEquals("only", ShaderStyles.drawFromMix(listOf("only"), listOf("only"), random)!!.first)
    }

    @Test
    fun theMixForgetsScenesThatHaveGoneAndPassesOverTheOneOnScreen() {
        val random = kotlin.random.Random(3)
        val (pick, now) = ShaderStyles.drawFromMix(listOf("a", "b", "c"), listOf("gone", "a"), random)!!
        assertTrue(pick == "b" || pick == "c")
        assertEquals(listOf("a", pick), now)
        // The scene on screen is given as shown, so a preview does not draw it twice running.
        repeat(20) { assertEquals("b", ShaderStyles.drawFromMix(listOf("a", "b"), listOf("a"), random)!!.first) }
    }

    @Test
    fun theMixIsOfferedOnlyWithScenesToChooseFrom() {
        fun scene(id: String) = ShaderStyle(id, id, null, "", emptyList(), "Lounge")
        assertEquals(listOf(ShaderStyles.PICTURES), styleChoices(emptyList()).map { it.first })
        assertEquals(listOf(ShaderStyles.PICTURES, "a"), styleChoices(listOf(scene("a"))).map { it.first })
        assertEquals(listOf(ShaderStyles.PICTURES, ShaderStyles.MIX, "a", "b"), styleChoices(listOf(scene("a"), scene("b"))).map { it.first })
    }

    @Test
    fun theHouseLightsGoDownAtNight() {
        assertEquals(1f, nightLights(12f), 0f)
        assertEquals(1f, nightLights(22f), 0f)
        assertEquals(0.9f, nightLights(22.5f), 0.001f)
        assertEquals(0.8f, nightLights(23f), 0.001f)
        assertEquals(0.8f, nightLights(0f), 0.001f)
        assertEquals(0.8f, nightLights(3.5f), 0.001f)
        assertEquals(0.8f, nightLights(6f), 0.001f)
        assertEquals(0.9f, nightLights(6.5f), 0.001f)
        assertEquals(1f, nightLights(7f), 0.001f)
        assertEquals(nightLights(1f), nightLights(25f), 0f)
    }

    @Test
    fun theClockNeverRestsAndStaysInItsBox() {
        assertEquals(IntOffset.Zero, captionDrift(0))
        var seenFar = false
        for (m in 0 until 24 * 60) {
            val at = captionDrift(m)
            assertTrue("minute $m: $at", at.x in 0..48 && at.y in 0..24)
            assertTrue("minute $m rests", at != captionDrift(m + 1))
            if (at.x >= 46) seenFar = true
        }
        assertTrue(seenFar)
    }
}
