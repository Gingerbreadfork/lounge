package tv.lounge.feature.home

import android.content.Intent
import android.net.Uri
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.foundation.background
import androidx.compose.foundation.focusGroup
import androidx.compose.foundation.focusable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.tv.material3.Icon
import androidx.tv.material3.Text
import tv.lounge.core.ChannelInfo
import tv.lounge.core.CoreClient
import tv.lounge.core.Diagnostics
import tv.lounge.core.PairingInfo
import tv.lounge.core.companionFailure
import tv.lounge.core.reason
import tv.lounge.design.Dimens
import tv.lounge.design.Dot
import tv.lounge.design.Hairline
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme
import tv.lounge.design.LocalSwapSlotActive
import tv.lounge.design.Swap
import tv.lounge.design.sounds
import tv.lounge.design.focusRing
import tv.lounge.design.lightGlow
import tv.lounge.platform.Prefs
import tv.lounge.power.PowerAction

/**
 * Right-hand `curtain` sheet over a scrim (DESIGN.md §5.4). One panel serves every sheet: it slides
 * in once, and moving between sheets crossfades the content with an 8 px shift in the direction of
 * travel. [visibleState] comes from `Presence` so Back plays the exit; [depth] gives the shift's
 * direction.
 */
@Composable
fun <K : Any> RightSheet(
    visibleState: MutableTransitionState<Boolean>,
    key: K,
    depth: Int,
    onDismiss: () -> Unit,
    content: @Composable (firstFocus: FocusRequester, key: K) -> Unit,
) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    var lastDepth by remember { mutableStateOf(depth) }
    val forward = depth >= lastDepth
    LaunchedEffect(depth) { lastDepth = depth }
    val leaving = !visibleState.targetState
    Box(Modifier.fillMaxSize()) {
        AnimatedVisibility(visibleState, enter = fadeIn(tween(200)), exit = fadeOut(tween(180))) {
            Box(Modifier.fillMaxSize().background(colors.scrim))
        }
        AnimatedVisibility(
            visibleState,
            enter = slideInHorizontally(motion.proscenium()) { it } + fadeIn(motion.proscenium()),
            exit = slideOutHorizontally(motion.settle()) { it } + fadeOut(motion.settle()),
            modifier = Modifier.align(Alignment.CenterEnd),
        ) {
            Row(Modifier.fillMaxHeight().width(Dimens.sheetWidth)) {
                Box(Modifier.width(1.dp).fillMaxHeight().background(colors.curtainEdge))
                Column(
                    Modifier
                        .fillMaxSize()
                        .background(colors.curtain)
                        .onPreviewKeyEvent { e -> if (e.isBack() && e.type == KeyEventType.KeyUp) { onDismiss(); true } else false }
                        .focusProperties { canFocus = !leaving }
                        .trapFocus(leaving)
                        .focusGroup()
                        // Half the side padding sits outside the scroll, so a focused row's glow has room.
                        .padding(horizontal = 24.dp, vertical = Dimens.safeY)
                        .verticalScroll(rememberScrollState())
                        .padding(horizontal = 24.dp),
                ) {
                    Swap(key, rise = 0.dp, shiftX = if (forward) 8.dp else (-8).dp, durationMs = 180) { shown ->
                        val firstFocus = remember { FocusRequester() }
                        Column { content(firstFocus, shown) }
                        // A sheet coming back mid-swap is still composed, so focus follows the live copy.
                        val live = LocalSwapSlotActive.current.value
                        LaunchedEffect(live) { if (live) firstFocus.requestWhenReady() }
                    }
                }
            }
        }
    }
}

/**
 * A 72 dp sheet row: glyph, title, optional trailing value. Focus is a lifted `curtainEdge` surface
 * with a glow; rested rows paint nothing. An `accent` of `signal` becomes a status dot. Rows with
 * [onCycle] change their value with Left and Right and show chevrons.
 */
@Composable
fun SheetRow(
    icon: ImageVector?,
    title: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    value: String? = null,
    accent: Color = LoungeTheme.colors.dust,
    focusRequester: FocusRequester? = null,
    height: Dp = 72.dp,
    onCycle: ((Int) -> Unit)? = null,
    leading: (@Composable () -> Unit)? = null,
    /** A second line under the title, for rows that must stay narrow; the row grows past [height] when large text needs it. */
    subtitle: String? = null,
) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    val visuals = LoungeTheme.visuals
    var focused by remember { mutableStateOf(false) }
    val lift = animateFloatAsState(if (focused) 1f else 0f, if (focused) motion.snap() else motion.settle(), label = "rowLift")
    val glow = animateFloatAsState(if (focused) visuals.glowAlpha * 0.5f else 0f, if (focused) motion.snap() else motion.settle(), label = "rowGlow")
    val status = accent == colors.signal
    val iconTint = when {
        focused -> colors.houselight
        status -> colors.dust
        else -> accent
    }
    var direction by remember { mutableStateOf(1) }
    Row(
        modifier
            .then(if (focusRequester != null) Modifier.focusRequester(focusRequester) else Modifier)
            .fillMaxWidth()
            .heightIn(min = height)
            .lightGlow(colors.houselight, { glow.value }, cornerRadius = 16.dp)
            .drawBehind {
                val a = lift.value
                if (a > 0.001f) drawRoundRect(colors.curtainEdge.copy(alpha = colors.curtainEdge.alpha * a), cornerRadius = CornerRadius(16.dp.toPx()))
            }
            .clip(RoundedCornerShape(16.dp))
            .focusRing(colors, { focused }, 16.dp)
            .onFocusChanged { focused = it.isFocused }
            .then(
                if (onCycle != null) {
                    Modifier.onPreviewKeyEvent { e ->
                        if (e.type == KeyEventType.KeyDown && (e.key == Key.DirectionLeft || e.key == Key.DirectionRight)) {
                            direction = if (e.key == Key.DirectionRight) 1 else -1
                            onCycle(direction)
                            true
                        } else {
                            false
                        }
                    }
                } else {
                    Modifier
                },
            )
            .tilePress({ direction = 1; onClick() }, onClick)
            .focusable()
            .semantics { role = Role.Button }
            .padding(horizontal = 20.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        if (leading != null) leading()
        if (icon != null) Icon(icon, contentDescription = null, tint = iconTint, modifier = Modifier.size(28.dp))
        if (subtitle != null) {
            Column(Modifier.weight(1f).padding(vertical = 4.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(title, style = LoungeTheme.typography.title, color = colors.limelight, maxLines = 2, overflow = TextOverflow.Ellipsis)
                Text(subtitle, style = LoungeTheme.typography.caption, color = colors.dust, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
        } else {
            // A long title wraps to a second line and the row grows, rather than losing its end.
            Text(title, style = LoungeTheme.typography.title, color = colors.limelight, maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f).padding(vertical = 8.dp))
        }
        if (value != null) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                if (status) Dot(colors.signal)
                if (onCycle != null && focused) Text("‹", style = LoungeTheme.typography.meta, color = colors.dust)
                Swap(value, rise = 0.dp, shiftX = if (onCycle != null) (8 * direction).dp else 0.dp, durationMs = 180) { v ->
                    Text(v, style = LoungeTheme.typography.meta, color = colors.dust, maxLines = 1)
                }
                if (onCycle != null && focused) Text("›", style = LoungeTheme.typography.meta, color = colors.dust)
            }
        }
    }
}

@Composable
fun SheetHeading(title: String, subtitle: String? = null) {
    val colors = LoungeTheme.colors
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        // A heading and a live region: TalkBack says it when the sheet opens.
        Text(
            title,
            style = LoungeTheme.typography.headline,
            color = colors.limelight,
            modifier = Modifier.semantics { heading(); liveRegion = LiveRegionMode.Polite },
        )
        if (subtitle != null) Text(subtitle, style = LoungeTheme.typography.meta, color = colors.dust)
    }
    Spacer(Modifier.height(32.dp))
}

/** A quiet label over a group of rows; not focusable. */
@Composable
fun SheetGroup(label: String) {
    Text(label, style = LoungeTheme.typography.caption, color = LoungeTheme.colors.dust, modifier = Modifier.padding(start = 20.dp, top = 8.dp, bottom = 6.dp))
}

@Composable
fun SheetFootnote(vararg lines: String) {
    val colors = LoungeTheme.colors
    Spacer(Modifier.height(32.dp))
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        lines.forEach { Text(it, style = LoungeTheme.typography.caption, color = colors.dust) }
    }
}

private fun Prefs.Theme.step(by: Int): Prefs.Theme {
    val all = Prefs.Theme.entries
    return all[(all.indexOf(this) + by + all.size) % all.size]
}

private fun Boolean?.stepTri(by: Int): Boolean? = when (this) {
    null -> if (by > 0) true else false
    true -> if (by > 0) false else null
    false -> if (by > 0) null else true
}

/**
 * Quick settings: look and sound, what Lounge may see, and this TV. Essentials only; the rest
 * lives in the companion (DESIGN.md §3.3). Values cycle with Left and Right.
 */
@Composable
fun QuickSettingsSheet(
    prefs: Prefs,
    theme: Prefs.Theme,
    sounds: Boolean,
    lite: Boolean?,
    reducedMotion: Boolean?,
    showMobileApps: Boolean,
    isDefaultHome: Boolean,
    tvListings: Boolean,
    notifications: Boolean,
    usage: Boolean,
    enabledChannels: Int,
    enabledPlugins: Int,
    diagnostics: () -> Diagnostics,
    onSetTheme: (Prefs.Theme) -> Unit,
    onSetSounds: (Boolean) -> Unit,
    onSetLite: (Boolean?) -> Unit,
    onSetReducedMotion: (Boolean?) -> Unit,
    onSetShowMobileApps: (Boolean) -> Unit,
    onRequestPermission: (String) -> Unit,
    onChannels: () -> Unit,
    onPlugins: () -> Unit,
    onCompanion: () -> Unit,
    companionOn: Boolean,
    onRecheckHome: () -> Unit,
    onPowerSetup: () -> Unit,
    powerStatus: tv.lounge.power.PowerStatus,
    onAmbient: () -> Unit,
    ambientOn: Boolean,
    onSetupAgain: () -> Unit,
    onUpdates: () -> Unit,
    updateStatus: String,
    firstFocus: FocusRequester,
    backdrop: String = Prefs.STILL_BACKDROP,
    backdrops: List<ShaderStyle> = emptyList(),
    liveBackdrops: Boolean = true,
    onSetBackdrop: (String) -> Unit = {},
) {
    val context = LocalContext.current
    val colors = LoungeTheme.colors
    val sounds0 = LoungeTheme.sounds
    var showDiagnostics by remember { mutableStateOf(false) }
    var diag by remember { mutableStateOf<Diagnostics?>(null) }

    SheetHeading("Settings")
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        SheetGroup("Look and sound")
        SheetRow(
            icon = LoungeIcons.Theme,
            title = "Theme",
            value = theme.title,
            onClick = { sounds0?.tick(); onSetTheme(theme.step(1)) },
            onCycle = { by -> sounds0?.tick(); onSetTheme(theme.step(by)) },
            focusRequester = firstFocus,
        )
        val scenes = listOf(Triple(Prefs.STILL_BACKDROP, "Still", "The room's own light")) + backdrops.map { Triple(it.id, it.name, it.description ?: it.origin) }
        val sceneAt = scenes.indexOfFirst { it.first == backdrop }.coerceAtLeast(0)
        SheetRow(
            icon = LoungeIcons.Frame,
            title = "Background",
            value = scenes[sceneAt].second,
            subtitle = if (!liveBackdrops && sceneAt > 0) "Still while Lite mode or Reduce motion is on" else scenes[sceneAt].third,
            onClick = { sounds0?.tick(); onSetBackdrop(scenes[(sceneAt + 1).mod(scenes.size)].first) },
            onCycle = { by -> sounds0?.tick(); onSetBackdrop(scenes[(sceneAt + by).mod(scenes.size)].first) },
        )
        SheetRow(
            icon = LoungeIcons.Volume,
            title = "Sounds",
            value = if (sounds) "On" else "Off",
            onClick = { sounds0?.tick(); onSetSounds(!sounds) },
            onCycle = { sounds0?.tick(); onSetSounds(!sounds) },
        )
        SheetRow(
            icon = LoungeIcons.Settings,
            title = "Lite mode",
            value = when (lite) { null -> "Automatic"; true -> "On"; false -> "Off" },
            onClick = { sounds0?.tick(); onSetLite(lite.stepTri(1)) },
            onCycle = { by -> sounds0?.tick(); onSetLite(lite.stepTri(by)) },
        )
        SheetRow(
            icon = LoungeIcons.Move,
            title = "Reduce motion",
            value = when (reducedMotion) { null -> "Follow system"; true -> "On"; false -> "Off" },
            onClick = { sounds0?.tick(); onSetReducedMotion(reducedMotion.stepTri(1)) },
            onCycle = { by -> sounds0?.tick(); onSetReducedMotion(reducedMotion.stepTri(by)) },
        )
        SheetRow(
            icon = LoungeIcons.Apps,
            title = "Phone apps in All apps",
            value = if (showMobileApps) "On" else "Off",
            onClick = { sounds0?.tick(); onSetShowMobileApps(!showMobileApps) },
            onCycle = { sounds0?.tick(); onSetShowMobileApps(!showMobileApps) },
        )
        Spacer(Modifier.height(16.dp))
        Hairline(Modifier.fillMaxWidth())
        SheetGroup("What Lounge can see")
        SheetRow(
            icon = LoungeIcons.Tv,
            title = "TV listings",
            value = if (tvListings) "On" else "Allow",
            accent = if (tvListings) colors.signal else colors.dust,
            onClick = { if (!tvListings) onRequestPermission("tv_listings") },
        )
        SheetRow(
            icon = LoungeIcons.Equaliser,
            title = "Now playing",
            value = if (notifications) "On" else "Allow",
            accent = if (notifications) colors.signal else colors.dust,
            onClick = { if (!notifications) onRequestPermission("notifications") },
        )
        SheetRow(
            icon = LoungeIcons.Apps,
            title = "Usage access",
            value = if (usage) "On" else "Allow",
            accent = if (usage) colors.signal else colors.dust,
            onClick = {
                runCatching { context.startActivity(Intent(android.provider.Settings.ACTION_USAGE_ACCESS_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
            },
        )
        SheetRow(icon = LoungeIcons.Tv, title = "Channels from apps", value = if (enabledChannels == 0) "None" else "$enabledChannels on", onClick = onChannels)
        SheetRow(icon = LoungeIcons.Apps, title = "Plugins", value = if (enabledPlugins == 0) "None on" else "$enabledPlugins on", onClick = onPlugins)
        SheetRow(icon = LoungeIcons.Phone, title = "Phone companion", value = if (companionOn) "On" else "Off", accent = if (companionOn) colors.signal else colors.dust, onClick = onCompanion)
        SheetRow(icon = LoungeIcons.Frame, title = "Screensaver", value = if (ambientOn) "Lounge" else "System", accent = if (ambientOn) colors.signal else colors.dust, onClick = onAmbient)
        Spacer(Modifier.height(16.dp))
        Hairline(Modifier.fillMaxWidth())
        SheetGroup("This TV")
        SheetRow(
            icon = LoungeIcons.Power,
            title = "Home screen",
            value = if (isDefaultHome) "Lounge" else "Set up",
            accent = if (isDefaultHome) colors.signal else colors.dust,
            onClick = onRecheckHome,
        )
        SheetRow(
            icon = LoungeIcons.Power,
            title = "Power Setup",
            value = when (powerStatus) {
                is tv.lounge.power.PowerStatus.Ready -> "Connected"
                tv.lounge.power.PowerStatus.WaitingForApproval -> "Waiting…"
                is tv.lounge.power.PowerStatus.Unavailable -> "Unavailable"
                else -> "Not connected"
            },
            accent = if (powerStatus is tv.lounge.power.PowerStatus.Ready) colors.signal else colors.dust,
            onClick = onPowerSetup,
        )
        SheetRow(icon = LoungeIcons.Refresh, title = "Updates", value = updateStatus, accent = if (updateStatus == "Up to date") colors.signal else colors.dust, onClick = onUpdates)
        SheetRow(icon = LoungeIcons.Info, title = "Diagnostics", value = if (showDiagnostics) "Hide" else "Show", onClick = { showDiagnostics = !showDiagnostics; if (showDiagnostics) diag = diagnostics() })
        if (showDiagnostics) {
            diag?.let { d ->
                Column(Modifier.padding(start = 20.dp, top = 4.dp, bottom = 8.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    DiagLine("Items in graph", d.items.toString())
                    DiagLine("Snapshot revision", d.revision.toString())
                    DiagLine("Core calls", d.ffiCalls.toString())
                    DiagLine("Last rebuild", "${d.lastBuildMicros / 1000u} ms")
                    DiagLine("Database", "${d.dbBytes / 1024u} KB")
                    DiagLine("Queue", d.queueDepth.toString())
                }
            }
        }
        SheetRow(icon = LoungeIcons.Refresh, title = "Run the setup steps again", onClick = onSetupAgain)
    }
    SheetFootnote("Left and Right change a value. Back closes.")
}

/**
 * The phone companion: a web app the TV serves on the home network. Pair by scanning the QR
 * code or typing the six digits; both expire after five minutes.
 */
@Composable
fun CompanionSheet(core: CoreClient, enabled: Boolean, onSetEnabled: (Boolean) -> Unit, firstFocus: FocusRequester) {
    val colors = LoungeTheme.colors
    var status by remember { mutableStateOf(core.companionStatus()) }
    var pairing by remember { mutableStateOf<PairingInfo?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    fun refresh() {
        status = core.companionStatus()
        if (status.running && (pairing == null || pairing!!.expiresMs < System.currentTimeMillis())) {
            core.companionPairing().onSuccess { pairing = it; error = null }.onFailure { error = it.reason }
        }
    }
    LaunchedEffect(enabled) {
        if (enabled && !core.companionStatus().running) {
            core.startCompanion(CoreClient.COMPANION_PORT).onFailure { error = companionFailure(it) }
        }
        if (!enabled) { core.stopCompanion(); pairing = null }
        refresh()
    }
    WhileStarted {
        while (true) {
            kotlinx.coroutines.delay(5000)
            refresh()
        }
    }

    SheetHeading("Phone companion", "Your phone becomes a remote, a keyboard and the place to arrange this screen.")
    SheetRow(
        icon = LoungeIcons.Phone,
        title = if (enabled) "On" else "Off",
        value = if (enabled) "Turn off" else "Turn on",
        accent = if (enabled && status.running) colors.signal else colors.dust,
        onClick = { onSetEnabled(!enabled) },
        focusRequester = firstFocus,
    )
    Spacer(Modifier.height(20.dp))
    if (enabled) {
        val p = pairing
        if (status.running && p != null) {
            Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.fillMaxWidth()) {
                Box(Modifier.size(250.dp).clip(RoundedCornerShape(16.dp)).background(colors.limelight)) {
                    coil3.compose.AsyncImage(model = p.qrPngPath, contentDescription = "Pairing code", modifier = Modifier.fillMaxSize())
                }
                Spacer(Modifier.height(20.dp))
                Text("Or type this code", style = LoungeTheme.typography.meta, color = colors.dust)
                Spacer(Modifier.height(6.dp))
                Text(p.code.chunked(3).joinToString(" "), style = LoungeTheme.typography.displayL, color = colors.limelight)
                Spacer(Modifier.height(10.dp))
                Text(status.url?.let { "at $it" } ?: "", style = LoungeTheme.typography.caption, color = colors.dust)
            }
            Spacer(Modifier.height(20.dp))
            SheetRow(icon = LoungeIcons.Refresh, title = "New code", onClick = { core.companionPairing().onSuccess { pairing = it } })
        } else if (error != null) {
            Text(error!!, style = LoungeTheme.typography.meta, color = colors.ember)
        } else {
            Text("Starting…", style = LoungeTheme.typography.meta, color = colors.dust)
        }
        Spacer(Modifier.height(8.dp))
        SheetRow(
            icon = LoungeIcons.Phone,
            title = "Paired phones",
            value = if (status.paired == 0u) "None" else "${status.paired}, forget all",
            onClick = { if (status.paired > 0u) { core.companionForgetAll(); refresh() } },
        )
    }
    SheetFootnote(
        "Everything stays on your home network.",
        "Add the page to your phone's home screen to use it like an app.",
    )
}

/** Rows other apps publish to the TV provider; each can be shown on the home screen. */
@Composable
fun ChannelsSheet(channels: List<ChannelInfo>, onToggle: (ChannelInfo) -> Unit, firstFocus: FocusRequester) {
    val colors = LoungeTheme.colors
    SheetHeading("Channels from apps", "Turn on the ones you want as rows on your home screen.")
    if (channels.isEmpty()) {
        Text("No app has published a channel yet.", style = LoungeTheme.typography.meta, color = colors.dust)
        Spacer(Modifier.height(24.dp))
        SheetRow(icon = LoungeIcons.Info, title = "Nothing to turn on", onClick = {}, focusRequester = firstFocus)
    } else {
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            channels.forEachIndexed { i, ch ->
                SheetRow(
                    icon = LoungeIcons.Tv,
                    title = ch.name,
                    value = (if (ch.enabled) "On" else "Off") + ", ${ch.programmes} programmes",
                    accent = if (ch.enabled) colors.signal else colors.dust,
                    onClick = { onToggle(ch) },
                    focusRequester = if (i == 0) firstFocus else null,
                )
            }
        }
    }
    SheetFootnote("Apps decide what goes in their channels; Lounge only chooses which to show.")
}

@Composable
private fun DiagLine(label: String, value: String) {
    val colors = LoungeTheme.colors
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
        Text(label, style = LoungeTheme.typography.caption, color = colors.dust)
        Text(value, style = LoungeTheme.typography.caption, color = colors.limelight)
    }
}

/**
 * The "make this my home screen" guide, honest about what each path needs. [takeHome] is what
 * "Do it now" runs, shown before it does; [homesOff] names the home screens switched off.
 */
@Composable
fun HomeRoleSheet(
    isDefaultHome: Boolean,
    packageName: String,
    powerReady: Boolean,
    takeHome: List<PowerAction>,
    homesOff: List<String>,
    onDoItNow: () -> Unit,
    onPowerSetup: () -> Unit,
    onRecheck: () -> Unit,
    firstFocus: FocusRequester,
) {
    val colors = LoungeTheme.colors
    val context = LocalContext.current
    val turnOff = takeHome.filter { it.id.startsWith("home_off_") }
    val them = if (turnOff.size == 1) "it" else "them"
    SheetHeading(
        if (isDefaultHome) "Lounge is your home screen" else "Make Lounge your home screen",
        if (isDefaultHome) "Pressing Home brings you here." else "Google TV has no setting for this. One of these works.",
    )
    if (!isDefaultHome) {
        SheetRow(
            icon = LoungeIcons.Power,
            title = if (powerReady) "Do it now" else "Let Lounge do it",
            value = if (powerReady) "Through Power Setup" else "Power Setup",
            accent = colors.houselight,
            onClick = if (powerReady) onDoItNow else onPowerSetup,
            focusRequester = firstFocus,
        )
        if (turnOff.isNotEmpty()) {
            Spacer(Modifier.height(12.dp))
            Step(
                if (turnOff.size == 1) "This TV's own home comes first" else "This TV's own homes come first",
                "Android opens $them before any chosen home, so this also turns $them off. Nothing is uninstalled; " +
                    "turn $them back on in Power Setup, and do that before you uninstall Lounge.",
                takeHome.joinToString("\n") { it.command },
            )
        }
        Spacer(Modifier.height(20.dp))
        Column(verticalArrangement = Arrangement.spacedBy(20.dp)) {
            Step(
                "Or from a computer, once",
                "Turn on Developer options and USB debugging on this device, connect with adb, then run:",
                takeHome.joinToString("\n") { "adb shell ${it.command}" }.ifEmpty { "adb shell cmd package set-home-activity $packageName/tv.lounge.launcher.MainActivity" },
            )
            Step(
                "Or let Android ask",
                "Some devices show a chooser the next time you press Home. Pick Lounge and choose Always.",
                null,
            )
        }
        Spacer(Modifier.height(24.dp))
        SheetRow(icon = LoungeIcons.Power, title = "Check again", onClick = onRecheck)
        SheetRow(
            icon = LoungeIcons.Settings,
            title = "Open Developer options",
            onClick = { runCatching { context.startActivity(Intent(android.provider.Settings.ACTION_APPLICATION_DEVELOPMENT_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) } },
        )
    } else {
        SheetRow(icon = LoungeIcons.Power, title = "Check again", onClick = onRecheck, focusRequester = firstFocus)
        if (homesOff.isNotEmpty()) {
            SheetRow(icon = LoungeIcons.Power, title = if (homesOff.size == 1) "Turn it back on" else "Turn them back on", value = "Power Setup", onClick = onPowerSetup)
        }
    }
    SheetFootnote(
        when {
            homesOff.isNotEmpty() -> {
                val off = if (homesOff.size == 1) "it" else "them"
                homesOff.joinToString(" and ") + (if (homesOff.size == 1) " is" else " are") +
                    " turned off. Turn $off back on before you uninstall Lounge, or the TV has no home screen to fall back on."
            }
            turnOff.isNotEmpty() -> "What $them provide${if (turnOff.size == 1) "s" else ""} may be missing while $them ${if (turnOff.size == 1) "is" else "are"} off."
            else -> "The stock launcher stays installed, so casting, the gear button and the programme guide keep working."
        },
    )
}

@Composable
private fun Step(title: String, body: String, command: String?) {
    val colors = LoungeTheme.colors
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text(title, style = LoungeTheme.typography.title, color = colors.limelight)
        Text(body, style = LoungeTheme.typography.meta, color = colors.dust)
        if (command != null) {
            Box(Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(colors.velvet).padding(16.dp)) {
                Text(command, style = LoungeTheme.typography.caption, color = colors.houselight)
            }
        }
    }
}

fun openPlayListing(context: android.content.Context, packageName: String) {
    val market = Intent(Intent.ACTION_VIEW, Uri.parse("market://details?id=$packageName")).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    runCatching { context.startActivity(market) }.onFailure {
        runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse("https://play.google.com/store/apps/details?id=$packageName")).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
    }
}

fun requestUninstall(context: android.content.Context, packageName: String) {
    val intent = Intent(Intent.ACTION_DELETE, Uri.parse("package:$packageName")).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    // Without the system's uninstaller, the app's page in Settings has the same button.
    runCatching { context.startActivity(intent) }.onFailure {
        val info = Intent(android.provider.Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:$packageName")).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        runCatching { context.startActivity(info) }
    }
}
