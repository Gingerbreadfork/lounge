package tv.lounge.power

import android.app.AlarmManager
import android.content.Context
import android.content.Intent
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf

/** The sleep timer keeps its time, turns everything off once when it runs out, and can be taken back. */
@RunWith(RobolectricTestRunner::class)
class SleepTimerTest {
    private val context: Context = RuntimeEnvironment.getApplication()
    private var now = 1_000_000L
    private var turnedOff = 0
    private var screenOn = true
    private fun timer() = SleepTimer(context, clock = { now }, screenOn = { screenOn }) { turnedOff++ }
    private fun alarms() = shadowOf(context.getSystemService(AlarmManager::class.java)).scheduledAlarms.size

    @Test
    fun aTimerOutlivesTheProcessAndTurnsEverythingOffOnce() = runBlocking {
        timer().set(30)
        assertEquals(1, alarms())
        // A new process picks the time up again.
        val later = timer()
        later.restore()
        assertEquals(now + 30 * 60_000L, later.endsAt.value)
        later.fireIfDue()
        assertEquals(0, turnedOff)
        now += 30 * 60_000L
        later.fireIfDue()
        later.fireIfDue()
        assertEquals(1, turnedOff)
        assertNull(later.endsAt.value)
        assertEquals(0, alarms())
    }

    @Test
    fun aCancelledTimerDoesNothing() = runBlocking {
        val timer = timer()
        timer.set(15)
        timer.cancel()
        assertNull(timer.endsAt.value)
        assertEquals(0, alarms())
        now += 60 * 60_000L
        timer.fireIfDue()
        assertEquals(0, turnedOff)
    }

    @Test
    fun aTimerThatRanOutWhileTheTvWasOffTurnsNothingOff() = runBlocking {
        timer().set(30)
        now += 3 * 60 * 60_000L
        val later = timer()
        later.restore()
        assertNull(later.endsAt.value)
        assertEquals(0, alarms())
        later.fireIfDue()
        assertEquals(0, turnedOff)
        // A late alarm for a time long past is dropped the same way.
        val alarmed = timer()
        alarmed.set(15)
        now += 60 * 60_000L
        alarmed.fireIfDue()
        assertEquals(0, turnedOff)
        assertNull(alarmed.endsAt.value)
    }

    @Test
    fun turningTheScreenOffEndsTheTimer() = runBlocking {
        val timer = timer()
        timer.set(30)
        context.sendBroadcast(Intent(Intent.ACTION_SCREEN_OFF))
        shadowOf(android.os.Looper.getMainLooper()).idle()
        assertNull(timer.endsAt.value)
        assertEquals(0, alarms())
        // One that runs out with the screen already off has nothing to turn off.
        timer.set(15)
        screenOn = false
        now += 15 * 60_000L
        timer.fireIfDue()
        assertEquals(0, turnedOff)
    }

    @Test
    fun pressesStepThroughTheTimesAndOff() {
        assertEquals(15, sleepStep(null, forward = true))
        assertEquals(30, sleepStep(15, forward = true))
        assertEquals(30, sleepStep(28, forward = true))
        assertEquals(0, sleepStep(120, forward = true))
        assertEquals(120, sleepStep(null, forward = false))
        assertEquals(15, sleepStep(28, forward = false))
        assertEquals(0, sleepStep(12, forward = false))
    }
}
