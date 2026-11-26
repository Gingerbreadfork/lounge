package tv.lounge.feature.home

import android.bluetooth.BluetoothManager
import android.content.Context
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.shadows.ShadowBluetoothDevice

/** The battery of paired remotes, controllers and headphones. */
@RunWith(RobolectricTestRunner::class)
class AccessoriesTest {
    private val context: Context = RuntimeEnvironment.getApplication()
    private val adapter = context.getSystemService(BluetoothManager::class.java).adapter

    private fun paired(address: String, name: String, percent: Int) = ShadowBluetoothDevice.newInstance(address).also {
        shadowOf(it).setName(name)
        shadowOf(it).setBatteryLevel(percent)
    }

    @Test
    fun nothingIsReadUntilNearbyDevicesIsAllowed() {
        shadowOf(adapter).setBondedDevices(setOf(paired("00:11:22:33:44:55", "Remote", 80)))
        assertEquals(emptyList<AccessoryBattery>(), accessoryBatteries(context))
    }

    @Test
    fun devicesThatReportAChargeAreListedLowestFirst() {
        shadowOf(RuntimeEnvironment.getApplication()).grantPermissions(NEARBY_DEVICES)
        shadowOf(adapter).setBondedDevices(
            setOf(paired("00:11:22:33:44:55", "Remote", 80), paired("00:11:22:33:44:56", " Controller ", 12), paired("00:11:22:33:44:57", "Not connected", -1)),
        )
        val read = accessoryBatteries(context)
        assertEquals(listOf(AccessoryBattery("Controller", 12, "00:11:22:33:44:56"), AccessoryBattery("Remote", 80, "00:11:22:33:44:55")), read)
        assertEquals(listOf(true, false), read.map { it.low })
    }
}
