package tv.lounge.power

import android.content.ComponentName
import android.content.Context
import android.provider.Settings
import java.io.File
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/** A shell action the helper is allowed to run, shown to the person before it runs. */
data class PowerAction(val id: String, val title: String, val detail: String, val command: String, val timeoutMs: Int = Privileged.DEFAULT_TIMEOUT_MS)

/** A value dropped into a shell command, quoted so it is one word whatever it contains. */
internal fun shellQuote(value: String): String = "'" + value.replace("'", "'\\''") + "'"

/** One thing the helper did, for the sheet and diagnostics. */
data class PowerLogEntry(val atMs: Long, val action: String, val command: String, val result: ShellResult?, val error: String?)

/** A CEC device the TV knows about, from `dumpsys hdmi_control`. */
data class CecDevice(val logicalAddress: Int, val type: Int, val name: String, val powerStatus: Int, val physicalAddress: Int) {
    val isTv: Boolean get() = logicalAddress == 0
    val isOn: Boolean get() = powerStatus == 0
}

/**
 * Power Setup: shell access through the device's own ADB daemon or Shizuku, for the few things an
 * app cannot do itself: the home role, its own optional accesses, HDMI-CEC. Every command is
 * whitelisted and logged.
 */
class PowerSetup(private val context: Context) {
    private val keys = AdbKeys(File(context.filesDir, "power"))
    private val adb = AdbBackend(AdbClient(keys))
    private val shizuku = ShizukuBackend()
    private val lock = Mutex()
    private val giveUpAt = AtomicLong(Long.MAX_VALUE)
    private val _status = MutableStateFlow<PowerStatus>(PowerStatus.Unavailable("Not checked yet"))
    private val _log = MutableStateFlow<List<PowerLogEntry>>(emptyList())

    val status: StateFlow<PowerStatus> = _status.asStateFlow()
    val log: StateFlow<List<PowerLogEntry>> = _log.asStateFlow()
    val packageName: String get() = context.packageName
    val fingerprint: String get() = keys.fingerprint()
    val hasKey: Boolean get() = keys.exists
    val shizukuInstalled: Boolean get() = shizuku.installed

    val adbEnabled: Boolean
        get() = runCatching { Settings.Global.getInt(context.contentResolver, Settings.Global.ADB_ENABLED, 0) == 1 }.getOrDefault(false)

    /** Finds out which backend works, without prompting anyone. */
    suspend fun refresh(): PowerStatus = lock.withLock {
        val s = when {
            shizuku.permitted -> PowerStatus.Ready("shizuku")
            adb.available() -> PowerStatus.Ready("adb")
            !adbEnabled -> PowerStatus.Unavailable(if (shizuku.installed) "Shizuku is installed but has not granted access" else "Developer options → USB debugging is off")
            else -> PowerStatus.NotPaired
        }
        _status.value = s
        s
    }

    /**
     * Connects to the daemon and, if our key is new to it, waits while the person answers the
     * on-screen prompt. Returns the resulting status.
     */
    suspend fun pair(): PowerStatus = lock.withLock {
        withContext(Dispatchers.IO) {
            giveUpAt.set(Long.MAX_VALUE)
            val outcome = AdbClient(keys).connect(onWaitingForUser = { _status.value = PowerStatus.WaitingForApproval }, giveUpAt = giveUpAt)
            val s = when (outcome) {
                is AdbClient.Outcome.Online -> { outcome.connection.close(); PowerStatus.Ready("adb") }
                AdbClient.Outcome.Refused -> PowerStatus.Unavailable("ADB is not listening. Turn on Developer options → USB debugging, then try again.")
                AdbClient.Outcome.Unknown, AdbClient.Outcome.Denied -> PowerStatus.NotPaired
                is AdbClient.Outcome.TlsRequired -> PowerStatus.Failed(outcome.detail)
                is AdbClient.Outcome.Error -> PowerStatus.Failed(outcome.detail)
            }
            _status.value = s
            s
        }
    }

    fun requestShizuku(requestCode: Int) = shizuku.requestPermission(requestCode)

    /** The launcher's window has focus again: if the prompt was up, it has been answered. */
    fun noteWindowFocused() {
        if (_status.value == PowerStatus.WaitingForApproval) {
            giveUpAt.set(System.currentTimeMillis() + 3_000)
        }
    }

    /** Forgets the key so the device prompts again next time. */
    fun forget() {
        keys.forget()
        _status.value = PowerStatus.NotPaired
    }

    private fun backend(): Privileged? = when (val s = _status.value) {
        is PowerStatus.Ready -> if (s.backend == "shizuku") shizuku else adb
        else -> null
    }

    /** Runs one whitelisted action; the result goes to the log either way. */
    suspend fun run(action: PowerAction): ShellResult {
        val backend = backend() ?: throw PrivilegedException("Power Setup is not connected")
        return try {
            val r = backend.run(action.command, action.timeoutMs)
            record(PowerLogEntry(System.currentTimeMillis(), action.title, action.command, r, null))
            r
        } catch (e: kotlinx.coroutines.CancellationException) {
            throw e
        } catch (e: Exception) {
            val failure = e as? PrivilegedException ?: PrivilegedException(e.message ?: "That did not work", e)
            record(PowerLogEntry(System.currentTimeMillis(), action.title, action.command, null, failure.message))
            if (backend === adb && e is PrivilegedException) _status.value = PowerStatus.NotPaired
            throw failure
        }
    }

    private fun record(entry: PowerLogEntry) {
        _log.value = (_log.value + entry).takeLast(60)
    }

    // ----- the whitelist -----

    private val home = ComponentName(context.packageName, "tv.lounge.launcher.MainActivity").flattenToShortString()

    val homeRole = PowerAction("home", "Make Lounge the home screen", "Pressing Home brings Lounge up; the stock launcher stays installed.", "cmd package set-home-activity $home")
    val tvListings = PowerAction("tv_listings", "Allow TV listings", "Lets Lounge read Watch Next, channels and the guide.", "pm grant ${context.packageName} android.permission.READ_TV_LISTINGS")
    val notificationAccess = PowerAction("notifications", "Allow Now playing", "Lets Lounge see what is playing and control it.", "cmd notification allow_listener ${context.packageName}/tv.lounge.collector.media.NowPlayingListener")
    val usageAccess = PowerAction("usage", "Allow usage access", "Sorts Your apps by what you actually open.", "appops set ${context.packageName} GET_USAGE_STATS allow")
    val quickAnimations = PowerAction("animations", "Faster system animations", "Sets the three animation scales to 0.5×.", "settings put global window_animation_scale 0.5; settings put global transition_animation_scale 0.5; settings put global animator_duration_scale 0.5")
    val normalAnimations = PowerAction("animations_reset", "Normal system animations", "Sets the three animation scales back to 1×.", "settings put global window_animation_scale 1.0; settings put global transition_animation_scale 1.0; settings put global animator_duration_scale 1.0")
    val optimise = PowerAction("optimise", "Optimise Lounge now", "Compiles the launcher ahead of time, as the system would overnight.", "cmd package compile -r bg-dexopt ${context.packageName}", timeoutMs = 5 * 60_000)

    val oneTouchPlay = PowerAction("cec_otp", "Wake the TV and switch to me", "HDMI-CEC One Touch Play.", "cmd hdmi_control onetouchplay")
    val tvPower = PowerAction("cec_tv_power", "TV power", "Toggles the TV on or off over HDMI-CEC.", "input keyevent KEYCODE_TV_POWER")
    val everythingOff = PowerAction("sleep", "Everything off", "Puts this device to sleep; the TV follows over HDMI-CEC.", "input keyevent KEYCODE_SLEEP")
    val systemAudioOn = PowerAction("cec_sam_on", "Sound through the soundbar", "HDMI-CEC System Audio Mode on.", "cmd hdmi_control setsystemaudiomode on")
    val systemAudioOff = PowerAction("cec_sam_off", "Sound through the TV", "HDMI-CEC System Audio Mode off.", "cmd hdmi_control setsystemaudiomode off")
    fun selectDevice(d: CecDevice) = PowerAction("cec_select_${d.logicalAddress}", "Switch to ${d.name}", "HDMI-CEC device select.", "cmd hdmi_control deviceselect ${d.logicalAddress}")
    fun mediaKey(key: String) = PowerAction("media_$key", "Media: $key", "Dispatches a media key to the active session.", "cmd media_session dispatch $key")
    fun volume(direction: String) = PowerAction("volume_$direction", "Volume $direction", "Adjusts the media stream.", "cmd media_session volume --adj $direction --show")

    val setupActions: List<PowerAction> get() = listOf(homeRole, tvListings, notificationAccess, usageAccess, quickAnimations, optimise)

    private val ambient = ComponentName(context.packageName, "tv.lounge.launcher.AmbientDreamService").flattenToShortString()

    val useAmbient = PowerAction("ambient", "Use Lounge as the screensaver", "Points the system screensaver at Lounge's Ambient mode.", "settings put secure screensaver_components $ambient; settings put secure screensaver_enabled 1")
    fun restoreScreensaver(component: String?) = PowerAction(
        "ambient_restore",
        "Back to the previous screensaver",
        "Puts the system screensaver back the way it was.",
        if (component.isNullOrBlank()) "settings delete secure screensaver_components" else "settings put secure screensaver_components ${shellQuote(component)}",
    )

    /** The system screensaver component as flattened by Settings, or null when none is set. */
    val currentScreensaver: String?
        get() = runCatching { Settings.Secure.getString(context.contentResolver, "screensaver_components") }.getOrNull()?.takeIf { it.isNotBlank() }

    /** A readable name for a flattened component: its app's label, else its package. */
    fun labelFor(component: String): String {
        val cn = ComponentName.unflattenFromString(component.substringBefore(',')) ?: return component
        return runCatching {
            context.packageManager.getApplicationLabel(context.packageManager.getApplicationInfo(cn.packageName, 0)).toString()
        }.getOrDefault(cn.packageName)
    }

    /** CEC devices on the bus right now, the TV first. */
    suspend fun cecDevices(): List<CecDevice> {
        val backend = backend() ?: return emptyList()
        val out = runCatching { backend.run("dumpsys hdmi_control").output }.getOrDefault("")
        return parseCecDevices(out)
    }

    /** A television has pass-through inputs of its own (HDMI, tuner); a box plugged into one has none. */
    val isTelevision: Boolean by lazy {
        val tim = context.getSystemService(android.media.tv.TvInputManager::class.java)
        runCatching { tim?.tvInputList.orEmpty().any { it.isPassthroughInput } }.getOrDefault(false)
    }

    val hasCec: Boolean get() = context.packageManager.hasSystemFeature("android.hardware.hdmi.cec")

    /** A box plugged into a TV gets the TV controls card; a TV gets its Inputs row instead. */
    val hasTvControls: Boolean get() = hasCec && !isTelevision

    companion object {
        private val cecLine = Regex("""logical_address:\s*0x([0-9a-fA-F]+)\s+device_type:\s*(\d+).*?display_name:\s*(.*?)\s+power_status:\s*(-?\d+)\s+physical_address:\s*0x([0-9a-fA-F]+)""")

        fun parseCecDevices(dump: String): List<CecDevice> {
            val seen = LinkedHashMap<Int, CecDevice>()
            for (line in dump.lineSequence()) {
                val m = cecLine.find(line) ?: continue
                val d = CecDevice(
                    logicalAddress = m.groupValues[1].toInt(16),
                    type = m.groupValues[2].toInt(),
                    name = m.groupValues[3].trim().ifEmpty { "Device ${m.groupValues[1]}" },
                    powerStatus = m.groupValues[4].toInt(),
                    physicalAddress = m.groupValues[5].toInt(16),
                )
                seen.putIfAbsent(d.logicalAddress, d)
            }
            return seen.values.sortedBy { it.logicalAddress }
        }
    }
}
