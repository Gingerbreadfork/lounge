package tv.lounge.feature.home

import android.content.Context
import android.content.Intent
import android.content.res.Resources
import android.media.AudioAttributes
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.media.MediaRouter
import android.os.Build
import androidx.compose.runtime.Composable
import androidx.compose.runtime.State
import androidx.compose.runtime.produceState
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.conflate
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.withContext

/** What kind of thing the TV's sound comes out of. */
enum class OutputKind { Speakers, Hdmi, HdmiArc, Optical, LineOut, Headphones, Usb, Bluetooth }

/** Where the TV's sound comes out. [name] is what a Bluetooth device calls itself, when it says. */
data class SoundOutput(val kind: OutputKind, val name: String? = null) {
    /** Whether a chip in the top bar says so: sound somewhere other than the room is worth knowing. */
    val wireless: Boolean get() = kind == OutputKind.Bluetooth
}

/** The kind of output an Android audio device is, or null for one that is no way out for sound. */
internal fun outputKind(type: Int): OutputKind? = when (type) {
    AudioDeviceInfo.TYPE_BUILTIN_SPEAKER, AudioDeviceInfo.TYPE_BUILTIN_SPEAKER_SAFE -> OutputKind.Speakers
    AudioDeviceInfo.TYPE_HDMI -> OutputKind.Hdmi
    AudioDeviceInfo.TYPE_HDMI_ARC, AudioDeviceInfo.TYPE_HDMI_EARC -> OutputKind.HdmiArc
    AudioDeviceInfo.TYPE_LINE_DIGITAL -> OutputKind.Optical
    AudioDeviceInfo.TYPE_LINE_ANALOG, AudioDeviceInfo.TYPE_AUX_LINE -> OutputKind.LineOut
    AudioDeviceInfo.TYPE_WIRED_HEADPHONES, AudioDeviceInfo.TYPE_WIRED_HEADSET -> OutputKind.Headphones
    AudioDeviceInfo.TYPE_USB_HEADSET, AudioDeviceInfo.TYPE_USB_DEVICE, AudioDeviceInfo.TYPE_USB_ACCESSORY -> OutputKind.Usb
    AudioDeviceInfo.TYPE_BLUETOOTH_A2DP,
    AudioDeviceInfo.TYPE_HEARING_AID,
    AudioDeviceInfo.TYPE_BLE_HEADSET,
    AudioDeviceInfo.TYPE_BLE_SPEAKER,
    AudioDeviceInfo.TYPE_BLE_BROADCAST,
    -> OutputKind.Bluetooth
    else -> null
}

private fun known(devices: List<Pair<Int, String?>>): List<Pair<OutputKind, String?>> =
    devices.mapNotNull { (type, name) -> outputKind(type)?.let { it to name } }

/**
 * The output to name. [routed] is where Android says media goes, each a type with what it calls
 * itself, and is believed as it is. Without that, [connected] is every output attached: of
 * those only something a person paired or plugged in is named ahead of the speakers or HDMI,
 * since a TV lists its ARC and optical sockets whether or not anything plays through them.
 */
internal fun chosenOutput(routed: List<Pair<Int, String?>>, connected: List<Pair<Int, String?>>): SoundOutput? {
    val (kind, name) = known(routed).firstOrNull()
        ?: known(connected).let { attached ->
            listOf(OutputKind.Bluetooth, OutputKind.Headphones, OutputKind.Usb, OutputKind.Speakers, OutputKind.Hdmi)
                .firstNotNullOfOrNull { want -> attached.firstOrNull { it.first == want } }
        }
        ?: return null
    // Anything but a Bluetooth device gives the TV's own model as its name, which says nothing.
    return SoundOutput(kind, name?.trim()?.takeIf { kind == OutputKind.Bluetooth && it.isNotEmpty() && it != Build.MODEL })
}

/** Where the TV's sound comes out now, or null when Android does not say. */
internal fun soundOutput(context: Context): SoundOutput? = runCatching {
    val audio = context.getSystemService(AudioManager::class.java) ?: return@runCatching null
    val media = AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA).setContentType(AudioAttributes.CONTENT_TYPE_MUSIC).build()
    fun named(devices: List<AudioDeviceInfo>) = devices.map { it.type to it.productName?.toString() }
    // Only Android 13 and later says where media is routed.
    val routed = if (Build.VERSION.SDK_INT >= 33) audio.getAudioDevicesForAttributes(media) else emptyList()
    chosenOutput(named(routed), named(audio.getDevices(AudioManager.GET_DEVICES_OUTPUTS).toList()))
}.getOrNull()

/** What an output reads as: a Bluetooth device's own name, or the kind of output it is. */
internal fun SoundOutput.label(resources: Resources): String = name ?: resources.getString(
    when (kind) {
        OutputKind.Speakers -> R.string.sound_output_speakers
        OutputKind.Hdmi -> R.string.sound_output_hdmi
        OutputKind.HdmiArc -> R.string.sound_output_hdmi_arc
        OutputKind.Optical -> R.string.sound_output_optical
        OutputKind.LineOut -> R.string.sound_output_line
        OutputKind.Headphones -> R.string.sound_output_headphones
        OutputKind.Usb -> R.string.sound_output_usb
        OutputKind.Bluetooth -> R.string.sound_output_bluetooth
    },
)

/**
 * [soundOutput] now, and again as outputs come and go or the sound is sent another way. The
 * reading is a call into another process, so it is kept off the main thread.
 */
private fun Context.soundOutputs(): Flow<SoundOutput?> = callbackFlow {
    val audio = getSystemService(AudioManager::class.java)
    val devices = object : AudioDeviceCallback() {
        override fun onAudioDevicesAdded(addedDevices: Array<out AudioDeviceInfo>?) { trySend(Unit) }
        override fun onAudioDevicesRemoved(removedDevices: Array<out AudioDeviceInfo>?) { trySend(Unit) }
    }
    // A change of route that adds or removes nothing, such as a soundbar taking over from the TV's speakers.
    val router = runCatching { getSystemService(MediaRouter::class.java) }.getOrNull()
    val routes = object : MediaRouter.SimpleCallback() {
        override fun onRouteSelected(router: MediaRouter?, type: Int, info: MediaRouter.RouteInfo?) { trySend(Unit) }
        override fun onRouteChanged(router: MediaRouter?, info: MediaRouter.RouteInfo?) { trySend(Unit) }
    }
    trySend(Unit)
    val watchingDevices = audio != null && runCatching { audio.registerAudioDeviceCallback(devices, null) }.isSuccess
    val watchingRoutes = router != null && runCatching { router.addCallback(MediaRouter.ROUTE_TYPE_LIVE_AUDIO, routes) }.isSuccess
    awaitClose {
        if (watchingDevices) runCatching { audio?.unregisterAudioDeviceCallback(devices) }
        if (watchingRoutes) runCatching { router?.removeCallback(routes) }
    }
}.conflate().map { withContext(Dispatchers.IO) { soundOutput(this@soundOutputs) } }.distinctUntilChanged()

/** Where the sound comes out, followed while Lounge is in front. Null until it has been read. */
@Composable
internal fun rememberSoundOutput(): State<SoundOutput?> {
    val context = LocalContext.current.applicationContext
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    return produceState<SoundOutput?>(null, context, lifecycle) {
        lifecycle.repeatOnLifecycle(Lifecycle.State.RESUMED) {
            context.soundOutputs().collect { value = it }
        }
    }
}

/** The TV's sound screen, or null on a TV with none Lounge may open. */
internal fun soundSettingsIntent(context: Context): Intent? = tvScreen(context, android.provider.Settings.ACTION_SOUND_SETTINGS)
