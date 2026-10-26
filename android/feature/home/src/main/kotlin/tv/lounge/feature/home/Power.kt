package tv.lounge.feature.home

import android.content.Intent
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.tv.material3.Text
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme
import tv.lounge.power.CecDevice
import tv.lounge.power.PowerAction
import tv.lounge.power.PowerSetup
import tv.lounge.power.PowerStatus
import tv.lounge.power.PrivilegedException

/** How one action went, under its row. */
private sealed interface RunState {
    data object Running : RunState
    data class Done(val output: String) : RunState
    data class Failed(val reason: String) : RunState
}

/**
 * Power Setup: connect the launcher to the device's own debugging service (or Shizuku) and
 * run the handful of shell actions an app cannot do on its own. Nothing runs unasked.
 */
@Composable
fun PowerSetupSheet(power: PowerSetup, onHomeRoleChanged: () -> Unit, firstFocus: FocusRequester) {
    val colors = LoungeTheme.colors
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val status by power.status.collectAsStateWithLifecycle()
    val runs = remember { mutableStateMapOf<String, RunState>() }
    var pairing by remember { mutableStateOf(false) }
    // Forgetting the key makes a new one, so the fingerprint is read again on every status change.
    val fingerprint by produceState<String?>(null, status) { value = power.fingerprint() }
    // Home screens to turn off or back on, read again after any home action runs.
    var homesChanged by remember { mutableIntStateOf(0) }
    val homeActions by produceState(emptyList<PowerAction>(), homesChanged) {
        value = withContext(Dispatchers.IO) { power.takeHome().filter { it.id.startsWith("home_off_") } + power.giveBackHome() }
    }

    WhileStarted {
        power.refresh()
        while (true) {
            delay(if (power.status.value is PowerStatus.Ready) 15_000 else 6_000)
            if (!pairing) power.refresh()
        }
    }

    fun run(action: PowerAction) {
        if (runs[action.id] == RunState.Running) return
        runs[action.id] = RunState.Running
        scope.launch {
            try {
                val r = power.run(action)
                runs[action.id] = if (r.ok) RunState.Done(r.output) else RunState.Failed(r.output.ifBlank { "exit ${r.exitCode}" })
                if (action.id.startsWith("home")) {
                    onHomeRoleChanged()
                    homesChanged++
                }
            } catch (e: PrivilegedException) {
                runs[action.id] = RunState.Failed(e.message ?: "failed")
            }
        }
    }

    val ready = status is PowerStatus.Ready
    SheetHeading(
        "Power Setup",
        when (val s = status) {
            is PowerStatus.Ready -> if (s.backend == "shizuku") "Connected through Shizuku." else "Connected to this device's own debugging service."
            PowerStatus.WaitingForApproval -> "Allow it on the prompt this device is showing. Tick “Always allow” so you only do this once."
            PowerStatus.NotPaired -> "Lets Lounge do the few things an app can't: become the home screen, grant its own accesses, control the TV over HDMI-CEC."
            is PowerStatus.Unavailable -> s.reason
            is PowerStatus.Failed -> s.reason
        },
    )

    Column(verticalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(8.dp)) {
        val s = status
        val unavailable = s is PowerStatus.Unavailable
        SheetRow(
            icon = if (unavailable) LoungeIcons.Settings else LoungeIcons.Power,
            title = when (s) {
                is PowerStatus.Ready -> "Connected"
                PowerStatus.WaitingForApproval -> "Waiting for you to allow it…"
                is PowerStatus.Unavailable -> "Open Developer options"
                else -> "Connect"
            },
            value = when (s) {
                is PowerStatus.Ready -> if (s.backend == "shizuku") "Shizuku" else "This device"
                PowerStatus.WaitingForApproval -> fingerprint?.let { "Key ${it.take(11)}…" } ?: "Look at the prompt"
                is PowerStatus.Unavailable -> "Turn on USB debugging"
                else -> if (pairing) "Connecting…" else "Asks once on screen"
            },
            accent = when (s) {
                is PowerStatus.Ready -> colors.signal
                is PowerStatus.Unavailable -> colors.dust
                else -> colors.houselight
            },
            onClick = {
                when (s) {
                    is PowerStatus.Ready, PowerStatus.WaitingForApproval -> Unit
                    is PowerStatus.Unavailable -> runCatching { context.startActivity(Intent(android.provider.Settings.ACTION_APPLICATION_DEVELOPMENT_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
                    else -> if (!pairing) {
                        pairing = true
                        scope.launch { power.pair(); pairing = false }
                    }
                }
            },
            focusRequester = firstFocus,
        )
        if (!ready && power.shizukuInstalled) {
            SheetRow(icon = LoungeIcons.Apps, title = "Use Shizuku instead", value = "Ask", onClick = { power.requestShizuku(7001); scope.launch { delay(1500); power.refresh() } })
        }
    }

    Spacer(Modifier.height(28.dp))
    Text("What it can do", style = LoungeTheme.typography.meta, color = colors.dust)
    Spacer(Modifier.height(8.dp))
    Column(verticalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(12.dp)) {
        power.setupActions.flatMap { if (it == power.homeRole) listOf(it) + homeActions else listOf(it) }
            .forEach { action -> ActionRow(action, runs[action.id], enabled = ready) { run(action) } }
    }

    if (ready) {
        Spacer(Modifier.height(20.dp))
        SheetRow(icon = LoungeIcons.Trash, title = "Forget this connection", value = "Asks again next time", onClick = { power.forget() })
    }
    SheetFootnote(
        "Nothing runs until you press it, and every command is shown first.",
        fingerprint?.let { "The key stays on this device: $it" } ?: "The key stays on this device.",
    )
}

@Composable
private fun ActionRow(action: PowerAction, state: RunState?, enabled: Boolean, onRun: () -> Unit) {
    val colors = LoungeTheme.colors
    Column(verticalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(6.dp)) {
        SheetRow(
            icon = LoungeIcons.Power,
            title = action.title,
            value = when (state) {
                RunState.Running -> "Running…"
                is RunState.Done -> "Done"
                is RunState.Failed -> "Failed"
                null -> if (enabled) "Run" else "Connect first"
            },
            accent = when (state) {
                is RunState.Done -> colors.signal
                is RunState.Failed -> colors.ember
                RunState.Running -> colors.houselight
                null -> colors.dust
            },
            onClick = { if (enabled) onRun() },
        )
        Column(Modifier.padding(start = 20.dp, end = 4.dp), verticalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(6.dp)) {
            Text(action.detail, style = LoungeTheme.typography.caption, color = colors.dust)
            Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(10.dp)).background(colors.velvet).padding(horizontal = 12.dp, vertical = 8.dp)) {
                Text(action.command, style = LoungeTheme.typography.caption.copy(fontFamily = FontFamily.Monospace), color = colors.houselight, maxLines = 3)
            }
            if (state is RunState.Failed) Text(state.reason.lineSequence().firstOrNull().orEmpty(), style = LoungeTheme.typography.caption, color = colors.ember, maxLines = 2)
            if (state is RunState.Done && state.output.isNotBlank()) Text(state.output.lineSequence().firstOrNull().orEmpty(), style = LoungeTheme.typography.caption, color = colors.dust, maxLines = 2)
        }
    }
}

/**
 * The TV controls card's sheet: power, input and sound over HDMI-CEC through Power Setup,
 * with volume and transport working even without it.
 */
@Composable
fun TvControlsSheet(power: PowerSetup, deviceName: String, onKey: (String) -> Unit, onPowerSetup: () -> Unit, firstFocus: FocusRequester) {
    val colors = LoungeTheme.colors
    val scope = rememberCoroutineScope()
    val status by power.status.collectAsStateWithLifecycle()
    val runs = remember { mutableStateMapOf<String, RunState>() }
    var devices by remember { mutableStateOf<List<CecDevice>>(emptyList()) }
    val ready = status is PowerStatus.Ready

    LaunchedEffect(Unit) { power.refresh() }
    LaunchedEffect(ready) { if (ready) devices = power.cecDevices() }

    fun run(action: PowerAction) {
        if (runs[action.id] == RunState.Running) return
        runs[action.id] = RunState.Running
        scope.launch {
            runs[action.id] = try {
                val r = power.run(action)
                if (r.ok) RunState.Done(r.output) else RunState.Failed(r.output.ifBlank { "exit ${r.exitCode}" })
            } catch (e: PrivilegedException) {
                RunState.Failed(e.message ?: "failed")
            }
            delay(2500)
            runs.remove(action.id)
        }
    }

    fun valueFor(action: PowerAction, idle: String? = null) = when (val s = runs[action.id]) {
        RunState.Running -> "Sending…"
        is RunState.Done -> "Sent"
        is RunState.Failed -> s.reason.lineSequence().firstOrNull()?.take(28) ?: "Failed"
        null -> idle
    }
    fun accentFor(action: PowerAction) = when (runs[action.id]) {
        is RunState.Done -> colors.signal
        is RunState.Failed -> colors.ember
        RunState.Running -> colors.houselight
        null -> colors.dust
    }

    SheetHeading("TV controls", if (ready) "$deviceName and the TV, over HDMI-CEC." else "Volume and playback work now. Power and input need Power Setup.")

    Column(verticalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(8.dp)) {
        val tv = devices.firstOrNull { it.isTv }
        SheetRow(
            icon = if (ready) LoungeIcons.Tv else LoungeIcons.Power,
            title = if (ready) "Wake the TV and switch to me" else "Set up TV power and input",
            value = if (ready) valueFor(power.oneTouchPlay, tv?.let { if (it.isOn) "TV is on" else "TV is off" }) else if (status is PowerStatus.Unavailable) "Needs USB debugging" else "Power Setup",
            accent = if (ready) accentFor(power.oneTouchPlay) else colors.houselight,
            onClick = { if (ready) run(power.oneTouchPlay) else onPowerSetup() },
            focusRequester = firstFocus,
        )
        if (ready) {
            SheetRow(icon = LoungeIcons.Power, title = "TV power", value = valueFor(power.tvPower, "On or off"), accent = accentFor(power.tvPower), onClick = { run(power.tvPower) })
            SheetRow(icon = LoungeIcons.Power, title = "Everything off", value = valueFor(power.everythingOff, "Sleep, TV follows"), accent = if (runs[power.everythingOff.id] == null) colors.ember else accentFor(power.everythingOff), onClick = { run(power.everythingOff) })
            if (power.isTelevision) {
                SheetRow(icon = LoungeIcons.Soundbar, title = "Sound through the soundbar", value = valueFor(power.systemAudioOn), accent = accentFor(power.systemAudioOn), onClick = { run(power.systemAudioOn) })
                SheetRow(icon = LoungeIcons.Soundbar, title = "Sound through the TV", value = valueFor(power.systemAudioOff), accent = accentFor(power.systemAudioOff), onClick = { run(power.systemAudioOff) })
            }
            val others = devices.filter { !it.isTv && it.physicalAddress != 0 }
            if (others.size > 1) {
                Spacer(Modifier.height(12.dp))
                Text("Switch the TV to", style = LoungeTheme.typography.meta, color = colors.dust)
                others.forEach { d ->
                    val action = power.selectDevice(d)
                    SheetRow(icon = LoungeIcons.Hdmi, title = d.name, value = valueFor(action, if (d.isOn) "On" else "Standby"), accent = accentFor(action), onClick = { run(action) })
                }
            }
        }
        Spacer(Modifier.height(12.dp))
        Text("Sound and playback", style = LoungeTheme.typography.meta, color = colors.dust)
        SheetRow(icon = LoungeIcons.Volume, title = "Volume up", onClick = { onKey("volume_up") })
        SheetRow(icon = LoungeIcons.Volume, title = "Volume down", onClick = { onKey("volume_down") })
        SheetRow(icon = LoungeIcons.Mute, title = "Mute", onClick = { onKey("mute") })
        SheetRow(icon = LoungeIcons.Play, title = "Play or pause", onClick = { onKey("play_pause") })
    }
    SheetFootnote(if (ready) "Commands go over HDMI-CEC; the TV decides what it honours." else "Volume reaches the TV or soundbar through HDMI-CEC the way the system remote does.")
}
