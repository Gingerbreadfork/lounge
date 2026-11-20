package tv.lounge.feature.home

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import tv.lounge.platform.PanelSize

class SceneStepsTest {
    private val uhd = PanelSize(3840, 2160)
    private val plain = SceneLadder.saver(1920, 1080, uhd, fine = false)
    private val fine = SceneLadder.saver(1920, 1080, uhd, fine = true)
    private val live = SceneLadder.live(1920, 1080)

    private fun SceneSteps.slow(times: Int = 1, ms: Float = 30f): StepChange {
        var last = StepChange.None
        repeat(times) { last = window(ms) }
        return last
    }

    @Test
    fun laddersStartAtHalfTheWindow() {
        assertEquals(0, plain.base)
        assertEquals(SceneRung(960, 540, 1920, 1080, 33L), plain.rungs[plain.base])
        assertEquals(SceneRung(320, 180, 1920, 1080, 50L), plain.rungs.last())
        // A fine scene may climb to the window's size and the panel's, and starts where the rest do.
        assertEquals(SceneRung(3840, 2160, 3840, 2160, 33L), fine.rungs.first())
        assertEquals(SceneRung(1920, 1080, 1920, 1080, 33L), fine.rungs[2])
        assertEquals(SceneRung(960, 540, 1920, 1080, 33L), fine.rungs[fine.base])
        // On a panel no larger than the window there is no rung above the window's size.
        assertEquals(2, SceneLadder.saver(1920, 1080, PanelSize(1920, 1080), fine = true).base)
        // The live background is finished at the scene's own size.
        assertEquals(SceneRung(960, 540, 960, 540, 33L), live.rungs.first())
        assertEquals(4, live.rungs.size)
    }

    @Test
    fun aStepTakesTwoSlowWindowsInARow() {
        val s = SceneSteps(plain, null)
        assertEquals(StepChange.None, s.slow())
        assertEquals(StepChange.None, s.window(20f))
        assertEquals(StepChange.None, s.slow())
        assertEquals(StepChange.Stepped, s.slow())
        assertEquals(1, s.step)
        assertEquals(SceneRecord(1, fails = 1), s.record)
    }

    @Test
    fun aRateThatCannotHelpIsPassedOver() {
        val s = SceneSteps(plain, null)
        // 60 ms is over what 20 frames a second allows at this size, so the picture shrinks.
        assertEquals(StepChange.Stepped, s.slow(2, ms = 60f))
        assertEquals(2, s.step)
        assertEquals(640, s.rung.sceneWidth)
    }

    @Test
    fun aLaterStartTriesTheRungAboveAndKeepsItWhenItHolds() {
        val s = SceneSteps(plain, SceneRecord(1, fails = 0, starts = 0))
        assertTrue(s.trying)
        assertEquals(0, s.step)
        assertEquals(StepChange.None, s.window(20f))
        assertEquals(StepChange.Kept, s.window(20f))
        assertFalse(s.trying)
        assertEquals(SceneRecord(0), s.record)
    }

    @Test
    fun aFailedTryGoesBackAndWaitsLonger() {
        val s = SceneSteps(plain, SceneRecord(1, fails = 1, starts = 1))
        assertTrue(s.trying)
        assertEquals(StepChange.Stepped, s.window(26f))
        assertEquals(1, s.step)
        assertEquals(SceneRecord(1, fails = 2, starts = 0), s.record)
        // Three more starts pass before the next try, which comes on the fourth.
        var record = s.record
        repeat(3) {
            val next = SceneSteps(plain, record)
            assertFalse(next.trying)
            assertEquals(1, next.step)
            record = next.record
        }
        assertTrue(SceneSteps(plain, record).trying)
    }

    @Test
    fun aTryFarOverBudgetWaitsMuchLonger() {
        val s = SceneSteps(fine, SceneRecord(fine.base, fails = 0, starts = 0))
        assertTrue(s.trying)
        assertEquals(1920, s.rung.sceneWidth)
        assertEquals(StepChange.Stepped, s.stalled(200f))
        assertEquals(fine.base, s.step)
        assertEquals(3, s.record.fails)
    }

    @Test
    fun aSceneNewToTheTvDoesNotTryAbove() {
        val s = SceneSteps(fine, null)
        assertFalse(s.trying)
        assertEquals(fine.base, s.step)
        // A scene with nothing above its place has nothing to try.
        assertFalse(SceneSteps(plain, SceneRecord(0)).trying)
    }

    @Test
    fun theScreensaverRemembersASceneTooHeavyAndTheBackgroundDoesNot() {
        val last = plain.rungs.lastIndex
        val s = SceneSteps(plain, SceneRecord(last, fails = 6))
        assertEquals(StepChange.Unusable, s.slow(2, ms = 40f))
        assertTrue(s.unusable)
        assertEquals(SceneRecord.HEAVY, s.record.problem)
        // It is passed over until a try is due, and that try starts at the smallest rung.
        val later = SceneSteps(plain, s.record)
        assertTrue(later.unusable)
        val due = SceneSteps(plain, s.record.copy(starts = 63))
        assertTrue(due.trying)
        assertEquals(last, due.step)
        assertEquals(StepChange.Unusable, due.window(40f))

        val b = SceneSteps(live, SceneRecord(live.rungs.lastIndex, fails = 6))
        assertEquals(StepChange.Unusable, b.hopeless(600f))
        assertEquals(live.rungs.lastIndex, b.record.step)
        assertNull(b.record.problem)
    }

    @Test
    fun aBrokenProgramIsNeverTriedAgain() {
        val s = SceneSteps(plain, null)
        assertEquals(StepChange.Unusable, s.broken())
        val again = SceneSteps(plain, s.record.copy(starts = 1000))
        assertTrue(again.unusable)
        assertFalse(again.trying)
    }

    @Test
    fun recordsSurviveBeingWritten() {
        for (r in listOf(SceneRecord(0), SceneRecord(3, 2, 5), SceneRecord(10, 6, 0, SceneRecord.HEAVY), SceneRecord(4, problem = SceneRecord.BROKEN))) {
            assertEquals(r, SceneRecord.decode(r.encode()))
        }
        assertNull(SceneRecord.decode(null))
        assertNull(SceneRecord.decode("2"))
        assertNull(SceneRecord.decode("a:b:c:"))
    }
}
