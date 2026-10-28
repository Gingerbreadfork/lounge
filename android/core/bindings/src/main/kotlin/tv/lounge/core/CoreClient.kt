package tv.lounge.core

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** Why a core call failed, as the core put it, without the binding's field names. */
val Throwable.reason: String
    get() = when (this) {
        is CoreException.Storage -> detail
        is CoreException.Platform -> detail
        is CoreException.Plugin -> detail
        is CoreException.Companion -> detail
        is CoreException.UnknownAction -> "Lounge doesn't know how to do $action"
        is CoreException.DiskFull -> "The TV's storage is full, or would not read or write"
        is PlatformException.Failed -> detail
        else -> message ?: javaClass.simpleName
    }

/** The companion's failure to start, worded for the TV. */
fun companionFailure(e: Throwable): String =
    if ("in use" in e.reason) "Another app on this TV is using port ${CoreClient.COMPANION_PORT}, so the companion can't start."
    else "The companion couldn't start: ${e.reason}"

/**
 * Adapter over the generated [Core]: exposes [HomeSnapshot]s as a [StateFlow] and keeps the
 * generated types out of the UI layer's constructors.
 */
class CoreClient(val core: Core) : SnapshotListener {
    private val _snapshots = MutableStateFlow(core.snapshot())
    val snapshots: StateFlow<HomeSnapshot> = _snapshots.asStateFlow()

    init {
        core.addListener(this)
    }

    override fun onSnapshot(snapshot: HomeSnapshot) {
        _snapshots.value = snapshot
    }

    fun ingestApps(apps: List<AppRecord>) = core.ingestApps(apps)

    fun reportUsage(usage: List<AppUsage>) = core.reportUsage(usage)

    fun command(command: LayoutCommand) = core.command(command)

    fun perform(action: String): Result<Unit> = runCatching { core.perform(action) }

    fun setScreenState(on: Boolean) = core.setScreenState(on)

    fun setUtcOffsetMinutes(minutes: Int) = core.setUtcOffsetMinutes(minutes)

    fun ingestPrograms(programs: List<ProgramRecord>) = core.ingestPrograms(programs)

    fun ingestSessions(sessions: List<SessionRecord>) = core.ingestSessions(sessions)

    fun ingestInputs(inputs: List<InputRecord>) = core.ingestInputs(inputs)

    fun guide(fromMs: Long, hours: UInt): Guide = core.guide(fromMs, hours)

    fun plugins(): List<PluginInfo> = core.plugins()

    fun screensavers(): List<ShaderScreensaver> = runCatching { core.screensavers() }.getOrDefault(emptyList())

    fun installPlugin(path: String, builtIn: Boolean): Result<PluginInfo> = runCatching { core.installPlugin(path, builtIn) }

    fun removePlugin(id: String) = core.removePlugin(id)

    fun setPluginEnabled(id: String, enabled: Boolean) = core.setPluginEnabled(id, enabled)

    fun setPluginSetting(id: String, key: String, value: String?) = core.setPluginSetting(id, key, value)

    fun refreshPlugin(id: String) = core.refreshPlugin(id)

    fun pluginLog(id: String): List<String> = core.pluginLog(id)

    /** Rows from the search plugins; blocks while they answer, so call it off the main thread. */
    fun searchPlugins(query: String): List<Row> = runCatching { core.searchPlugins(query) }.getOrDefault(emptyList())

    fun startCompanion(port: Int): Result<CompanionStatus> = runCatching { core.startCompanion(port.toUShort()) }

    fun stopCompanion() = core.stopCompanion()

    fun companionStatus(): CompanionStatus = core.companionStatus()

    fun companionPairing(): Result<PairingInfo> = runCatching { core.companionPairing() }

    fun companionForgetAll() = core.companionForgetAll()

    fun setPermissions(permissions: Permissions) = core.setPermissions(permissions)

    fun setUpdateNotice(version: String?) = core.setUpdateNotice(version)

    fun availableChannels(): List<ChannelInfo> = core.availableChannels()

    fun diagnostics(): Diagnostics = core.diagnostics()

    companion object {
        /** The port the phone companion listens on; the pairing page and the README name it. */
        const val COMPANION_PORT = 8787
    }
}
