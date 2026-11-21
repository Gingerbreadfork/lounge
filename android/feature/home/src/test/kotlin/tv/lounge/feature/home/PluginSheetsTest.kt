package tv.lounge.feature.home

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import tv.lounge.core.PluginSheetRow
import tv.lounge.core.PluginSheetRowKind

/** The rules a plugin's sheet follows: where focus goes, and how a row that asks first is armed. */
class PluginSheetsTest {
    private fun row(id: String, kind: PluginSheetRowKind) =
        PluginSheetRow(id = id, glyph = null, title = id, subtitle = null, value = null, on = null, kind = kind)

    private val rows = listOf(
        row("lights", PluginSheetRowKind.Group),
        row("note", PluginSheetRowKind.Text),
        row("lamp", PluginSheetRowKind.Press("toggle:lamp")),
        row("level", PluginSheetRowKind.Step(down = "dim", up = "brighten", press = null)),
        row("more", PluginSheetRowKind.Open("more")),
        row("unlock", PluginSheetRowKind.Confirm("unlock:door", "Unlock the door?")),
    )

    @Test
    fun focusRestsOnlyOnRowsThatDoSomething() {
        assertEquals(listOf("lamp", "level", "more", "unlock"), rows.filter { it.takesFocus() }.map { it.id })
    }

    @Test
    fun aSheetOpensOnItsFirstRowOrTheOneBackCameFrom() {
        assertEquals("lamp", firstFocusRow(rows, cameFrom = null))
        assertEquals("more", firstFocusRow(rows, cameFrom = "more"))
        // A sheet none of the rows leads to opens on the first, as any other.
        assertEquals("lamp", firstFocusRow(rows, cameFrom = "elsewhere"))
        assertNull(firstFocusRow(rows.take(2), cameFrom = null))
        // Back from a picture a row opened lands on that row, if it is still there.
        assertEquals("level", firstFocusRow(rows, cameFrom = null, leftOn = "level"))
        assertEquals("more", firstFocusRow(rows, cameFrom = "more", leftOn = "gone"))
        assertEquals("lamp", firstFocusRow(rows, cameFrom = null, leftOn = "note"))
    }

    @Test
    fun aRowThatLeavesHandsFocusToTheOneInItsPlace() {
        val before = listOf("a", "b", "c")
        // Still there: focus stays where it is, wherever the row has moved to.
        assertNull(focusAfter(listOf("c", "a", "b"), focused = "b", at = 1))
        assertNull(focusAfter(before, focused = null, at = 0))
        assertEquals("c", focusAfter(listOf("a", "c"), focused = "b", at = 1))
        // The last row gone leaves focus on the new last.
        assertEquals("b", focusAfter(listOf("a", "b"), focused = "c", at = 2))
        assertNull(focusAfter(emptyList(), focused = "a", at = 0))
    }

    @Test
    fun aRowThatAsksActsOnTheSecondPressOnly() {
        val confirm = ConfirmArm()
        assertFalse(confirm.press("unlock", "unlock:door"))
        assertTrue(confirm.waiting)
        assertTrue(confirm.armedFor("unlock", "unlock:door"))
        assertTrue(confirm.press("unlock", "unlock:door"))
        // Having acted, it asks again the next time.
        assertFalse(confirm.waiting)
        assertFalse(confirm.press("unlock", "unlock:door"))
    }

    @Test
    fun theQuestionIsTakenBackByLeavingAndByAChangeOfAction() {
        val confirm = ConfirmArm()
        confirm.press("unlock", "unlock:door")
        // Another row losing focus changes nothing; this one leaving takes the question back.
        confirm.leave("lamp")
        assertTrue(confirm.waiting)
        confirm.leave("unlock")
        assertFalse(confirm.waiting)

        // The door locked itself meanwhile, so the row now locks: the press asks afresh.
        confirm.press("unlock", "unlock:door")
        assertFalse(confirm.armedFor("unlock", "lock:door"))
        assertFalse(confirm.press("unlock", "lock:door"))
        assertTrue(confirm.armedFor("unlock", "lock:door"))

        confirm.disarm()
        assertFalse(confirm.waiting)

        // Someone else unlocks the door: the row becomes one to press, and comes back unarmed.
        confirm.press("unlock", "unlock:door")
        val asks = { now: List<PluginSheetRow> -> { id: String, action: String -> now.any { it.id == id && (it.kind as? PluginSheetRowKind.Confirm)?.action == action } } }
        confirm.settle(asks(rows))
        assertTrue(confirm.waiting)
        confirm.settle(asks(rows.dropLast(1) + row("unlock", PluginSheetRowKind.Press("lock:door"))))
        assertFalse(confirm.armedFor("unlock", "unlock:door"))
    }

    @Test
    fun aLivePictureIsHalvedUntilItFitsAndSaysWhenItHasStopped() {
        assertEquals(1, liveSampleSize(1920))
        assertEquals(1, liveSampleSize(640))
        assertEquals(2, liveSampleSize(2560))
        assertEquals(2, liveSampleSize(3840))
        assertEquals(4, liveSampleSize(4000))

        assertEquals("One moment", liveNote(hasPicture = false, resting = false, misses = 0))
        assertNull(liveNote(hasPicture = true, resting = false, misses = 0))
        // One late picture is not worth a word.
        assertNull(liveNote(hasPicture = true, resting = false, misses = 2))
        assertEquals("The picture has stopped arriving. Still trying.", liveNote(hasPicture = true, resting = false, misses = 3))
        assertEquals("The picture isn't arriving. Still trying.", liveNote(hasPicture = false, resting = false, misses = 5))
        assertEquals("Resting. Press OK to carry on.", liveNote(hasPicture = true, resting = true, misses = 0))
    }
}
