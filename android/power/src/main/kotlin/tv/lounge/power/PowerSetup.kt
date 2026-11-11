package tv.lounge.power

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.provider.Settings
import java.io.File
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/** A shell action the helper is allowed to run, shown to the person before it runs. */
data class PowerAction(val id: String, val title: String, val detail: String, val command: String, val timeoutMs: Int = Privileged.DEFAULT_TIMEOUT_MS)

/** A value dropped into a shell command, quoted so it is one word whatever it contains. */
internal fun shellQuote(value: String): String = "'" + value.replace("'", "'\\''") + "'"

/** One thing the helper did, for the sheet and diagnostics. */
data class PowerLogEntry(val atMs: Long, val action: String, val command: String, val result: ShellResult?, val error: String?)

/**
 * A home screen on this device besides Lounge: [enabled] when Android can pick it now, [appEnabled]
 * false when its whole app is switched off.
 */
data class HomeApp(val packageName: String, val label: String, val priority: Int, val enabled: Boolean, val appEnabled: Boolean)

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
    /** Why the last Connect failed, shown until the next try or a connection. */
    @Volatile private var pairFailure: PowerStatus.Failed? = null
    private val _status = MutableStateFlow<PowerStatus>(PowerStatus.Unavailable("Checking USB debugging…"))
    private val _log = MutableStateFlow<List<PowerLogEntry>>(emptyList())
    private val _results = MutableStateFlow<Map<String, PowerResult>>(emptyMap())
    private val _running = MutableStateFlow<Set<String>>(emptySet())
    private val _applied = MutableStateFlow<Map<String, Boolean>>(emptyMap())
    /** Outlives every sheet, so work begun in one is finished and kept after Back. */
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val _accessGranted = MutableSharedFlow<Unit>(extraBufferCapacity = 1, onBufferOverflow = BufferOverflow.DROP_OLDEST)

    val status: StateFlow<PowerStatus> = _status.asStateFlow()
    val log: StateFlow<List<PowerLogEntry>> = _log.asStateFlow()
    /** How each action last went while Lounge has been running, by action id. */
    val results: StateFlow<Map<String, PowerResult>> = _results.asStateFlow()
    /** The actions [start] is running now, by id. */
    val running: StateFlow<Set<String>> = _running.asStateFlow()
    /** Whether what each setup action sets up is so, by action id, as [recheck] last read it. */
    val applied: StateFlow<Map<String, Boolean>> = _applied.asStateFlow()
    /** Emits after one of Lounge's own accesses has been granted, so the app can read them again. */
    val accessGranted: SharedFlow<Unit> = _accessGranted.asSharedFlow()
    val packageName: String get() = context.packageName
    val hasKey: Boolean get() = keys.exists

    /** The key's fingerprint as the device's prompt shows it. The first call makes the key, so it runs off the main thread. */
    suspend fun fingerprint(): String = withContext(Dispatchers.IO) { keys.fingerprint() }
    val shizukuInstalled: Boolean get() = shizuku.installed

    val adbEnabled: Boolean
        get() = runCatching { Settings.Global.getInt(context.contentResolver, Settings.Global.ADB_ENABLED, 0) == 1 }.getOrDefault(false)

    /** Finds out which backend works, without prompting anyone. With debugging off, nothing on the ADB port is adbd. */
    suspend fun refresh(): PowerStatus = lock.withLock {
        val s = when {
            shizuku.permitted -> PowerStatus.Ready("shizuku")
            !adbEnabled -> PowerStatus.Unavailable(if (shizuku.installed) SHIZUKU_WAITING else DEBUGGING_OFF)
            adb.available() -> PowerStatus.Ready("adb")
            else -> PowerStatus.NotPaired
        }
        val shown = keptFailure(s, pairFailure)
        if (shown !is PowerStatus.Failed) pairFailure = null
        _status.value = shown
        shown
    }

    /**
     * Connects to the daemon and, if our key is new to it, waits while the person answers the
     * on-screen prompt. Returns the resulting status.
     */
    suspend fun pair(): PowerStatus = lock.withLock {
        withContext(Dispatchers.IO) {
            if (!adbEnabled) {
                return@withContext PowerStatus.Unavailable(DEBUGGING_OFF).also { _status.value = it }
            }
            giveUpAt.set(Long.MAX_VALUE)
            pairFailure = null
            val outcome = AdbClient(keys).connect(onWaitingForUser = { _status.value = PowerStatus.WaitingForApproval }, giveUpAt = giveUpAt)
            val s = when (outcome) {
                is AdbClient.Outcome.Online -> { outcome.connection.close(); PowerStatus.Ready("adb") }
                AdbClient.Outcome.Refused -> PowerStatus.Failed(AdbBackend.UNREACHABLE)
                AdbClient.Outcome.Unknown, AdbClient.Outcome.Denied -> PowerStatus.NotPaired
                is AdbClient.Outcome.TlsRequired -> PowerStatus.Failed(AdbBackend.TLS)
                is AdbClient.Outcome.NotThisDevice -> PowerStatus.Failed(AdbBackend.NOT_THIS_TV)
                is AdbClient.Outcome.Error -> PowerStatus.Failed(AdbBackend.DROPPED)
            }
            when (outcome) {
                is AdbClient.Outcome.TlsRequired -> outcome.detail
                is AdbClient.Outcome.NotThisDevice -> outcome.detail
                is AdbClient.Outcome.Error -> outcome.detail
                else -> null
            }?.let { android.util.Log.w("Lounge", "power setup: $it") }
            pairFailure = s as? PowerStatus.Failed
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

    /** Forgets the key so the device prompts again next time, and how each action went with it. */
    fun forget() {
        keys.forget()
        pairFailure = null
        _results.value = emptyMap()
        _status.value = PowerStatus.NotPaired
    }

    private fun backend(): Privileged? = when (val s = _status.value) {
        is PowerStatus.Ready -> if (s.backend == "shizuku") shizuku else adb
        else -> null
    }

    /** Runs [actions] in order, stopping at the first that fails; returns that one's result, or the last. */
    suspend fun runAll(actions: List<PowerAction>): ShellResult {
        var last = ShellResult(0, "")
        for (action in actions) {
            last = run(action)
            if (!last.ok) break
        }
        return last
    }

    /**
     * Runs [action] in Power Setup's own scope, so it finishes and keeps its result after the sheet
     * that asked has closed. One still running is not started again.
     */
    fun start(action: PowerAction) {
        var fresh = false
        _running.update { ids -> fresh = action.id !in ids; ids + action.id }
        if (!fresh) return
        scope.launch {
            try {
                runCatching { run(action) }
                recheck()
            } finally {
                _running.update { it - action.id }
            }
        }
    }

    /** Runs [block] to its end in Power Setup's own scope, even when the sheet that asked closes first. */
    suspend fun <T> finish(block: suspend () -> T): T = scope.async { block() }.await()

    /** Reads again what is already so on the TV, for [applied]. */
    suspend fun recheck() = withContext(Dispatchers.IO) {
        _applied.value = (setupActions + normalAnimations).mapNotNull { a -> isApplied(a)?.let { a.id to it } }.toMap()
    }

    /** Runs one whitelisted action. A failure's raw reason goes to the log, and its sentence to [results]. */
    suspend fun run(action: PowerAction): ShellResult {
        // After a restart nothing is known until checked; with a key or Shizuku there, check now.
        val backend = backend() ?: (if (hasKey || shizuku.installed) refresh().let { backend() } else null)
            ?: throw PrivilegedException(NOT_CONNECTED).also { keepResult(action, PowerResult(false, "", it.message)) }
        return try {
            val r = backend.run(action.command, action.timeoutMs)
            record(PowerLogEntry(System.currentTimeMillis(), action.title, action.command, r, null))
            if (!r.ok) android.util.Log.w("Lounge", "power setup: ${action.id} exited ${r.exitCode}: ${r.output.take(LOGGED_OUTPUT)}")
            keepResult(action, PowerResult(r.ok, r.output, if (r.ok) null else failureSentence(r.output)))
            if (r.ok && (action == tvListings || action == notificationAccess || action == usageAccess)) _accessGranted.tryEmit(Unit)
            r
        } catch (e: kotlinx.coroutines.CancellationException) {
            throw e
        } catch (e: Exception) {
            val failure = e as? PrivilegedException ?: PrivilegedException("Power Setup couldn't run that. Try again.", e)
            android.util.Log.w("Lounge", "power setup: ${action.id} did not run", e)
            record(PowerLogEntry(System.currentTimeMillis(), action.title, action.command, null, listOfNotNull(failure.message, failure.cause?.message).joinToString(": ")))
            keepResult(action, PowerResult(false, "", failure.message))
            // A lost connection or a timeout says nothing about the pairing; only a refused key does.
            if (backend === adb && e is KeyNotApprovedException) _status.value = PowerStatus.NotPaired
            throw failure
        }
    }

    private fun record(entry: PowerLogEntry) {
        // Two commands can finish at once; update keeps both entries.
        _log.update { (it + entry).takeLast(60) }
    }

    private fun keepResult(action: PowerAction, result: PowerResult) {
        _results.update { withResult(it, action.id, result) }
    }

    /**
     * Whether what [action] sets up is already so, read from the system; null when that cannot be
     * told. Reads settings and the package list, so call it off the main thread.
     */
    fun isApplied(action: PowerAction): Boolean? = runCatching {
        val resolver = context.contentResolver
        fun scales() = listOf("window_animation_scale", "transition_animation_scale", "animator_duration_scale").map { Settings.Global.getFloat(resolver, it, 1f) }
        when (action.id) {
            homeRole.id -> context.packageManager.resolveActivity(Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_HOME), PackageManager.MATCH_DEFAULT_ONLY)?.activityInfo?.packageName == context.packageName
            tvListings.id -> context.checkSelfPermission("android.permission.READ_TV_LISTINGS") == PackageManager.PERMISSION_GRANTED
            notificationAccess.id -> androidx.core.app.NotificationManagerCompat.getEnabledListenerPackages(context).contains(context.packageName)
            usageAccess.id -> context.getSystemService(android.app.AppOpsManager::class.java)?.let { ops ->
                if (android.os.Build.VERSION.SDK_INT >= 29) {
                    ops.unsafeCheckOpNoThrow(android.app.AppOpsManager.OPSTR_GET_USAGE_STATS, android.os.Process.myUid(), context.packageName)
                } else {
                    @Suppress("DEPRECATION")
                    ops.checkOpNoThrow(android.app.AppOpsManager.OPSTR_GET_USAGE_STATS, android.os.Process.myUid(), context.packageName)
                } == android.app.AppOpsManager.MODE_ALLOWED
            }
            quickAnimations.id -> scales().all { it == 0.5f }
            normalAnimations.id -> scales().all { it == 1f }
            else -> null
        }
    }.getOrNull()

    // ----- the whitelist -----

    private val home = ComponentName(context.packageName, "tv.lounge.launcher.MainActivity").flattenToShortString()

    val homeRole = PowerAction("home", "Make Lounge the home screen", "Pressing Home brings Lounge up; the TV's own home screen stays installed.", homeCommand(home, userId))
    val tvListings = PowerAction("tv_listings", "Allow TV listings", "Lets Lounge see what you were watching, channels and the TV guide.", grantCommand(context.packageName, "android.permission.READ_TV_LISTINGS", userId))
    val notificationAccess = PowerAction("notifications", "Allow Now playing", "Lets Lounge see what is playing and control it.", "cmd notification allow_listener ${context.packageName}/tv.lounge.collector.media.NowPlayingListener")
    val usageAccess = PowerAction("usage", "Allow usage access", "Lets Your apps follow what you open.", "appops set ${context.packageName} GET_USAGE_STATS allow")
    val quickAnimations = PowerAction("animations", "Faster system animations", "Halves the TV's own animations, so its menus and apps open quicker.", "settings put global window_animation_scale 0.5; settings put global transition_animation_scale 0.5; settings put global animator_duration_scale 0.5")
    val normalAnimations = PowerAction("animations_reset", "Normal system animations", "Puts the TV's own animations back to their usual length.", "settings put global window_animation_scale 1.0; settings put global transition_animation_scale 1.0; settings put global animator_duration_scale 1.0")
    val optimise = PowerAction("optimise", "Optimise Lounge now", "Prepares Lounge now so it opens and scrolls faster, as the TV would overnight.", "cmd package compile -r bg-dexopt ${context.packageName}", timeoutMs = 5 * 60_000)

    val oneTouchPlay = PowerAction("cec_otp", "Wake the TV and switch to me", "Turns the TV on and switches it to Lounge.", "cmd hdmi_control onetouchplay")
    val tvPower = PowerAction("cec_tv_power", "TV power", "Turns the TV on or off.", "input keyevent KEYCODE_TV_POWER")
    val everythingOff = PowerAction("sleep", "Everything off", "Goes to sleep, and the TV turns off with it.", "input keyevent KEYCODE_SLEEP")
    val systemAudioOn = PowerAction("cec_sam_on", "Sound through the soundbar", "Plays the sound through the soundbar or receiver.", "cmd hdmi_control setsystemaudiomode on")
    val systemAudioOff = PowerAction("cec_sam_off", "Sound through the TV", "Plays the sound through the TV's own speakers.", "cmd hdmi_control setsystemaudiomode off")
    fun selectDevice(d: CecDevice) = PowerAction("cec_select_${d.logicalAddress}", "Switch to ${d.name}", "Switches the TV to ${d.name}.", "cmd hdmi_control deviceselect ${d.logicalAddress}")

    /** A remote key for whatever is in front, while Lounge is not; only the d-pad, Back, Home and Menu, anything else is refused. */
    fun inputKey(keyCode: Int): PowerAction? {
        val name = REMOTE_KEYS[keyCode] ?: return null
        // From Android 12 the input service answers `cmd` directly, without starting a new process.
        val command = if (android.os.Build.VERSION.SDK_INT >= 31) "cmd input keyevent $name" else "input keyevent $name"
        return PowerAction("key_$name", "Remote: $name", "Presses a remote key for the app in front.", command, timeoutMs = 5_000)
    }

    val setupActions: List<PowerAction> get() = listOf(homeRole, tvListings, notificationAccess, usageAccess, quickAnimations, optimise)

    // ----- home screens that come before Lounge -----

    /** This app's user, which `pm` needs named: Android gives each user its own range of uids. */
    private val userId: Int get() = android.os.Process.myUid() / 100_000

    /** Lounge's own home priority as Android sees it, and every other home screen on the device. */
    private fun homes(): Pair<Int, List<HomeApp>> {
        val pm = context.packageManager
        val intent = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_HOME)
        val all = runCatching { pm.queryIntentActivities(intent, PackageManager.MATCH_DISABLED_COMPONENTS) }.getOrDefault(emptyList())
        val pickable = runCatching { pm.queryIntentActivities(intent, 0) }.getOrDefault(emptyList()).map { it.activityInfo.packageName }.toSet()
        val own = all.filter { it.activityInfo.packageName == context.packageName }.maxOfOrNull { it.priority } ?: 0
        // Below zero are the system's stand-ins, such as the one shown before the device unlocks.
        val others = all.filter { it.activityInfo.packageName != context.packageName && it.priority >= 0 }
            .groupBy { it.activityInfo.packageName }
            .map { (pkg, found) ->
                HomeApp(pkg, found.first().loadLabel(pm).toString(), found.maxOf { it.priority }, pkg in pickable, found.first().activityInfo.applicationInfo.enabled)
            }
        return own to others
    }

    /**
     * Everything that makes Lounge the home screen: choosing it, then switching off each home
     * screen Android would still pick first. Reads the package list, so call it off the main thread.
     */
    fun takeHome(): List<PowerAction> {
        val (own, others) = homes()
        return listOf(homeRole) + outranking(others, own).map { home ->
            PowerAction(
                "home_off_${home.packageName}",
                "Turn off ${home.label}",
                "Android opens it before any chosen home. It stays installed and can be turned back on here.",
                disableCommand(home.packageName, userId),
            )
        }
    }

    /** Home screens Android would open before Lounge. Reads the package list, so call it off the main thread. */
    fun homesFirst(): List<HomeApp> = homes().let { (own, others) -> outranking(others, own) }

    /** Home screens switched off as a whole app. Reads the package list, so call it off the main thread. */
    fun homesOff(): List<HomeApp> = switchedOff(homes().second)

    /** Turns back on the home screens that are switched off. Reads the package list, so call it off the main thread. */
    fun giveBackHome(): List<PowerAction> = homesOff().map { home ->
        PowerAction(
            "home_on_${home.packageName}",
            "Turn ${home.label} back on",
            "Do this before uninstalling Lounge, or the TV has no home screen to fall back on.",
            enableCommand(home.packageName, userId),
        )
    }

    private val ambient = ComponentName(context.packageName, "tv.lounge.launcher.AmbientDreamService").flattenToShortString()

    /** Prints the screensaver's enabled flag as it was (see [previousScreensaverEnabled]), then switches to Lounge's. */
    val useAmbient = PowerAction(
        "ambient",
        "Use Lounge as the screensaver",
        "Makes Lounge's the screensaver the TV starts when it is left idle.",
        "settings get secure screensaver_enabled; settings put secure screensaver_components $ambient; settings put secure screensaver_enabled 1",
    )

    /**
     * Puts the screensaver back: [component] as it was, and the enabled flag as [useAmbient] printed
     * it ("1", "0" or "null" for unset). An unknown flag is left alone.
     */
    fun restoreScreensaver(component: String?, enabled: String?): PowerAction {
        val restoreComponent = if (component.isNullOrBlank()) "settings delete secure screensaver_components" else "settings put secure screensaver_components ${shellQuote(component)}"
        val restoreEnabled = when (enabled) {
            "0", "1" -> "; settings put secure screensaver_enabled $enabled"
            "null" -> "; settings delete secure screensaver_enabled"
            else -> ""
        }
        return PowerAction("ambient_restore", "Back to the previous screensaver", "Puts the system screensaver back the way it was.", restoreComponent + restoreEnabled)
    }

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
        const val DEBUGGING_OFF = "USB debugging is off. Turn it on in Developer options, then come back."
        const val SHIZUKU_WAITING = "Shizuku is installed but hasn't allowed Lounge yet. Choose Use Shizuku instead, or turn on USB debugging."
        const val NOT_CONNECTED = "Power Setup isn't connected. In Settings, open Power Setup and choose Connect."

        /** How much of a failed command's output is logged. */
        private const val LOGGED_OUTPUT = 400

        /** A command that exited with an error, as a sentence chosen by what it printed. */
        fun failureSentence(output: String): String {
            val o = output.lowercase()
            return when {
                "securityexception" in o || "permission denial" in o || "not allowed" in o ->
                    "This TV doesn't let USB debugging do that. Look in Developer options for a setting that allows it, then try again."
                "unknown command" in o || "can't find service" in o || "no shell command implementation" in o ->
                    "This TV's version of Android has no command for that, so Lounge can't do it here."
                "protected package" in o -> "Android protects that app, so it can't be turned off."
                else -> "The TV didn't do that. Try again."
            }
        }

        private val REMOTE_KEYS = mapOf(
            android.view.KeyEvent.KEYCODE_DPAD_UP to "KEYCODE_DPAD_UP",
            android.view.KeyEvent.KEYCODE_DPAD_DOWN to "KEYCODE_DPAD_DOWN",
            android.view.KeyEvent.KEYCODE_DPAD_LEFT to "KEYCODE_DPAD_LEFT",
            android.view.KeyEvent.KEYCODE_DPAD_RIGHT to "KEYCODE_DPAD_RIGHT",
            android.view.KeyEvent.KEYCODE_DPAD_CENTER to "KEYCODE_DPAD_CENTER",
            android.view.KeyEvent.KEYCODE_BACK to "KEYCODE_BACK",
            android.view.KeyEvent.KEYCODE_MENU to "KEYCODE_MENU",
        )

        /**
         * The home screens Android picks before one of [ownPriority], whichever home is chosen: it
         * goes by the priority a filter declares first, and an app's own count as zero.
         */
        fun outranking(homes: List<HomeApp>, ownPriority: Int): List<HomeApp> = homes.filter { it.enabled && it.priority > ownPriority }

        /** Home screens whose whole app is off. One with only its activity off, as a finished setup leaves it, is left alone. */
        fun switchedOff(homes: List<HomeApp>): List<HomeApp> = homes.filter { !it.appEnabled }

        /** What a status check shows: [found], or [failure] from the last Connect while the TV is still only waiting to be paired. */
        fun keptFailure(found: PowerStatus, failure: PowerStatus.Failed?): PowerStatus =
            if (found == PowerStatus.NotPaired && failure != null) failure else found

        /** The action that undoes [id], or null when it has none. */
        fun oppositeOf(id: String): String? = when {
            id.startsWith("home_off_") -> "home_on_" + id.removePrefix("home_off_")
            id.startsWith("home_on_") -> "home_off_" + id.removePrefix("home_on_")
            id == "animations" -> "animations_reset"
            id == "animations_reset" -> "animations"
            else -> null
        }

        /** [results] with [id]'s new [result]. A success drops the result of the action it undoes, which no longer holds. */
        fun withResult(results: Map<String, PowerResult>, id: String, result: PowerResult): Map<String, PowerResult> =
            (if (result.ok) results - listOfNotNull(oppositeOf(id)).toSet() else results) + (id to result)

        fun homeCommand(component: String, userId: Int) = "cmd package set-home-activity --user $userId $component"
        fun grantCommand(packageName: String, permission: String, userId: Int) = "pm grant --user $userId $packageName $permission"
        fun disableCommand(packageName: String, userId: Int) = "pm disable-user --user $userId ${shellQuote(packageName)}"
        fun enableCommand(packageName: String, userId: Int) = "pm enable --user $userId ${shellQuote(packageName)}"

        /** The enabled flag [useAmbient] printed before switching: "1", "0" or "null", else null. */
        fun previousScreensaverEnabled(output: String): String? =
            output.lineSequence().firstOrNull()?.trim()?.takeIf { it == "0" || it == "1" || it == "null" }

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
