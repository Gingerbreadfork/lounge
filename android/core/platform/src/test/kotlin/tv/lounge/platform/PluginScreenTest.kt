package tv.lounge.platform

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** The controls that open a plugin's own screens, as the core writes them. */
class PluginScreenTest {
    @Test
    fun aSheetIsNamedByItsPluginAndAnIdThatMayHoldColons() {
        assertEquals(UiCommand.OpenPluginSheet("tv.lounge.homeassistant", "room:living_room"), pluginScreen("sheet:tv.lounge.homeassistant:room:living_room"))
        assertNull(pluginScreen("sheet:tv.lounge.homeassistant"))
        assertNull(pluginScreen("sheet::room"))
    }

    @Test
    fun aLivePictureCarriesItsTitleAndAddressAndNothingToRun() {
        assertEquals(
            UiCommand.OpenLive("tv.lounge.homeassistant", "Front door", "http://ha.local:8123/api/camera_proxy/camera.door?token=abc"),
            pluginScreen("live:tv.lounge.homeassistant\nFront door\nhttp://ha.local:8123/api/camera_proxy/camera.door?token=abc"),
        )
        // A line after the address is not an action to hand to the plugin.
        assertEquals(UiCommand.OpenLive("p", "", "http://cam/1.jpg"), pluginScreen("live:p\n\nhttp://cam/1.jpg\nunlock:lock.front"))
        assertNull(pluginScreen("live:p\nTitle"))
        assertNull(pluginScreen("search:lamp"))
    }
}
