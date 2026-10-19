package tv.lounge.core

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

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

    fun installPlugin(path: String, builtIn: Boolean): Result<PluginInfo> = runCatching { core.installPlugin(path, builtIn) }

    fun removePlugin(id: String) = core.removePlugin(id)

    fun setPluginEnabled(id: String, enabled: Boolean) = core.setPluginEnabled(id, enabled)

    fun setPluginSetting(id: String, key: String, value: String?) = core.setPluginSetting(id, key, value)

    fun refreshPlugin(id: String) = core.refreshPlugin(id)

    fun pluginLog(id: String): List<String> = core.pluginLog(id)

    fun startCompanion(port: Int): Result<CompanionStatus> = runCatching { core.startCompanion(port.toUShort()) }

    fun stopCompanion() = core.stopCompanion()

    fun companionStatus(): CompanionStatus = core.companionStatus()

    fun companionPairing(): Result<PairingInfo> = runCatching { core.companionPairing() }

    fun companionForgetAll() = core.companionForgetAll()

    fun setPermissions(permissions: Permissions) = core.setPermissions(permissions)

    fun setUpdateNotice(version: String?) = core.setUpdateNotice(version)

    fun availableChannels(): List<ChannelInfo> = core.availableChannels()

    fun diagnostics(): Diagnostics = core.diagnostics()
}
