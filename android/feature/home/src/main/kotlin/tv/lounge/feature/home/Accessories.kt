package tv.lounge.feature.home

import android.annotation.SuppressLint
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothManager
import android.content.Context
import android.content.pm.PackageManager

/** How much charge a paired remote, controller or pair of headphones says it has left. Two of one make differ by [address]. */
data class AccessoryBattery(val name: String, val percent: Int, val address: String = name) {
    val low: Boolean get() = percent <= LOW_BATTERY
}

/** At or under this many percent, a battery is worth a word. */
internal const val LOW_BATTERY = 15

/**
 * The charge of each paired Bluetooth device that reports one, lowest first. A device says
 * nothing while it is not connected. Empty until Lounge is allowed Nearby devices, on Android
 * before 12, and on a TV that keeps the reading to itself.
 */
@SuppressLint("MissingPermission")
internal fun accessoryBatteries(context: Context): List<AccessoryBattery> = runCatching {
    if (context.checkSelfPermission(NEARBY_DEVICES) != PackageManager.PERMISSION_GRANTED) return@runCatching emptyList()
    val adapter = context.getSystemService(BluetoothManager::class.java)?.adapter ?: return@runCatching emptyList()
    // Android has no public call for this. The one its own settings use answers -1 for no reading.
    val level = BluetoothDevice::class.java.getMethod("getBatteryLevel")
    adapter.bondedDevices.orEmpty()
        .mapNotNull { device ->
            val percent = runCatching { level.invoke(device) as? Int }.getOrNull() ?: return@mapNotNull null
            val name = (runCatching { device.alias }.getOrNull() ?: device.name)?.trim().orEmpty()
            if (percent in 0..100 && name.isNotEmpty()) AccessoryBattery(name, percent, device.address ?: name) else null
        }
        .sortedBy { it.percent }
}.onFailure { android.util.Log.w("Lounge", "accessories: no battery readings: $it") }.getOrDefault(emptyList())
