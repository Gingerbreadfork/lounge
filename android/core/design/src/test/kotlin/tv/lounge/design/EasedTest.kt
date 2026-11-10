package tv.lounge.design

import androidx.compose.animation.core.LinearEasing
import androidx.compose.runtime.BroadcastFrameClock
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.MotionDurationScale
import kotlin.coroutines.CoroutineContext
import kotlin.coroutines.EmptyCoroutineContext
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.Runnable
import kotlinx.coroutines.launch
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class EasedTest {
    private val ms = 1_000_000L

    /**
     * A room whose frames and clock the test moves by hand. What a coroutine does after a frame
     * waits for [frame] to end or for [settle], as it waits for the frame to be drawn on a device.
     */
    private class Room(extra: CoroutineContext = EmptyCoroutineContext) {
        val light = LightState()
        val frames = BroadcastFrameClock()
        var now = 1_000_000_000L
        private val waiting = ArrayDeque<Runnable>()
        private val afterTheFrame = object : CoroutineDispatcher() {
            override fun dispatch(context: CoroutineContext, block: Runnable) {
                waiting += block
            }
        }
        private val scope = CoroutineScope(afterTheFrame + frames + extra)
        private val stepper = stepper()

        init {
            light.nanoTime = { now }
        }

        fun stepper(): Job = scope.launch { light.stepFrames() }.also { settle() }

        fun settle() {
            while (true) (waiting.removeFirstOrNull() ?: break).run()
        }

        /** One frame at [nanos]: [draw] reads what a draw would, then the frame ends. */
        fun frame(nanos: Long, draw: () -> Unit = {}) {
            now = nanos
            frames.sendFrame(nanos)
            draw()
            settle()
        }

        /** A value that has been seen once and now stands at [at]. */
        fun eased(at: Float) = Eased(at).also { it.toward(at, light, 180, LinearEasing) }

        fun close() {
            stepper.cancel()
            settle()
        }
    }

    private fun room(extra: CoroutineContext = EmptyCoroutineContext, test: Room.() -> Unit) {
        val room = Room(extra)
        try {
            room.test()
        } finally {
            room.close()
        }
    }

    @Test
    fun withNothingSteppingFramesTheTargetShowsAtOnce() {
        val light = LightState()
        val eased = Eased(0f)
        eased.toward(0f, light, 180, LinearEasing)
        assertEquals(1f, eased.toward(1f, light, 180, LinearEasing), 0f)
    }

    @Test
    fun whatItShowsFirstItShowsAtOnce() = room {
        assertEquals(1f, Eased(0f).toward(1f, light, 180, LinearEasing), 0f)
        settle()
        assertFalse(frames.hasAwaiters)
    }

    @Test
    fun easesAcrossItsDurationOnTheRoomsFrames() = room {
        val eased = eased(0f)
        assertEquals(0f, eased.toward(1f, light, 180, LinearEasing), 0f)
        settle()
        frame(now + 90 * ms) { assertEquals(0.5f, eased.toward(1f, light, 180, LinearEasing), 0f) }
        frame(now + 90 * ms) { assertEquals(1f, eased.toward(1f, light, 180, LinearEasing), 0f) }
    }

    @Test
    fun framesAreAskedForOnlyWhileSomethingEases() = room {
        assertFalse(frames.hasAwaiters)
        val eased = eased(0f)
        eased.toward(1f, light, 180, LinearEasing)
        settle()
        assertTrue(frames.hasAwaiters)
        frame(now + 90 * ms) { eased.toward(1f, light, 180, LinearEasing) }
        assertTrue(frames.hasAwaiters)
        frame(now + 90 * ms) { eased.toward(1f, light, 180, LinearEasing) }
        assertFalse(frames.hasAwaiters)
    }

    @Test
    fun aNewTargetPartWayStartsFromWhereItIs() = room {
        val eased = eased(0f)
        eased.toward(1f, light, 180, LinearEasing)
        settle()
        frame(now + 90 * ms) {
            assertEquals(0.5f, eased.toward(1f, light, 180, LinearEasing), 0f)
            assertEquals(0.5f, eased.toward(0f, light, 180, LinearEasing), 0f)
        }
        frame(now + 90 * ms) { assertEquals(0.25f, eased.toward(0f, light, 180, LinearEasing), 0f) }
        frame(now + 90 * ms) { assertEquals(0f, eased.toward(0f, light, 180, LinearEasing), 0f) }
    }

    @Test
    fun anEaseThatBeginsInAFrameStartsOnThatFramesTime() = room {
        val first = eased(0f)
        val second = eased(0f)
        first.toward(1f, light, 180, LinearEasing)
        settle()
        // The clock has run on by the time the frame is drawn; the frame's own time is what counts.
        frame(now + 60 * ms) {
            now += 30 * ms
            assertEquals(0f, second.toward(1f, light, 180, LinearEasing), 0f)
        }
        frame(now + 60 * ms) { assertEquals(0.5f, second.toward(1f, light, 180, LinearEasing), 0f) }
    }

    @Test
    fun anEaseThatBeginsBetweenFramesStartsOnTheClock() = room {
        val first = eased(0f)
        val second = eased(0f)
        first.toward(1f, light, 180, LinearEasing)
        settle()
        frame(now + 16 * ms)
        // Long after that frame was drawn: its time must not be taken for now.
        now += 100 * ms
        assertEquals(0f, second.toward(1f, light, 180, LinearEasing), 0f)
        frame(now + 90 * ms) { assertEquals(0.5f, second.toward(1f, light, 180, LinearEasing), 0f) }
    }

    @Test
    fun anEaseRunsForAsLongAsTheSystemsAnimationSettingSays() = room(object : MotionDurationScale {
        override val scaleFactor = 0.5f
    }) {
        val eased = eased(0f)
        eased.toward(1f, light, 180, LinearEasing)
        settle()
        frame(now + 45 * ms) { assertEquals(0.5f, eased.toward(1f, light, 180, LinearEasing), 0f) }
        frame(now + 45 * ms) { assertEquals(1f, eased.toward(1f, light, 180, LinearEasing), 0f) }
    }

    @Test
    fun withAnimationsSwitchedOffTheTargetShowsAtOnce() = room(object : MotionDurationScale {
        override val scaleFactor = 0f
    }) {
        assertEquals(1f, eased(0f).toward(1f, light, 180, LinearEasing), 0f)
    }

    @Test
    fun aStepperThatEndsLeavesALaterOneStepping() = room {
        val later = stepper()
        val eased = eased(0f)
        // The first stepper ends once the later one has taken over; eases must still run.
        close()
        assertEquals(0f, eased.toward(1f, light, 180, LinearEasing), 0f)
        settle()
        frame(now + 90 * ms) { assertEquals(0.5f, eased.toward(1f, light, 180, LinearEasing), 0f) }
        later.cancel()
        settle()
        assertEquals(1f, eased.toward(1f, light, 180, LinearEasing), 0f)
    }

    @Test
    fun aStepperCancelledPartWayLetsTheValueArrive() = room {
        val eased = eased(0f)
        eased.toward(1f, light, 180, LinearEasing)
        settle()
        frame(now + 90 * ms) { eased.toward(1f, light, 180, LinearEasing) }
        close()
        assertEquals(1f, eased.toward(1f, light, 180, LinearEasing), 0f)
    }

    @Test
    fun aTileThatIsNotShownTakesItsStateAtOnceAndFollowsNothing() = room {
        val house = HouseLight(Lights.OtherRow)
        val motion = LoungeMotion.Default
        house.brightness(Lights.OtherRow, light, motion)
        val read = HashSet<Any>()
        fun observed(block: () -> Unit): Int {
            read.clear()
            Snapshot.observe(readObserver = { read += it }, block = block)
            return read.size
        }
        // Hidden, a change of state shows at once, asks for no frames and reads nothing that could draw it again.
        assertEquals(0, observed { assertEquals(0.78f, house.brightness(Lights.Rested, light, motion, shown = false), 0.0001f) })
        settle()
        assertFalse(frames.hasAwaiters)
        // Shown again in another state, it is there at once: nobody saw the one before.
        assertTrue(observed { assertEquals(1f, house.brightness(Lights.Focused, light, motion), 0.0001f) } > 0)
        settle()
        assertFalse(frames.hasAwaiters)
        // From then on it eases.
        assertEquals(1f, house.brightness(Lights.Rested, light, motion), 0.0001f)
        settle()
        assertTrue(frames.hasAwaiters)
    }

    @Test
    fun theIdleDimComesOffATilesBrightnessAndSaturation() {
        val light = LightState()
        val motion = LoungeMotion.Default
        val house = HouseLight(Lights.Rested)
        assertEquals(0.78f, house.brightness(Lights.Rested, light, motion), 0.0001f)
        assertEquals(0.92f, house.saturation(Lights.Rested, light, motion), 0.0001f)
        light.idleLevel = 0.75f
        assertEquals(0.72f, house.brightness(Lights.Rested, light, motion), 0.0001f)
        assertEquals(0.895f, house.saturation(Lights.Rested, light, motion), 0.0001f)
        light.idleLevel = 0.5f
        assertEquals(0.66f, house.brightness(Lights.Rested, light, motion), 0.0001f)
        assertEquals(0.87f, house.saturation(Lights.Rested, light, motion), 0.0001f)
    }
}
