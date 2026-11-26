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
        // The count is not a setting, so a backup carries the switch and not the count.
        assertEquals(listOf("hold_hints"), AndroidPlatform(RuntimeEnvironment.getApplication()).preferences().keys.filter { "hint" in it })
    }

    @Test
    fun the_hint_is_turned_off_in_settings_and_owed_in_full_when_turned_back_on() = runBlocking {
        val prefs = Prefs(RuntimeEnvironment.getApplication())
        prefs.useHoldHint(learned = true)
        prefs.setHoldHints(false)
        assertEquals(false, prefs.all.first().holdHints)
        assertEquals("false", prefs.exported()["hold_hints"])
        prefs.setHoldHints(true)
        assertEquals(true to Prefs.HOLD_HINTS, prefs.all.first().let { it.holdHints to it.holdHintsLeft })
        prefs.restore(mapOf("hold_hints" to "false"))
        assertEquals(false, prefs.all.first().holdHints)
        // The store is shared by every test here, so it is left as a new TV has it.
        prefs.setHoldHints(true)
    }
}
