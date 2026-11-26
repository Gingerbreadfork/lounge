package tv.lounge.feature.home

import android.media.AudioDeviceInfo
import android.os.Build
import androidx.test.core.app.ApplicationProvider
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

/** Which of the TV's sound outputs is named, and what it reads as. */
@RunWith(RobolectricTestRunner::class)
class SoundTest {
    private val resources = ApplicationProvider.getApplicationContext<android.content.Context>().resources

    private val speakers = AudioDeviceInfo.TYPE_BUILTIN_SPEAKER to Build.MODEL
    private val hdmi = AudioDeviceInfo.TYPE_HDMI to "LG TV"
    private val arc = AudioDeviceInfo.TYPE_HDMI_ARC to "Soundbar"
    private val optical = AudioDeviceInfo.TYPE_LINE_DIGITAL to "spdif"
    private val buds = AudioDeviceInfo.TYPE_BLUETOOTH_A2DP to " WH-1000XM4 "
    private val none = emptyList<Pair<Int, String?>>()

    @Test
    fun whereAndroidSaysMediaGoesIsBelieved() {
        assertEquals(SoundOutput(OutputKind.HdmiArc), chosenOutput(listOf(arc), listOf(speakers, arc, optical)))
        assertEquals(SoundOutput(OutputKind.Speakers), chosenOutput(listOf(speakers), listOf(speakers, arc, buds)))
        assertEquals(SoundOutput(OutputKind.Bluetooth, "WH-1000XM4"), chosenOutput(listOf(buds), listOf(speakers, buds)))
    }

    @Test
    fun withoutThatASocketWithNothingPlayingThroughItIsNotNamed() {
        // A TV lists its ARC and optical sockets whether or not anything is plugged in.
        assertEquals(SoundOutput(OutputKind.Speakers), chosenOutput(none, listOf(arc, optical, speakers)))
        assertEquals(SoundOutput(OutputKind.Hdmi), chosenOutput(none, listOf(hdmi)))
        assertEquals(SoundOutput(OutputKind.Bluetooth, "WH-1000XM4"), chosenOutput(none, listOf(speakers, arc, buds)))
        assertNull(chosenOutput(none, listOf(arc, optical)))
        // A route Lounge has no word for falls back the same way.
        assertEquals(SoundOutput(OutputKind.Speakers), chosenOutput(listOf(AudioDeviceInfo.TYPE_BUS to "bus"), listOf(speakers, arc)))
    }

    @Test
    fun somethingThatIsNoWayOutForSoundIsNotNamed() {
        assertNull(chosenOutput(none, none))
        assertNull(chosenOutput(none, listOf(AudioDeviceInfo.TYPE_REMOTE_SUBMIX to "x", AudioDeviceInfo.TYPE_IP to "y", AudioDeviceInfo.TYPE_BUILTIN_MIC to "z")))
    }

    @Test
    fun onlyABluetoothDeviceIsSaidInTheTopBarAndByItsOwnName() {
        val named = chosenOutput(listOf(AudioDeviceInfo.TYPE_BLE_HEADSET to "Kitchen buds"), none)!!
        assertEquals(true to "Kitchen buds", named.wireless to named.label(resources))
        // One that gives no name, or the TV's own, reads as what it is.
        assertEquals("Bluetooth", chosenOutput(listOf(AudioDeviceInfo.TYPE_BLUETOOTH_A2DP to Build.MODEL), none)!!.label(resources))
        assertEquals("Bluetooth", chosenOutput(none, listOf(AudioDeviceInfo.TYPE_BLUETOOTH_A2DP to null))!!.label(resources))
        val wired = chosenOutput(none, listOf(AudioDeviceInfo.TYPE_WIRED_HEADPHONES to "h2w", speakers))!!
        assertEquals(false to "Headphones", wired.wireless to wired.label(resources))
        assertEquals("TV speakers", SoundOutput(OutputKind.Speakers).label(resources))
        assertEquals("HDMI ARC", chosenOutput(listOf(AudioDeviceInfo.TYPE_HDMI_EARC to "x"), none)!!.label(resources))
    }
}
