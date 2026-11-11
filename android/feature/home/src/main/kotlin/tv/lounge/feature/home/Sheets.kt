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
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.Stable
import androidx.compose.runtime.staticCompositionLocalOf
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
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import androidx.tv.material3.Icon
import androidx.tv.material3.Text
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import tv.lounge.core.ChannelInfo
import tv.lounge.core.CompanionStatus
import tv.lounge.core.CoreClient
import tv.lounge.core.Diagnostics
import tv.lounge.core.PairingInfo
import tv.lounge.core.reason
import tv.lounge.design.Dimens
import tv.lounge.design.Dot
import tv.lounge.design.Hairline
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme
import tv.lounge.design.LocalSwapSlotActive
import tv.lounge.design.Swap
import tv.lounge.design.sounds
import tv.lounge.design.focusRim
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
    val back = remember { SheetBack() }
    Box(Modifier.fillMaxSize()) {
        AnimatedVisibility(
            visibleState,
            enter = fadeIn(tween(if (motion.reducedMotion) 1 else 200)),
            exit = fadeOut(tween(if (motion.reducedMotion) 1 else 180)),
        ) {
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
                        .onPreviewKeyEvent { e ->
                            if (e.isBack() && e.type == KeyEventType.KeyUp) {
                                if (back.handler?.invoke() != true) onDismiss()
                                true
                            } else {
                                false
                            }
                        }
                        .focusProperties { canFocus = !leaving }
                        .trapFocus(leaving)
                        .focusGroup()
                        // Half the side padding sits outside the scroll, so a focused row's glow has room.
                        .padding(horizontal = 24.dp, vertical = Dimens.safeY)
                        .verticalScroll(rememberScrollState())
                        .padding(horizontal = 24.dp),
                ) {
                    // The trap only holds focus that is already inside, so a sheet still loading or
                    // with nothing to focus keeps it on this anchor instead of the home behind.
                    val anchor = remember { FocusRequester() }
                    var contentHasFocus by remember { mutableStateOf(false) }
                    Box(Modifier.size(1.dp).focusRequester(anchor).focusProperties { canFocus = !contentHasFocus }.focusable())
                    Swap(key, rise = 0.dp, shiftX = if (forward) 8.dp else (-8).dp, durationMs = 180) { shown ->
                        val firstFocus = remember { FocusRequester() }
                        val live = LocalSwapSlotActive.current.value
                        val scope = rememberCoroutineScope()
                        Column(
                            Modifier.onFocusChanged {
                                contentHasFocus = it.hasFocus
                                if (!it.hasFocus && live && !leaving) scope.launch { anchor.requestWhenReady() }
                            },
                        ) { CompositionLocalProvider(LocalSheetBack provides back) { content(firstFocus, shown) } }
                        // A sheet coming back mid-swap is still composed, so focus follows the live copy.
                        LaunchedEffect(live) {
                            if (live) {
                                anchor.requestWhenReady()
                                firstFocus.requestWhenReady()
                            }
                        }
                    }
                }
            }
        }
    }
}

/** Lets what a sheet shows take Back for itself, such as a row being moved; the sheet closes otherwise. */
class SheetBack {
    /** Returns true when it used the press. */
    var handler: (() -> Boolean)? = null
}

val LocalSheetBack = staticCompositionLocalOf { SheetBack() }

/**
 * A 72 dp sheet row: glyph, title, optional trailing value. Focus is a lifted `curtainEdge` surface
 * with a glow and the tiles' rim; rested rows paint nothing. An `accent` of `signal` adds a status
 * dot, and `signal`, `houselight` or `ember` colour the value. Rows with [onCycle] change their
 * value with Left and Right and show chevrons.
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
    /** Holding OK, where a row has a second action; a plain press otherwise. */
    onLongClick: (() -> Unit)? = null,
    subtitleLines: Int = 2,
    /** `ember` for the second step of something that cannot be undone. */
    titleColor: Color = LoungeTheme.colors.limelight,
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
    val valueColor = if (accent == colors.signal || accent == colors.houselight || accent == colors.ember) accent else colors.dust
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
            .focusRim(colors, { lift.value }, 16.dp)
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
            .tilePress({ direction = 1; onClick() }, onLongClick ?: onClick)
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
                Text(title, style = LoungeTheme.typography.title, color = titleColor, maxLines = 2, overflow = TextOverflow.Ellipsis)
                Text(subtitle, style = LoungeTheme.typography.caption, color = colors.dust, maxLines = subtitleLines, overflow = TextOverflow.Ellipsis)
            }
        } else {
            // A long title wraps to a second line and the row grows, rather than losing its end.
            Text(title, style = LoungeTheme.typography.title, color = titleColor, maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f).padding(vertical = 8.dp))
        }
        if (value != null) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                if (status) Dot(colors.signal)
                if (onCycle != null && focused) Text("‹", style = LoungeTheme.typography.meta, color = colors.dust)
                Swap(value, rise = 0.dp, shiftX = if (onCycle != null) (8 * direction).dp else 0.dp, durationMs = 180) { v ->
                    Text(v, style = LoungeTheme.typography.meta, color = valueColor, maxLines = 1)
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
 * Quick settings: the home screen, look and sound, what Lounge may see, and this TV. Essentials
 * only; the rest lives in the companion. Values cycle with Left and Right.
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
    enabledChannels: Int,
    enabledPlugins: Int,
    diagnostics: () -> Diagnostics,
    onSetTheme: (Prefs.Theme) -> Unit,
    onSetSounds: (Boolean) -> Unit,
    onSetLite: (Boolean?) -> Unit,
    onSetReducedMotion: (Boolean?) -> Unit,
    onSetShowMobileApps: (Boolean) -> Unit,
    /** Asks Android for an access through its own dialog. */
    onRequestPermission: (String) -> Unit,
    say: (String) -> Unit,
    onChannels: () -> Unit,
    onRows: () -> Unit,
    /** "All shown" or "2 hidden": what the row editor would say first. */
    rowsSummary: String,
    /** How many recently opened apps follow the pins in Your apps. */
    recentApps: Int,
    onSetRecentApps: (Int) -> Unit,
    onPlugins: () -> Unit,
    onCompanion: () -> Unit,
    companionOn: Boolean,
    onRecheckHome: () -> Unit,
    onPowerSetup: () -> Unit,
    powerStatus: tv.lounge.power.PowerStatus,
    onAmbient: () -> Unit,
    /** The screensaver's name as the Screensaver sheet gives it, read again on return to Lounge. */
    screensaver: () -> String,
    onSetupAgain: () -> Unit,
    onUpdates: () -> Unit,
    updateStatus: String,
    firstFocus: FocusRequester,
    cameFrom: Overlay? = null,
    backdrop: String = Prefs.STILL_BACKDROP,
    backdrops: List<ShaderStyle> = emptyList(),
    liveBackdrops: Boolean = true,
    onSetBackdrop: (String) -> Unit = {},
) {
    val colors = LoungeTheme.colors
    val sounds0 = LoungeTheme.sounds
    val liteNow = !LoungeTheme.visuals.shaderWallpaper
    var showDiagnostics by remember { mutableStateOf(false) }
    var diag by remember { mutableStateOf<Diagnostics?>(null) }
    LaunchedEffect(showDiagnostics) {
        diag = if (showDiagnostics) withContext(Dispatchers.IO) { diagnostics() } else null
    }
    val access = rememberAccessAsker(onRequestPermission, say, onPowerSetup)
    var saverName by remember { mutableStateOf(screensaver()) }
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) { saverName = screensaver() }
    // Back from one of its pages puts focus on the row that opened it.
    val pages = listOf(Overlay.Rows, Overlay.Channels, Overlay.Plugins, Overlay.Companion, Overlay.Ambient, Overlay.HomeRole, Overlay.PowerSetup, Overlay.Updates, Overlay.Onboarding)
    val returnTo = cameFrom?.takeIf { it in pages }
    fun focusFor(page: Overlay?) = firstFocus.takeIf { page == returnTo }

    SheetHeading("Settings")
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        SheetGroup("Home screen")
        SheetRow(
            icon = LoungeIcons.Rows,
            title = "Rows",
            subtitle = "Show, hide and move rows",
            value = rowsSummary,
            onClick = onRows,
            focusRequester = focusFor(Overlay.Rows) ?: focusFor(null),
        )
        val steps = listOf(0, 5, 10, 15)
        val at = steps.indexOf(recentApps).let { if (it < 0) 1 else it }
        SheetRow(
            icon = LoungeIcons.Apps,
            title = "Your apps",
            subtitle = "Pinned apps, then recent ones",
            value = if (recentApps == 0) "Pinned only" else "$recentApps recent",
            onClick = { sounds0?.tick(); onSetRecentApps(steps[(at + 1) % steps.size]) },
            onCycle = { by -> sounds0?.tick(); onSetRecentApps(steps[(at + by).mod(steps.size)]) },
        )
        SheetRow(
            icon = SettingsGlyphs.PhoneApps,
            title = "Phone apps in All apps",
            subtitle = "Apps made for phones, which may not work well with a remote",
            value = if (showMobileApps) "On" else "Off",
            accent = if (showMobileApps) colors.signal else colors.dust,
            onClick = { sounds0?.tick(); onSetShowMobileApps(!showMobileApps) },
            onCycle = { sounds0?.tick(); onSetShowMobileApps(!showMobileApps) },
        )
        SheetRow(
            icon = LoungeIcons.Guide,
            title = "Channels from apps",
            subtitle = "Rows that apps on this TV publish",
            value = if (enabledChannels == 0) "None" else "$enabledChannels on",
            onClick = onChannels,
            focusRequester = focusFor(Overlay.Channels),
        )
        SheetRow(
            icon = SettingsGlyphs.Plugin,
            title = "Plugins",
            subtitle = "Rows, channels and guide data from your services",
            value = if (enabledPlugins == 0) "None on" else "$enabledPlugins on",
            onClick = onPlugins,
            focusRequester = focusFor(Overlay.Plugins),
        )
        Spacer(Modifier.height(16.dp))
        Hairline(Modifier.fillMaxWidth())
        SheetGroup("Look and sound")
        SheetRow(
            icon = LoungeIcons.Theme,
            title = "Theme",
            value = theme.title,
            onClick = { sounds0?.tick(); onSetTheme(theme.step(1)) },
            onCycle = { by -> sounds0?.tick(); onSetTheme(theme.step(by)) },
        )
        val scenes = listOf(Triple(Prefs.STILL_BACKDROP, "Still", "The room's own light")) + backdrops.map { Triple(it.id, it.name, it.description ?: it.origin) }
        val sceneAt = scenes.indexOfFirst { it.first == backdrop }.coerceAtLeast(0)
        SheetRow(
            icon = SettingsGlyphs.Scene,
            title = "Background",
            value = scenes[sceneAt].second,
            subtitle = if (!liveBackdrops && sceneAt > 0) "Still while Lite mode or Reduce motion is on" else scenes[sceneAt].third,
            onClick = { sounds0?.tick(); onSetBackdrop(scenes[(sceneAt + 1).mod(scenes.size)].first) },
            onCycle = { by -> sounds0?.tick(); onSetBackdrop(scenes[(sceneAt + by).mod(scenes.size)].first) },
        )
        SheetRow(
            icon = LoungeIcons.Frame,
            title = "Screensaver",
            value = saverName,
            accent = if (saverName == "Lounge") colors.signal else colors.dust,
            onClick = onAmbient,
            focusRequester = focusFor(Overlay.Ambient),
        )
        SheetRow(
            icon = LoungeIcons.Volume,
            title = "Sounds",
            value = if (sounds) "On" else "Off",
            accent = if (sounds) colors.signal else colors.dust,
            onClick = { sounds0?.tick(); onSetSounds(!sounds) },
            onCycle = { sounds0?.tick(); onSetSounds(!sounds) },
        )
        SheetRow(
            icon = SettingsGlyphs.Gauge,
            title = "Lite mode",
            subtitle = when (lite) {
                null -> if (liteNow) "On for this TV now: no moving light or live backgrounds" else "Off for this TV now; on, it stops the moving light and live backgrounds"
                true -> "No moving light or live backgrounds, so the TV stays quick"
                false -> "The moving light and live backgrounds stay on"
            },
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
        Spacer(Modifier.height(16.dp))
        Hairline(Modifier.fillMaxWidth())
        SheetGroup("Access")
        Access.entries.forEach { a -> AccessRow(a, access) }
        Spacer(Modifier.height(16.dp))
        Hairline(Modifier.fillMaxWidth())
        SheetGroup("This TV")
        SheetRow(
            icon = SettingsGlyphs.Home,
            title = "Default home screen",
            subtitle = "What the Home button opens",
            value = if (isDefaultHome) "Lounge" else "Set up",
            accent = if (isDefaultHome) colors.signal else colors.dust,
            onClick = onRecheckHome,
            focusRequester = focusFor(Overlay.HomeRole),
        )
        SheetRow(
            icon = LoungeIcons.Power,
            title = "Power Setup",
            subtitle = "Lets Lounge do what an app can't on its own",
            value = when (powerStatus) {
                is tv.lounge.power.PowerStatus.Ready -> "Connected"
                tv.lounge.power.PowerStatus.WaitingForApproval -> "Waiting for you"
                else -> "Not connected"
            },
            accent = if (powerStatus is tv.lounge.power.PowerStatus.Ready) colors.signal else colors.dust,
            onClick = onPowerSetup,
            focusRequester = focusFor(Overlay.PowerSetup),
        )
        SheetRow(
            icon = LoungeIcons.Phone,
            title = "Phone companion",
            value = if (companionOn) "On" else "Off",
            accent = if (companionOn) colors.signal else colors.dust,
            onClick = onCompanion,
            focusRequester = focusFor(Overlay.Companion),
        )
        SheetRow(
            icon = SettingsGlyphs.Download,
            title = "Updates",
            value = updateStatus,
            accent = when {
                updateStatus == "Up to date" -> colors.signal
                updateStatus == "Problem" -> colors.ember
                updateStatus.endsWith(" available") || updateStatus.endsWith(" ready") -> colors.houselight
                else -> colors.dust
            },
            onClick = onUpdates,
            focusRequester = focusFor(Overlay.Updates),
        )
        SheetRow(
            icon = LoungeIcons.Info,
            title = "Diagnostics",
            subtitle = "Numbers to include when you report a problem",
            value = if (showDiagnostics) "Hide" else "Show",
            onClick = { showDiagnostics = !showDiagnostics },
        )
        if (showDiagnostics) {
            diag?.let { d ->
                Column(Modifier.padding(start = 20.dp, top = 4.dp, bottom = 8.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    DiagLine("Things Lounge knows about", d.items.toString())
                    DiagLine("Home screen version", d.revision.toString())
                    DiagLine("Requests answered", d.ffiCalls.toString())
                    DiagLine("Last rebuild took", "${d.lastBuildMicros / 1000u} ms")
                    DiagLine("Saved data", "${d.dbBytes / 1024u} KB")
                    DiagLine("Work waiting", d.queueDepth.toString())
                }
            }
        }
        SheetRow(icon = LoungeIcons.Refresh, title = "Run the setup steps again", onClick = onSetupAgain, focusRequester = focusFor(Overlay.Onboarding))
    }
    SheetFootnote("Left and Right change a value. Back closes.")
}

/** One of Lounge's optional accesses, as Settings and the setup steps name it. */
internal enum class Access(val key: String, val title: String, val what: String) {
    TvListings("tv_listings", "TV listings", "Continue watching, channels and the TV guide"),
    NowPlaying("notifications", "Now playing", "What is playing, with play and pause"),
    Usage("usage", "Usage access", "Lets Your apps follow what you open"),
}

private fun Access.granted(context: android.content.Context): Boolean = when (this) {
    Access.TvListings -> hasTvListings(context)
    Access.NowPlaying -> hasNotificationAccess(context)
    Access.Usage -> hasUsageAccess(context)
}

/** The system screen that allows an access; TV listings has a dialog instead. */
private fun Access.screen(): String? = when (this) {
    Access.TvListings -> null
    Access.NowPlaying -> android.provider.Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS
    Access.Usage -> android.provider.Settings.ACTION_USAGE_ACCESS_SETTINGS
}

private const val NO_SCREEN = "This TV has no screen for this. OK opens Power Setup, which can allow it."

/**
 * Lounge's accesses and how to ask for each without leaving the sheet. Once Android stops showing
 * its TV listings dialog, the next press opens Lounge's page in Android's settings; an access with
 * no screen on this TV goes to Power Setup. Every way out says where it goes first.
 */
@Stable
internal class AccessAsker(private val context: android.content.Context, private val scope: kotlinx.coroutines.CoroutineScope) {
    var granted by mutableStateOf(read())
        private set
    private val refused = androidx.compose.runtime.mutableStateMapOf<Access, Boolean>()
    internal var pausedAt = 0L
    private var leaving = false
    internal var requestDialog: (String) -> Unit = {}
    internal var say: (String) -> Unit = {}
    internal var onPowerSetup: () -> Unit = {}

    private fun read() = Access.entries.associateWith { it.granted(context) }

    fun reread() {
        granted = read()
    }

    fun isOn(a: Access) = granted[a] == true

    private fun opens(intent: Intent) = runCatching { intent.resolveActivity(context.packageManager) != null }.getOrDefault(false)

    /** What the row says under its title: what the access is for, or where OK goes instead. */
    fun note(a: Access): String {
        val screen = a.screen()
        return when {
            isOn(a) -> a.what
            refused[a] == true -> "Android won't ask again. OK opens Lounge's page in Android's settings."
            screen != null && !opens(Intent(screen)) -> NO_SCREEN
            else -> a.what
        }
    }

    fun ask(a: Access) {
        if (isOn(a) || leaving) return
        val screen = a.screen()
        when {
            screen != null -> leaveFor(Intent(screen), "Find Lounge in the list and turn it on, then press Back.", a)
            refused[a] == true -> leaveFor(
                Intent(android.provider.Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null)),
                "Open Permissions and allow ${a.title}, then press Back.",
                a,
            )
            else -> {
                val asked = android.os.SystemClock.uptimeMillis()
                requestDialog(a.key)
                scope.launch {
                    kotlinx.coroutines.delay(2_000)
                    reread()
                    // No dialog came up over Lounge, so Android has stopped asking.
                    if (!isOn(a) && pausedAt < asked) refused[a] = true
                }
            }
        }
    }

    /** Says [hint] and leaves it on screen a moment, then opens [intent]; Power Setup when nothing can. */
    private fun leaveFor(intent: Intent, hint: String, a: Access) {
        intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        val noScreen = "This TV has no screen to allow ${a.title}. Power Setup can allow it."
        if (!opens(intent)) {
            say(noScreen)
            onPowerSetup()
            return
        }
        leaving = true
        say(hint)
        scope.launch {
            kotlinx.coroutines.delay(1_500)
            leaving = false
            runCatching { context.startActivity(intent) }.onFailure {
                say(noScreen)
                onPowerSetup()
            }
        }
    }
}

/** An [AccessAsker] that reads the accesses again whenever Lounge comes back to the front. */
@Composable
internal fun rememberAccessAsker(requestDialog: (String) -> Unit, say: (String) -> Unit, onPowerSetup: () -> Unit): AccessAsker {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val asker = remember { AccessAsker(context, scope) }
    asker.requestDialog = requestDialog
    asker.say = say
    asker.onPowerSetup = onPowerSetup
    LifecycleEventEffect(Lifecycle.Event.ON_PAUSE) { asker.pausedAt = android.os.SystemClock.uptimeMillis() }
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) { asker.reread() }
    return asker
}

/** An access as a row: its state as the value, what it is for underneath, and OK asks for it. */
@Composable
internal fun AccessRow(access: Access, asker: AccessAsker, focusRequester: FocusRequester? = null) {
    val colors = LoungeTheme.colors
    val on = asker.isOn(access)
    SheetRow(
        icon = when (access) {
            Access.TvListings -> LoungeIcons.Tv
            Access.NowPlaying -> LoungeIcons.Equaliser
            Access.Usage -> SettingsGlyphs.Clock
        },
        title = access.title,
        subtitle = asker.note(access),
        value = if (on) "On" else "Not allowed",
        accent = if (on) colors.signal else colors.dust,
        onClick = { asker.ask(access) },
        focusRequester = focusRequester,
    )
}

/**
 * The phone companion: a web app the TV serves on the home network. Pair by scanning the QR
 * code or typing the six digits; both expire after five minutes, and a new code takes their
 * place as soon as a phone pairs.
 */
@Composable
fun CompanionSheet(core: CoreClient, enabled: Boolean, onSetEnabled: (Boolean) -> Unit, firstFocus: FocusRequester) {
    val colors = LoungeTheme.colors
    val scope = rememberCoroutineScope()
    var status by remember { mutableStateOf<CompanionStatus?>(null) }
    var pairing by remember { mutableStateOf<PairingInfo?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var forgetFailed by remember { mutableStateOf(false) }
    var forgetting by remember { mutableStateOf(false) }
    var forgotten by remember { mutableStateOf(false) }
    // Starting binds a port and a new code writes its picture, so the server's calls run off the main thread.
    suspend fun newCode() {
        withContext(Dispatchers.IO) { core.companionPairing() }.onSuccess { pairing = it; error = null }.onFailure { pairing = null; error = pairingProblem(it) }
    }
    suspend fun refresh() {
        val s = withContext(Dispatchers.IO) { core.companionStatus() }
        status = s
        if (!s.running) return
        // The core replaces a used code at once, so a new one is asked for only when it offers none.
        val live = s.livePairing()
        if (live != null) { pairing = live; error = null } else newCode()
    }
    LaunchedEffect(enabled) {
        if (enabled) {
            withContext(Dispatchers.IO) { if (core.companionStatus().running) null else core.startCompanion(CoreClient.COMPANION_PORT) }
                ?.onFailure { error = companionStartProblem(it) }
        } else {
            withContext(Dispatchers.IO) { core.stopCompanion() }
            pairing = null
        }
        refresh()
    }
    WhileStarted {
        while (true) {
            kotlinx.coroutines.delay(2000)
            refresh()
        }
    }

    SheetHeading("Phone companion", "Your phone becomes a remote, a keyboard and the place to arrange this screen.")
    val running = enabled && status?.running == true
    SheetRow(
        icon = LoungeIcons.Phone,
        title = "Phone companion",
        subtitle = if (enabled) null else "Turn it on to pair a phone",
        value = if (enabled) "On" else "Off",
        accent = if (running) colors.signal else colors.dust,
        onClick = { onSetEnabled(!enabled) },
        focusRequester = firstFocus,
    )
    Spacer(Modifier.height(20.dp))
    val s = status
    if (enabled) {
        val p = pairing
        val why = error
        if (why != null) {
            Text(why, style = LoungeTheme.typography.meta, color = colors.ember)
        } else if (s != null && s.running && p != null) {
            Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.fillMaxWidth()) {
                Box(Modifier.size(300.dp).clip(RoundedCornerShape(16.dp)).background(colors.limelight)) {
                    coil3.compose.AsyncImage(model = p.qrPngPath, contentDescription = "Pairing code", modifier = Modifier.fillMaxSize())
                }
                Spacer(Modifier.height(20.dp))
                Text("Or type this code", style = LoungeTheme.typography.meta, color = colors.dust)
                Spacer(Modifier.height(6.dp))
                Text(p.code.chunked(3).joinToString(" "), style = LoungeTheme.typography.displayL, color = colors.limelight)
                Spacer(Modifier.height(10.dp))
                Text(s.url?.let { "at $it" } ?: "", style = LoungeTheme.typography.caption, color = colors.dust)
            }
            Spacer(Modifier.height(20.dp))
            SheetRow(
                icon = LoungeIcons.Refresh,
                title = "New code",
                onClick = { scope.launch { newCode() } },
            )
        } else {
            Text("Starting…", style = LoungeTheme.typography.meta, color = colors.dust)
        }
        if (s != null) {
            Spacer(Modifier.height(8.dp))
            // Forgetting asks once more first, and moving away takes the question back. One that
            // could not be saved would undo itself at the next restart, so it stays on offer.
            SheetRow(
                icon = LoungeIcons.Phone,
                title = if (forgetting) "Yes, forget every phone" else "Paired phones",
                titleColor = if (forgetting) colors.ember else colors.limelight,
                subtitle = when {
                    forgetFailed -> "Lounge couldn't save that. Try again."
                    forgetting -> "Each phone needs a new code to pair again"
                    forgotten && s.paired == 0u -> "Forgotten. Each phone needs a new code to pair again."
                    s.paired > 0u -> "OK forgets every paired phone"
                    else -> null
                },
                value = if (forgetting) null else if (s.paired == 0u) "None" else s.paired.toString(),
                accent = if (forgetFailed) colors.ember else colors.dust,
                modifier = Modifier.onFocusChanged { if (!it.isFocused) forgetting = false },
                onClick = {
                    when {
                        forgetting -> {
                            forgetting = false
                            scope.launch {
                                forgetFailed = withContext(Dispatchers.IO) { core.companionForgetAll() }.isFailure
                                forgotten = !forgetFailed
                                refresh()
                            }
                        }
                        s.paired > 0u || forgetFailed -> forgetting = true
                    }
                },
            )
        }
    }
    SheetFootnote(
        "Everything stays on your home network.",
        "Add the page to your phone's home screen to open it quickly.",
    )
}

/** The code that pairs a phone at the TV's address now, or null when a new one has to be made. */
internal fun CompanionStatus.livePairing(): PairingInfo? = pairing?.takeIf { p -> url?.let { p.url.startsWith(it) } == true }

/** Why the companion could not start, with what to do; the raw reason goes to the log. */
internal fun companionStartProblem(e: Throwable): String {
    val why = e.reason
    android.util.Log.w("Lounge", "companion: $why")
    return when {
        "in use" in why -> "Another app on this TV is using port ${CoreClient.COMPANION_PORT}, so the phone companion can't start. Restart the TV, then turn it on again."
        else -> "The phone companion couldn't start. Turn it off and on again, or restart the TV."
    }
}

/** Why no pairing code could be made: the core's own sentence when it wrote one, with the raw reason in the log. */
internal fun pairingProblem(e: Throwable): String {
    val why = e.reason
    android.util.Log.w("Lounge", "companion: $why")
    return if (readsAsSentence(why)) why.trim() else "Lounge couldn't make a pairing code. Turn the phone companion off and on again."
}

/**
 * Rows other apps publish to the TV provider; each can be shown on the home screen. [channels] is
 * null until read, and nothing takes focus before then.
 */
@Composable
fun ChannelsSheet(channels: List<ChannelInfo>?, onToggle: (ChannelInfo) -> Unit, firstFocus: FocusRequester) {
    val colors = LoungeTheme.colors
    SheetHeading("Channels from apps", "Turn on the ones you want as rows on your home screen.")
    when {
        channels == null -> Text("Loading…", style = LoungeTheme.typography.meta, color = colors.dust)
        channels.isEmpty() -> Text("No app has published a channel yet.", style = LoungeTheme.typography.meta, color = colors.dust)
        else -> Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            channels.forEachIndexed { i, ch ->
                SheetRow(
                    icon = LoungeIcons.Guide,
                    title = ch.name,
                    subtitle = if (ch.programmes == 1u) "1 programme" else "${ch.programmes} programmes",
                    value = if (ch.enabled) "On" else "Off",
                    accent = if (ch.enabled) colors.signal else colors.dust,
                    onClick = { onToggle(ch) },
                    focusRequester = if (i == 0) firstFocus else null,
                )
            }
        }
    }
    SheetFootnote("Apps decide what goes in their channels; Lounge only chooses which to show.")
    // The list can arrive after the sheet's own request for focus has given up.
    LaunchedEffect(channels != null) { if (!channels.isNullOrEmpty()) firstFocus.requestWhenReady() }
}

@Composable
private fun DiagLine(label: String, value: String) {
    val colors = LoungeTheme.colors
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
        Text(label, style = LoungeTheme.typography.caption, color = colors.dust)
        Text(value, style = LoungeTheme.typography.caption, color = colors.limelight)
    }
}

internal const val HOME_NOT_TAKEN = "Android didn't make Lounge the home screen. Try one of the other ways below."

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
                "Turn on USB debugging on this TV, connect to it with adb from a computer, then run:",
                takeHome.joinToString("\n") { "adb shell ${it.command}" }.ifEmpty { "adb shell cmd package set-home-activity $packageName/tv.lounge.launcher.MainActivity" },
            )
            Step(
                "Or let Android ask",
                "Some TVs show a chooser the next time you press Home. Pick Lounge and choose Always.",
                null,
            )
        }
        Spacer(Modifier.height(24.dp))
        SheetRow(icon = LoungeIcons.Refresh, title = "Check again", onClick = onRecheck)
        SheetRow(
            icon = LoungeIcons.Settings,
            title = "Open Developer options",
            onClick = { runCatching { context.startActivity(Intent(android.provider.Settings.ACTION_APPLICATION_DEVELOPMENT_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) } },
        )
    } else {
        SheetRow(icon = LoungeIcons.Refresh, title = "Check again", onClick = onRecheck, focusRequester = firstFocus)
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
            else -> "The TV's own home screen stays installed, so casting, the gear button and the TV guide keep working."
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
