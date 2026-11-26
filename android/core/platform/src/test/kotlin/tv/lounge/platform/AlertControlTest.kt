package tv.lounge.platform

import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import tv.lounge.core.PlatformException

/** A pop-up the core passes on reaches the shell as it was sent, and the shell's refusal goes back. */
@RunWith(RobolectricTestRunner::class)
class AlertControlTest {
    private val platform = AndroidPlatform(RuntimeEnvironment.getApplication())
    private val shown = mutableListOf<HomeAlert>()

    @Test
    fun aPopUpArrivesWithItsWordsPictureAndTime() {
        platform.showAlert = { shown += it }
        platform.control("""alert:{"title":"Someone is at the door","message":"Front door","image":"http://ha.local:8123/c.jpg","seconds":30}""")
        platform.control("""alert:{"title":"Post","seconds":20}""")
        assertEquals(
            listOf(
                HomeAlert("Someone is at the door", "Front door", "http://ha.local:8123/c.jpg", 30),
                HomeAlert("Post", null, null, 20),
            ),
            shown,
        )
    }

    @Test
    fun theShellsRefusalIsWhatTheCallerHears() {
        platform.showAlert = { throw PlatformException.Failed("Not over another app yet.") }
        val refused = assertThrows(PlatformException.Failed::class.java) { platform.control("""alert:{"title":"Post","seconds":20}""") }
        assertEquals("Not over another app yet.", refused.detail)
    }

    @Test
    fun whatIsNotAPopUpIsRefused() {
        platform.showAlert = { shown += it }
        assertThrows(PlatformException.Failed::class.java) { platform.control("alert:not json") }
        assertEquals(emptyList<HomeAlert>(), shown)
    }
}
