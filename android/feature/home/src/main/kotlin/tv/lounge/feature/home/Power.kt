package tv.lounge.feature.home

import android.content.Context
import android.content.Intent
import android.content.res.Resources
import androidx.annotation.StringRes
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableLongStateOf
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
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.tv.material3.Text
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import tv.lounge.core.PlatformException
import tv.lounge.core.TvControl
import tv.lounge.core.TvControls
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme
import tv.lounge.design.R as DesignR
import tv.lounge.power.CecDevice
import tv.lounge.power.PowerAction
import tv.lounge.power.PowerSetup
import tv.lounge.power.PowerStatus
import tv.lounge.power.PrivilegedException
import tv.lounge.power.sleepStep

/** How one action went, under its row; `output` is what its command printed. */
private sealed interface RunState {
    data object Running : RunState
    data class Done(val output: String) : RunState
    data class Failed(val reason: String, val output: String = "") : RunState
    /** It ran without an error and the TV still says otherwise; [next] is what to do about it. */
    data class DidntTake(@StringRes val next: Int, val output: String = "") : RunState
}

/** What staying on means, said before anyone turns USB debugging on for Power Setup. */
@StringRes
internal val DEBUGGING_STAYS_ON: Int = R.string.power_debugging_stays_on

/** How to turn USB debugging on, step by step, for Power Setup and the setup steps. */
@Composable
internal fun DebuggingHowTo() {
    val colors = LoungeTheme.colors
    Column(verticalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(4.dp)) {
        Text(stringResource(R.string.power_debugging_how_heading), style = LoungeTheme.typography.caption, color = colors.limelight)
        Text(stringResource(R.string.power_debugging_how_1), style = LoungeTheme.typography.caption, color = colors.dust)
        Text(stringResource(R.string.power_debugging_how_2), style = LoungeTheme.typography.caption, color = colors.dust)
        Text(stringResource(R.string.power_debugging_how_3), style = LoungeTheme.typography.caption, color = colors.dust)
    }
}

/** One glyph for each thing a Power Setup action sets up. */
private fun glyphFor(action: PowerAction): ImageVector = when {
    action.id.startsWith("home") -> SettingsGlyphs.Home
    action.id == "tv_listings" -> LoungeIcons.Tv
    action.id == "notifications" -> LoungeIcons.Equaliser
    action.id == "usage" -> SettingsGlyphs.Clock
    action.id.startsWith("animations") -> LoungeIcons.Move
    action.id == "optimise" -> SettingsGlyphs.Gauge
    else -> LoungeIcons.Power
}

/**
 * Power Setup: connect the launcher to the TV's own USB debugging or to Shizuku, and run the
 * handful of shell actions an app cannot do on its own. Nothing runs unasked, and each action
 * shows what is already so and how it last went.
 */
@Composable
fun PowerSetupSheet(power: PowerSetup, onHomeRoleChanged: () -> Unit, say: (String) -> Unit, firstFocus: FocusRequester) {
    val colors = LoungeTheme.colors
    val context = LocalContext.current
    val resources = LocalResources.current
    val scope = rememberCoroutineScope()
    val status by power.status.collectAsStateWithLifecycle()
    val results by power.results.collectAsStateWithLifecycle()
    // Power Setup runs the actions and keeps how each went, so both outlive this sheet.
    val running by power.running.collectAsStateWithLifecycle()
    val applied by power.applied.collectAsStateWithLifecycle()
    var pairing by remember { mutableStateOf(false) }
    var forgetting by remember { mutableStateOf(false) }
    // Forgetting the key makes a new one, so the fingerprint is read again on every status change.
    val fingerprint by produceState<String?>(null, status) { value = power.fingerprint() }
    // What is already so on the TV is read again on return to Lounge; an action reads it as it finishes.
    var checks by remember { mutableIntStateOf(0) }
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) { checks++ }
    LaunchedEffect(checks) { power.recheck() }
    val homeActions by produceState(emptyList<PowerAction>(), checks, running) {
        value = withContext(Dispatchers.IO) { power.takeHome().filter { it.id.startsWith("home_off_") } + power.giveBackHome() }
    }
    val isHome = applied[power.homeRole.id]
    LaunchedEffect(isHome) { onHomeRoleChanged() }
    // The undo for faster animations stays on offer once it has been shown, so focus never loses its row.
    var offerUndo by remember { mutableStateOf(false) }
    if (applied[power.normalAnimations.id] == false) offerUndo = true

    WhileStarted {
        power.refresh()
        while (true) {
            delay(if (power.status.value is PowerStatus.Ready) 15_000 else 6_000)
            if (!pairing) power.refresh()
        }
    }

    fun stateOf(action: PowerAction): RunState? {
        val last = results[action.id]
        return when {
            action.id in running -> RunState.Running
            applied[action.id] == true -> RunState.Done(last?.takeIf { it.ok }?.output.orEmpty())
            last != null && !last.ok -> RunState.Failed(last.said ?: PowerSetup.failureSentence(resources, last.output), last.output)
            last != null && PowerSetup.didntTake(applied[action.id], last) -> RunState.DidntTake(notTaken(action, power, homeActions.any { it.id.startsWith("home_off_") }), last.output)
            applied[action.id] == false -> null
            last != null -> RunState.Done(last.output)
            else -> null
        }
    }

    val ready = status is PowerStatus.Ready
    SheetHeading(
        stringResource(DesignR.string.common_power_setup),
        when (val s = status) {
            is PowerStatus.Ready -> stringResource(if (s.backend == "shizuku") R.string.power_status_shizuku else R.string.power_status_debugging)
            PowerStatus.WaitingForApproval -> stringResource(R.string.power_status_waiting)
            PowerStatus.NotPaired -> stringResource(R.string.power_status_not_paired)
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
                is PowerStatus.Ready -> stringResource(R.string.power_connect_connected)
                PowerStatus.WaitingForApproval -> stringResource(R.string.power_connect_waiting)
                is PowerStatus.Unavailable -> stringResource(R.string.power_connect_open_developer_options)
                else -> stringResource(R.string.power_connect)
            },
            value = when (s) {
                is PowerStatus.Ready -> if (s.backend == "shizuku") "Shizuku" else stringResource(R.string.power_connect_by_debugging)
                PowerStatus.WaitingForApproval -> fingerprint?.let { stringResource(R.string.power_connect_key, it.take(11)) } ?: stringResource(R.string.power_connect_look_at_prompt)
                is PowerStatus.Unavailable -> stringResource(R.string.power_connect_turn_on_debugging)
                else -> stringResource(if (pairing) R.string.power_connect_connecting else R.string.power_connect_asks_once)
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
            SheetRow(icon = LoungeIcons.Power, title = stringResource(R.string.power_shizuku_instead), value = stringResource(R.string.power_shizuku_ask), onClick = { power.requestShizuku(7001); scope.launch { delay(1500); power.refresh() } })
        }
        if (!ready) {
            Column(Modifier.padding(start = 20.dp, end = 4.dp, top = 4.dp), verticalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(10.dp)) {
                if (s is PowerStatus.Unavailable && s.debuggingOff) DebuggingHowTo()
                Text(stringResource(DEBUGGING_STAYS_ON), style = LoungeTheme.typography.caption, color = colors.dust)
            }
        }
    }

    Spacer(Modifier.height(28.dp))
    Text(stringResource(R.string.power_actions_heading), style = LoungeTheme.typography.meta, color = colors.dust)
    Spacer(Modifier.height(8.dp))
    Column(verticalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(12.dp)) {
        power.setupActions.flatMap {
            when (it) {
                power.homeRole -> listOf(it) + homeActions
                power.quickAnimations -> if (offerUndo) listOf(it, power.normalAnimations) else listOf(it)
                else -> listOf(it)
            }
        }.forEach { action -> ActionRow(action, stateOf(action), enabled = ready) { power.start(action) } }
    }

    if (ready) {
        Spacer(Modifier.height(20.dp))
        val forgotten = stringResource(R.string.power_forgotten)
        // Forgetting asks once more first, like removing a plugin; moving away takes the question back.
        SheetRow(
            icon = LoungeIcons.Trash,
            title = stringResource(if (forgetting) R.string.power_forget_confirm else R.string.power_forget),
            titleColor = if (forgetting) colors.ember else colors.limelight,
            subtitle = stringResource(R.string.power_forget_subtitle),
            modifier = Modifier.onFocusChanged { if (!it.isFocused) forgetting = false },
            onClick = {
                if (!forgetting) {
                    forgetting = true
                } else {
                    forgetting = false
                    power.forget()
                    say(forgotten)
                    scope.launch { firstFocus.requestWhenReady() }
                }
            },
        )
    }
    SheetFootnote(
        stringResource(R.string.power_footnote_shown_first),
        fingerprint?.let { stringResource(R.string.power_footnote_key, it) } ?: stringResource(R.string.power_footnote_key_unknown),
    )
}

@Composable
private fun ActionRow(action: PowerAction, state: RunState?, enabled: Boolean, onRun: () -> Unit) {
    val colors = LoungeTheme.colors
    Column(verticalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(6.dp)) {
        SheetRow(
            icon = glyphFor(action),
            title = action.title,
            value = when (state) {
                RunState.Running -> stringResource(R.string.power_action_running)
                is RunState.Done -> stringResource(R.string.power_action_done)
                is RunState.Failed -> stringResource(R.string.power_action_failed)
                is RunState.DidntTake -> stringResource(R.string.power_action_didnt_take)
                null -> stringResource(if (enabled) R.string.power_action_run else R.string.power_action_connect_first)
            },
            accent = when (state) {
                is RunState.Done -> colors.signal
                is RunState.Failed, is RunState.DidntTake -> colors.ember
                RunState.Running -> colors.houselight
                null -> colors.dust
            },
            onClick = { if (enabled) onRun() },
        )
        Column(Modifier.padding(start = 20.dp, end = 4.dp), verticalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(6.dp)) {
            Text(action.detail, style = LoungeTheme.typography.caption, color = colors.dust)
            val printed = when (state) {
                is RunState.Done -> state.output
                is RunState.Failed -> state.output
                is RunState.DidntTake -> state.output
                else -> ""
            }.trim()
            val mono = LoungeTheme.typography.caption.copy(fontFamily = FontFamily.Monospace)
            Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(10.dp)).background(colors.velvet).padding(horizontal = 12.dp, vertical = 8.dp)) {
                Text(action.command, style = mono, color = colors.houselight, maxLines = 3)
                // What the command printed stays in its box; the row itself speaks in sentences.
                if (printed.isNotEmpty()) Text(printed, style = mono, color = colors.dust, maxLines = 3, overflow = TextOverflow.Ellipsis)
            }
            if (state is RunState.Failed) Text(state.reason, style = LoungeTheme.typography.caption, color = colors.ember)
            if (state is RunState.DidntTake) Text(stringResource(state.next), style = LoungeTheme.typography.caption, color = colors.ember)
        }
    }
}

/** What to do about [action] when it ran without an error and changed nothing; [homesFirst] when a home to turn off is listed below. */
@StringRes
private fun notTaken(action: PowerAction, power: PowerSetup, homesFirst: Boolean): Int = when {
    action.id != power.homeRole.id -> R.string.power_not_taken
    homesFirst -> R.string.power_not_taken_homes_first
    else -> R.string.power_home_not_taken
}

/** A TV control that did not work, as a sentence with what to try. */
private fun controlFailure(resources: Resources, action: PowerAction, power: PowerSetup): String = when (action) {
    power.everythingOff -> resources.getString(R.string.power_control_no_sleep)
    else -> resources.getString(R.string.power_control_no_answer)
}

/** The other devices the TV can be switched to, when there is more than this one to choose from. */
internal fun switchable(devices: List<CecDevice>): List<CecDevice> =
    devices.filter { !it.isTv && it.physicalAddress != 0 }.takeIf { it.size > 1 }.orEmpty()

/**
 * What the phone lists as TV controls. A box plugged into a TV gets the TV controls sheet's
 * power and input rows, each under the id of the action it runs. A television, or a box whose
 * cable carries no control, can only be put to sleep. Its words are in the language of [resources].
 */
internal fun phoneTvControls(resources: Resources, ready: Boolean, hasCec: Boolean, isTelevision: Boolean, devices: List<CecDevice>, sleepLeft: Int? = null): TvControls {
    fun text(@StringRes id: Int) = resources.getString(id)
    // A press steps the timer on to its next time and then off, as OK does on the TV.
    val timer = TvControl(SLEEP_TIMER, text(R.string.power_sleep_timer), sleepLeft?.let { resources.getQuantityString(R.plurals.power_sleep_timer_left, it, it) } ?: text(DesignR.string.common_off), null, "sleep")
    // A timer left running while Power Setup is away is still listed, so it can be taken back.
    if (!ready) return TvControls(note = text(R.string.power_phone_needs_power_setup), controls = if (sleepLeft != null) listOf(timer) else emptyList())
    if (isTelevision) return TvControls(note = null, controls = listOf(TvControl("sleep", text(R.string.power_phone_turn_tv_off), null, null, "power"), timer))
    if (!hasCec) return TvControls(note = null, controls = listOf(TvControl("sleep", text(R.string.power_phone_sleep), null, null, "power"), timer))
    val tv = devices.firstOrNull { it.isTv }
    val rows = buildList {
        add(TvControl("cec_otp", text(R.string.power_phone_wake), tv?.let { text(if (it.isOn) R.string.power_tv_is_on else R.string.power_tv_is_off) }, null, "tv"))
        add(TvControl("cec_tv_power", text(R.string.power_tv_power), text(R.string.power_tv_power_value), null, "power"))
        add(TvControl("sleep", text(R.string.power_everything_off), text(R.string.power_everything_off_value), null, "power"))
        add(timer)
        switchable(devices).forEach { d ->
            add(TvControl("cec_select_${d.logicalAddress}", d.name, text(if (d.isOn) R.string.power_device_on else R.string.power_device_standby), text(R.string.power_switch_tv_to), "input"))
        }
    }
    return TvControls(note = null, controls = rows)
}

/** The id of the sleep timer among the phone's TV controls. */
internal const val SLEEP_TIMER = "sleep_timer"

/** The minutes a sleep timer ending at [endsAt] has left at [now], rounded up, or null with none running. */
internal fun sleepMinutesLeft(endsAt: Long?, now: Long): Int? = endsAt?.let { ((it - now + 59_999) / 60_000).toInt().coerceAtLeast(1) }

/** How long the phone's list waits for Power Setup, which is busy while it is being connected. */
private const val PHONE_STATUS_WAIT_MS = 4_000L

/** The TV's power and input controls for a paired phone: what there is, and running one. Their words come from [context]. */
class PhoneTvControls(private val power: PowerSetup, private val context: Context) {
    /** The rows as they stand now. Nothing is shown on the TV to find out. */
    suspend fun list(): TvControls {
        val status = withTimeoutOrNull(PHONE_STATUS_WAIT_MS) { power.refresh() } ?: power.status.value
        val ready = status is PowerStatus.Ready
        val onCable = ready && power.hasTvControls
        return phoneTvControls(context.resources, ready, power.hasCec, power.isTelevision, if (onCable) power.cecDevices() else emptyList(), sleepLeft())
    }

    private fun sleepLeft(): Int? = sleepMinutesLeft(power.sleepTimer.endsAt.value, System.currentTimeMillis())

    /** Runs the row with this id, if it is one the list holds now. A failure is thrown as a sentence saying what to check. */
    suspend fun run(id: String) {
        if (id == SLEEP_TIMER) {
            // Without Power Setup a press can only take a running timer back.
            val ready = power.status.value is PowerStatus.Ready
            if (!ready && sleepLeft() == null) throw PlatformException.Failed(context.getString(R.string.power_phone_needs_power_setup))
            val minutes = if (ready) sleepStep(sleepLeft(), forward = true) else 0
            return if (minutes == 0) power.sleepTimer.cancel() else power.sleepTimer.set(minutes)
        }
        val devices = if (power.hasTvControls) power.cecDevices() else emptyList()
        val listed = phoneTvControls(context.resources, ready = true, power.hasCec, power.isTelevision, devices).controls.any { it.id == id }
        val action = when {
            !listed -> null
            id == power.oneTouchPlay.id -> power.oneTouchPlay
            id == power.tvPower.id -> power.tvPower
            id == power.everythingOff.id -> power.everythingOff
            else -> switchable(devices).map { power.selectDevice(it) }.firstOrNull { it.id == id }
        } ?: throw PlatformException.Failed(context.getString(R.string.power_phone_control_gone))
        val worked = try {
            power.run(action).ok
        } catch (e: PrivilegedException) {
            throw PlatformException.Failed(e.message ?: controlFailure(context.resources, action, power))
        }
        if (!worked) throw PlatformException.Failed(controlFailure(context.resources, action, power))
    }
}

/**
 * The TV controls card's sheet: power, input and sound through Power Setup, with volume and
 * playback working even without it.
 */
@Composable
fun TvControlsSheet(
    power: PowerSetup,
    deviceName: String,
    onKey: (String) -> Unit,
    onPowerSetup: () -> Unit,
    firstFocus: FocusRequester,
    /** Where the TV's sound comes out now, as the home already knows it. */
    soundOutput: () -> SoundOutput? = { null },
) {
    val colors = LoungeTheme.colors
    val resources = LocalResources.current
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
            val result = try {
                val r = power.run(action)
                if (r.ok) RunState.Done(r.output) else RunState.Failed(controlFailure(resources, action, power))
            } catch (e: PrivilegedException) {
                RunState.Failed(e.message ?: controlFailure(resources, action, power))
            }
            runs[action.id] = result
            // A failure stays with its row until it is pressed again, so there is time to read it.
            if (result is RunState.Done) {
                delay(2500)
                if (runs[action.id] == result) runs.remove(action.id)
            }
        }
    }

    @Composable
    fun valueFor(action: PowerAction, idle: String? = null) = when (runs[action.id]) {
        RunState.Running -> stringResource(R.string.power_control_sending)
        is RunState.Done -> stringResource(R.string.power_control_sent)
        is RunState.Failed, is RunState.DidntTake -> stringResource(R.string.power_control_didnt_work)
        null -> idle
    }
    fun reasonFor(action: PowerAction) = (runs[action.id] as? RunState.Failed)?.reason
    fun accentFor(action: PowerAction) = when (runs[action.id]) {
        is RunState.Done -> colors.signal
        is RunState.Failed, is RunState.DidntTake -> colors.ember
        RunState.Running -> colors.houselight
        null -> colors.dust
    }

    SheetHeading(stringResource(DesignR.string.common_tv_controls), stringResource(if (ready) R.string.power_controls_subtitle_ready else R.string.power_controls_subtitle_not_ready))

    Column(verticalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(8.dp)) {
        val tv = devices.firstOrNull { it.isTv }
        SheetRow(
            icon = if (ready) LoungeIcons.Tv else LoungeIcons.Power,
            title = stringResource(if (ready) R.string.power_controls_wake else R.string.power_controls_set_up),
            subtitle = if (ready) reasonFor(power.oneTouchPlay) else null,
            subtitleLines = WHOLE,
            value = when {
                ready -> valueFor(power.oneTouchPlay, tv?.let { stringResource(if (it.isOn) R.string.power_tv_is_on else R.string.power_tv_is_off) })
                status is PowerStatus.Unavailable -> stringResource(R.string.power_controls_needs_debugging)
                else -> stringResource(DesignR.string.common_power_setup)
            },
            accent = if (ready) accentFor(power.oneTouchPlay) else colors.houselight,
            onClick = { if (ready) run(power.oneTouchPlay) else onPowerSetup() },
            focusRequester = firstFocus,
        )
        if (ready) {
            SheetRow(icon = LoungeIcons.Power, title = stringResource(R.string.power_tv_power), subtitle = reasonFor(power.tvPower), subtitleLines = WHOLE, value = valueFor(power.tvPower, stringResource(R.string.power_tv_power_value)), accent = accentFor(power.tvPower), onClick = { run(power.tvPower) })
            SheetRow(icon = LoungeIcons.Power, title = stringResource(R.string.power_everything_off), subtitle = reasonFor(power.everythingOff), subtitleLines = WHOLE, value = valueFor(power.everythingOff, stringResource(R.string.power_everything_off_value)), accent = if (runs[power.everythingOff.id] == null) colors.ember else accentFor(power.everythingOff), onClick = { run(power.everythingOff) })
        }
        // Left and Right step the time, and OK steps it on to the next and then off. A timer left
        // running while Power Setup is away still shows, so it can be taken back.
        val endsAt by power.sleepTimer.endsAt.collectAsStateWithLifecycle()
        var clock by remember { mutableLongStateOf(System.currentTimeMillis()) }
        LaunchedEffect(endsAt) {
            while (endsAt != null) {
                clock = System.currentTimeMillis()
                delay(15_000)
            }
        }
        val left = endsAt?.let { ((it - clock + 59_999) / 60_000).toInt().coerceAtLeast(1) }
        fun step(forward: Boolean) {
            val minutes = if (ready) sleepStep(left, forward) else 0
            if (minutes == 0) power.sleepTimer.cancel() else power.sleepTimer.set(minutes)
            clock = System.currentTimeMillis()
        }
        if (ready || endsAt != null) SheetRow(
            icon = LoungeIcons.Sleep,
            title = stringResource(R.string.power_sleep_timer),
            subtitle = stringResource(R.string.power_sleep_timer_subtitle),
            value = left?.let { pluralStringResource(R.plurals.power_sleep_timer_left, it, it) } ?: stringResource(DesignR.string.common_off),
            accent = if (left != null) colors.houselight else colors.dust,
            onClick = { step(forward = true) },
            onCycle = { by -> step(forward = by > 0) },
        )
        if (ready) {
            if (power.isTelevision) {
                SheetRow(icon = LoungeIcons.Soundbar, title = stringResource(R.string.power_controls_sound_soundbar), subtitle = reasonFor(power.systemAudioOn), subtitleLines = WHOLE, value = valueFor(power.systemAudioOn), accent = accentFor(power.systemAudioOn), onClick = { run(power.systemAudioOn) })
                SheetRow(icon = LoungeIcons.Soundbar, title = stringResource(R.string.power_controls_sound_tv), subtitle = reasonFor(power.systemAudioOff), subtitleLines = WHOLE, value = valueFor(power.systemAudioOff), accent = accentFor(power.systemAudioOff), onClick = { run(power.systemAudioOff) })
            }
            val others = switchable(devices)
            if (others.isNotEmpty()) {
                Spacer(Modifier.height(12.dp))
                Text(stringResource(R.string.power_switch_tv_to), style = LoungeTheme.typography.meta, color = colors.dust)
                others.forEach { d ->
                    val action = power.selectDevice(d)
                    SheetRow(icon = LoungeIcons.Hdmi, title = d.name, subtitle = reasonFor(action), subtitleLines = WHOLE, value = valueFor(action, stringResource(if (d.isOn) R.string.power_device_on else R.string.power_device_standby)), accent = accentFor(action), onClick = { run(action) })
                }
            }
        }
        Spacer(Modifier.height(12.dp))
        Text(stringResource(R.string.power_controls_sound_heading), style = LoungeTheme.typography.meta, color = colors.dust)
        // Where the sound is going now. OK opens the TV's own sound screen, where it is changed.
        val context = LocalContext.current
        val soundSettings = remember { soundSettingsIntent(context) }
        val output = soundOutput()
        if (output != null && soundSettings != null) {
            SheetRow(
                icon = LoungeIcons.Soundbar,
                title = stringResource(R.string.power_controls_sound_output),
                value = output.label(resources),
                onClick = { runCatching { context.startActivity(soundSettings) } },
            )
        }
        SheetRow(icon = LoungeIcons.Volume, title = stringResource(R.string.power_controls_volume_up), onClick = { onKey("volume_up") })
        SheetRow(icon = LoungeIcons.Volume, title = stringResource(R.string.power_controls_volume_down), onClick = { onKey("volume_down") })
        SheetRow(icon = LoungeIcons.Mute, title = stringResource(R.string.power_controls_mute), onClick = { onKey("mute") })
        SheetRow(icon = LoungeIcons.Play, title = stringResource(R.string.power_controls_play_pause), onClick = { onKey("play_pause") })
    }
    SheetFootnote(stringResource(if (ready) R.string.power_controls_footnote_ready else R.string.power_controls_footnote_not_ready))
}
