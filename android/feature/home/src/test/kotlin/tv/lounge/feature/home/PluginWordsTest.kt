package tv.lounge.feature.home

import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

@RunWith(RobolectricTestRunner::class)
class PluginWordsTest {
    private val resources get() = RuntimeEnvironment.getApplication().resources

    @Test
    fun longHelpKeepsItsFirstSentenceAndPointsToThePhone() {
        assertEquals(
            "Your server's address, such as http://192.168.0.20:8096. Your phone shows the full steps.",
            shortHelp(resources, "Your server's address, such as http://192.168.0.20:8096. Find it in the dashboard under Networking, then copy the local address shown there, port and all, and paste it here from the phone."),
        )
        // Help short enough to read whole stays whole.
        assertEquals(
            "Add the region after a comma when the name is shared, as in Portland, Maine. Coordinates work too, as in 45.76, 4.84.",
            shortHelp(resources, "Add the region after a comma when the name is shared, as in Portland, Maine. Coordinates work too, as in 45.76, 4.84."),
        )
        // One sentence stays as it is, and an address's own dots are not the end of one.
        assertEquals("Open plex.tv/link on your phone.", shortHelp(resources, "Open plex.tv/link on your phone."))
        assertEquals("Leave it empty to use the default", shortHelp(resources, "  Leave it empty to use the default "))
    }
}
