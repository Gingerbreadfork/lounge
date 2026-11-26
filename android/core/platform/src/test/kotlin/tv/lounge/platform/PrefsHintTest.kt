package tv.lounge.platform

import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

/** How many times the home still owes the hint about holding OK. */
@RunWith(RobolectricTestRunner::class)
class PrefsHintTest {
    @Test
    fun the_hint_is_owed_a_few_times_and_never_once_learned() = runBlocking {
        val prefs = Prefs(RuntimeEnvironment.getApplication())
        assertEquals(Prefs.HOLD_HINTS, prefs.all.first().holdHintsLeft)
        prefs.useHoldHint()
        assertEquals(Prefs.HOLD_HINTS - 1, prefs.all.first().holdHintsLeft)
        prefs.useHoldHint(learned = true)
        assertEquals(0, prefs.all.first().holdHintsLeft)
        prefs.useHoldHint()
        assertEquals(0, prefs.all.first().holdHintsLeft)
        // It is not a setting, so a backup does not carry it.
        assertEquals(false, AndroidPlatform(RuntimeEnvironment.getApplication()).preferences().keys.any { "hint" in it })
    }
}
