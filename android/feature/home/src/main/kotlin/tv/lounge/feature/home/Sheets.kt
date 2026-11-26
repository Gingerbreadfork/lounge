package tv.lounge.feature.home

import android.content.Intent
import android.content.res.Resources
import android.net.Uri
import androidx.annotation.StringRes
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
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
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
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.Stable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.runtime.withFrameNanos
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
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
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
import kotlinx.coroutines.delay
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
import tv.lounge.design.Language
import tv.lounge.design.LoungeTheme
import tv.lounge.design.LocalSwapSlotActive
import tv.lounge.design.R as DesignR
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
 * direction. A sheet that shows its subject behind it goes without the [scrim].
 */
@Composable
fun <K : Any> RightSheet(
    visibleState: MutableTransitionState<Boolean>,
    key: K,
    depth: Int,
    onDismiss: () -> Unit,
    scrim: Boolean = true,
    content: @Composable (firstFocus: FocusRequester, key: K) -> Unit,
) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    val scrimLevel = animateFloatAsState(if (scrim) 1f else 0f, tween(if (motion.reducedMotion) 1 else 200), label = "sheetScrim")
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
            Box(Modifier.fillMaxSize().drawBehind { drawRect(colors.scrim, alpha = scrimLevel.value) })
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
                                // Focus leaves the rows for an instant as it moves between them; the anchor takes it only if it stays gone.
                                if (!it.hasFocus && live && !leaving) {
                                    scope.launch {
                                        withFrameNanos { }
                                        if (!contentHasFocus) anchor.requestWhenReady()
                                    }
                                }
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

/** No limit on a sheet row's second line. */
internal const val WHOLE = Int.MAX_VALUE

/** The width a sheet row leaves its words beside a glyph and with no value: the sheet less its edge, its padding and the row's own. */
internal val SheetRowWordsWidth: Dp = Dimens.sheetWidth - 1.dp - 96.dp - 40.dp - 28.dp - 16.dp

/**
 * A 72 dp sheet row: glyph, title, optional trailing value. Focus is a lifted `curtainEdge` surface
 * with a glow and the tiles' rim, and a rested row paints nothing. An [accent] of `signal`,
 * `houselight` or `ember` colours the value, and `signal` adds a status dot.
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
    /** Left and Right change the value through this, and the focused row shows chevrons. */
    onCycle: ((Int) -> Unit)? = null,
    leading: (@Composable RowScope.() -> Unit)? = null,
    /** A second line under the title, for rows that must stay narrow; the row grows past [height] when large text needs it. */
    subtitle: String? = null,
    /** Holding OK, where a row has a second action; a plain press otherwise. */
    onLongClick: (() -> Unit)? = null,
    /** How many lines [subtitle] may take; [WHOLE] for a failure or a state, which is never cut. */
    subtitleLines: Int = 2,
    /** `ember` for the second step of something that cannot be undone. */
    titleColor: Color = LoungeTheme.colors.limelight,
    /** Lines of the row's own under the title, where a subtitle is not enough; the row grows for them. */
    lines: (@Composable ColumnScope.() -> Unit)? = null,
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
        if (subtitle != null || lines != null) {
            Column(Modifier.weight(1f).padding(vertical = if (lines != null) 12.dp else 4.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(title, style = LoungeTheme.typography.title, color = titleColor, maxLines = 2, overflow = TextOverflow.Ellipsis)
                if (subtitle != null) Text(subtitle, style = LoungeTheme.typography.caption, color = colors.dust, maxLines = subtitleLines, overflow = TextOverflow.Ellipsis)
                if (lines != null) lines()
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
    onGroups: () -> Unit,
    /** "None" or "2 groups". */
    groupsSummary: String,
    /** Where the TV's sound comes out now, as the home already knows it. */
    soundOutput: () -> SoundOutput? = { null },
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
    /** The screensaver the TV starts, as the Screensaver sheet names it, read again on return to Lounge. */
    screensaver: () -> TvScreensaver,
    onSetupAgain: () -> Unit,
    onAbout: () -> Unit,
    onDonate: () -> Unit,
    onUpdates: () -> Unit,
    updateStatus: UpdateWord,
    firstFocus: FocusRequester,
    cameFrom: Overlay? = null,
    backdrop: String = Prefs.STILL_BACKDROP,
    backdrops: List<ShaderStyle> = emptyList(),
    liveBackdrops: Boolean = true,
    onSetBackdrop: (String) -> Unit = {},
    notificationsOn: Boolean = true,
    onNotifications: () -> Unit = {},
    /** The tag of the language chosen over the TV's own, or null while Lounge follows the TV. */
    language: String? = null,
    onLanguage: () -> Unit = {},
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
    val resources = LocalResources.current
    val on = stringResource(DesignR.string.common_on)
    val off = stringResource(DesignR.string.common_off)
    var saver by remember { mutableStateOf(screensaver()) }
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) { saver = screensaver() }
    // Rows that hand over to the TV's own settings, on a TV with a screen for them.
    val context = LocalContext.current
    val tvSettings = remember { tvSettingsIntent(context) }
    val networkSettings = remember { networkSettingsIntent(context) }
    val soundSettings = remember { soundSettingsIntent(context) }
    var accessories by remember { mutableStateOf(accessoryScreen(context)) }
    // What each paired device says of its battery, read again on coming back and once Nearby devices is allowed.
    var batteriesRead by remember { mutableIntStateOf(0) }
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) { batteriesRead++ }
    val batteries by produceState(emptyList<AccessoryBattery>(), batteriesRead) { value = withContext(Dispatchers.IO) { accessoryBatteries(context) } }
    val accessoryScope = rememberCoroutineScope()
    var leavingForSettings by remember { mutableStateOf<kotlinx.coroutines.Job?>(null) }
    // Says what to look for and leaves it on screen a moment, since the TV's settings cover it.
    fun findInTvSettings(settings: Intent?) {
        if (settings == null || leavingForSettings != null) return
        say(resources.getString(R.string.sheets_accessories_in_settings))
        leavingForSettings = accessoryScope.launch {
            delay(2_000)
            leavingForSettings = null
            runCatching { context.startActivity(settings) }
        }
    }
    // Another screen opened in the meantime takes its place.
    LifecycleEventEffect(Lifecycle.Event.ON_PAUSE) {
        leavingForSettings?.cancel()
        leavingForSettings = null
    }
    // Asked on the row's press where the TV wants it. Refused, the TV's settings open instead.
    val askNearby = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        val now = accessoryScreen(context)
        accessories = now
        batteriesRead++
        val screen = now?.open.takeIf { granted }
        if (screen != null) runCatching { context.startActivity(screen) } else findInTvSettings(now?.settings)
    }
    val link by rememberLink()
    // Back from one of its pages puts focus on the row that opened it.
    val pages = listOf(Overlay.Rows, Overlay.AppGroups(), Overlay.Channels, Overlay.Plugins, Overlay.NotificationSettings, Overlay.Companion, Overlay.Ambient, Overlay.Language, Overlay.HomeRole, Overlay.PowerSetup, Overlay.Updates, Overlay.Onboarding, Overlay.About, Overlay.Donate)
    val returnTo = cameFrom?.takeIf { it in pages }
    fun focusFor(page: Overlay?) = firstFocus.takeIf { page == returnTo }

    SheetHeading(stringResource(DesignR.string.common_settings))
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        SheetGroup(stringResource(R.string.sheets_group_home))
        SheetRow(
            icon = LoungeIcons.Rows,
            title = stringResource(R.string.sheets_rows_title),
            subtitle = stringResource(R.string.sheets_rows_subtitle),
            value = rowsSummary,
            onClick = onRows,
            focusRequester = focusFor(Overlay.Rows) ?: focusFor(null),
        )
        SheetRow(
            icon = LoungeIcons.Apps,
            title = stringResource(R.string.groups_title),
            subtitle = stringResource(R.string.sheets_groups_subtitle),
            value = groupsSummary,
            onClick = onGroups,
            focusRequester = focusFor(Overlay.AppGroups()),
        )
        val steps = listOf(0, 5, 10, 15)
        val at = steps.indexOf(recentApps).let { if (it < 0) 1 else it }
        SheetRow(
            icon = LoungeIcons.Apps,
            title = stringResource(R.string.sheets_your_apps_title),
            subtitle = stringResource(R.string.sheets_your_apps_subtitle),
            value = if (recentApps == 0) stringResource(R.string.sheets_your_apps_pinned_only) else pluralStringResource(R.plurals.sheets_your_apps_recent, recentApps, recentApps),
            onClick = { sounds0?.tick(); onSetRecentApps(steps[(at + 1) % steps.size]) },
            onCycle = { by -> sounds0?.tick(); onSetRecentApps(steps[(at + by).mod(steps.size)]) },
        )
        SheetRow(
            icon = SettingsGlyphs.PhoneApps,
            title = stringResource(R.string.sheets_phone_apps_title),
            subtitle = stringResource(R.string.sheets_phone_apps_subtitle),
            value = if (showMobileApps) on else off,
            accent = if (showMobileApps) colors.signal else colors.dust,
            onClick = { sounds0?.tick(); onSetShowMobileApps(!showMobileApps) },
            onCycle = { sounds0?.tick(); onSetShowMobileApps(!showMobileApps) },
        )
        SheetRow(
            icon = LoungeIcons.Guide,
            title = stringResource(R.string.sheets_channels_title),
            subtitle = stringResource(R.string.sheets_channels_subtitle),
            value = if (enabledChannels == 0) stringResource(R.string.sheets_channels_none) else pluralStringResource(R.plurals.sheets_channels_on, enabledChannels, enabledChannels),
            onClick = onChannels,
            focusRequester = focusFor(Overlay.Channels),
        )
        SheetRow(
            icon = SettingsGlyphs.Plugin,
            title = stringResource(R.string.sheets_plugins_title),
            subtitle = stringResource(R.string.sheets_plugins_subtitle),
            value = if (enabledPlugins == 0) stringResource(R.string.sheets_plugins_none_on) else pluralStringResource(R.plurals.sheets_plugins_on, enabledPlugins, enabledPlugins),
            onClick = onPlugins,
            focusRequester = focusFor(Overlay.Plugins),
        )
        val notificationsAllowed = access.isOn(Access.NowPlaying)
        SheetRow(
            icon = LoungeIcons.Bell,
            title = stringResource(DesignR.string.common_notifications),
            subtitle = stringResource(R.string.sheets_notifications_subtitle),
            value = if (!notificationsAllowed) stringResource(R.string.sheets_not_allowed) else if (notificationsOn) on else off,
            accent = if (notificationsAllowed && notificationsOn) colors.signal else colors.dust,
            onClick = onNotifications,
            focusRequester = focusFor(Overlay.NotificationSettings),
        )
        Spacer(Modifier.height(16.dp))
        Hairline(Modifier.fillMaxWidth())
        SheetGroup(stringResource(R.string.sheets_group_look))
        SheetRow(
            icon = LoungeIcons.Theme,
            title = stringResource(R.string.sheets_theme_title),
            value = stringResource(theme.title),
            onClick = { sounds0?.tick(); onSetTheme(theme.step(1)) },
            onCycle = { by -> sounds0?.tick(); onSetTheme(theme.step(by)) },
        )
        val scenes = listOf(Triple(Prefs.STILL_BACKDROP, stringResource(R.string.sheets_background_still), stringResource(R.string.sheets_background_still_about))) +
            backdrops.map { Triple(it.id, it.nameIn(resources), it.aboutIn(resources)) }
        val sceneAt = scenes.indexOfFirst { it.first == backdrop }.coerceAtLeast(0)
        SheetRow(
            icon = SettingsGlyphs.Scene,
            title = stringResource(R.string.sheets_background_title),
            value = scenes[sceneAt].second,
            subtitle = if (!liveBackdrops && sceneAt > 0) stringResource(R.string.sheets_background_still_for_now) else scenes[sceneAt].third,
            onClick = { sounds0?.tick(); onSetBackdrop(scenes[(sceneAt + 1).mod(scenes.size)].first) },
            onCycle = { by -> sounds0?.tick(); onSetBackdrop(scenes[(sceneAt + by).mod(scenes.size)].first) },
        )
        SheetRow(
            icon = LoungeIcons.Frame,
            title = stringResource(DesignR.string.common_screensaver),
            value = saver.words(),
            accent = if (saver == TvScreensaver.Lounge) colors.signal else colors.dust,
            onClick = onAmbient,
            focusRequester = focusFor(Overlay.Ambient),
        )
        SheetRow(
            icon = LoungeIcons.Volume,
            title = stringResource(R.string.sheets_sounds_title),
            value = if (sounds) on else off,
            accent = if (sounds) colors.signal else colors.dust,
            onClick = { sounds0?.tick(); onSetSounds(!sounds) },
            onCycle = { sounds0?.tick(); onSetSounds(!sounds) },
        )
        SheetRow(
            icon = SettingsGlyphs.Gauge,
            title = stringResource(R.string.sheets_lite_title),
            subtitle = stringResource(
                when (lite) {
                    null -> if (liteNow) R.string.sheets_lite_auto_on else R.string.sheets_lite_auto_off
                    true -> R.string.sheets_lite_on
                    false -> R.string.sheets_lite_off
                },
            ),
            subtitleLines = WHOLE,
            value = when (lite) { null -> stringResource(R.string.sheets_automatic); true -> on; false -> off },
            onClick = { sounds0?.tick(); onSetLite(lite.stepTri(1)) },
            onCycle = { by -> sounds0?.tick(); onSetLite(lite.stepTri(by)) },
        )
        SheetRow(
            icon = LoungeIcons.Move,
            title = stringResource(R.string.sheets_reduce_motion_title),
            value = when (reducedMotion) { null -> stringResource(R.string.sheets_follow_system); true -> on; false -> off },
            onClick = { sounds0?.tick(); onSetReducedMotion(reducedMotion.stepTri(1)) },
            onCycle = { by -> sounds0?.tick(); onSetReducedMotion(reducedMotion.stepTri(by)) },
        )
        Spacer(Modifier.height(16.dp))
        Hairline(Modifier.fillMaxWidth())
        SheetGroup(stringResource(R.string.sheets_group_access))
        Access.entries.forEach { a -> AccessRow(a, access) }
        Spacer(Modifier.height(16.dp))
        Hairline(Modifier.fillMaxWidth())
        SheetGroup(stringResource(R.string.sheets_group_this_tv))
        SheetRow(
            icon = SettingsGlyphs.Language,
            title = stringResource(R.string.sheets_language_title),
            value = Language.shipped.firstOrNull { it.tag == language }?.name ?: stringResource(R.string.sheets_language_same_as_tv),
            onClick = onLanguage,
            focusRequester = focusFor(Overlay.Language),
        )
        SheetRow(
            icon = SettingsGlyphs.Home,
            title = stringResource(R.string.sheets_default_home_title),
            subtitle = stringResource(R.string.sheets_default_home_subtitle),
            value = if (isDefaultHome) "Lounge" else stringResource(R.string.sheets_default_home_set_up),
            accent = if (isDefaultHome) colors.signal else colors.dust,
            onClick = onRecheckHome,
            focusRequester = focusFor(Overlay.HomeRole),
        )
        SheetRow(
            icon = LoungeIcons.Power,
            title = stringResource(DesignR.string.common_power_setup),
            subtitle = stringResource(R.string.sheets_power_subtitle),
            value = stringResource(
                when (powerStatus) {
                    is tv.lounge.power.PowerStatus.Ready -> R.string.sheets_power_connected
                    tv.lounge.power.PowerStatus.WaitingForApproval -> R.string.sheets_power_waiting
                    else -> R.string.sheets_power_not_connected
                },
            ),
            accent = if (powerStatus is tv.lounge.power.PowerStatus.Ready) colors.signal else colors.dust,
            onClick = onPowerSetup,
            focusRequester = focusFor(Overlay.PowerSetup),
        )
        SheetRow(
            icon = LoungeIcons.Phone,
            title = stringResource(DesignR.string.common_phone_companion),
            value = if (companionOn) on else off,
            accent = if (companionOn) colors.signal else colors.dust,
            onClick = onCompanion,
            focusRequester = focusFor(Overlay.Companion),
        )
        SheetRow(
            icon = SettingsGlyphs.Download,
            title = stringResource(R.string.sheets_updates_title),
            value = updateStatus.shown(),
            accent = when (updateStatus.text) {
                R.string.updates_word_up_to_date -> colors.signal
                R.string.updates_word_problem -> colors.ember
                R.string.updates_word_available, R.string.updates_word_ready -> colors.houselight
                else -> colors.dust
            },
            onClick = onUpdates,
            focusRequester = focusFor(Overlay.Updates),
        )
        networkSettings?.let { screen ->
            SheetRow(
                icon = if (link == Link.Ethernet) LoungeIcons.Ethernet else LoungeIcons.Network,
                title = stringResource(R.string.sheets_network_title),
                subtitle = stringResource(R.string.sheets_network_subtitle),
                value = stringResource(
                    when (link) {
                        Link.Wifi -> R.string.sheets_network_wifi
                        Link.Ethernet -> R.string.sheets_network_ethernet
                        Link.Other -> R.string.sheets_network_connected
                        Link.None -> R.string.top_bar_no_connection
                    },
                ),
                accent = if (link == Link.None) colors.ember else colors.signal,
                onClick = { runCatching { context.startActivity(screen) } },
            )
        }
        soundSettings?.let { screen ->
            SheetRow(
                icon = LoungeIcons.Soundbar,
                title = stringResource(R.string.sheets_sound_title),
                subtitle = stringResource(R.string.sheets_sound_subtitle),
                value = soundOutput()?.label(resources),
                onClick = { runCatching { context.startActivity(screen) } },
            )
        }
        accessories?.let { screen ->
            SheetRow(
                icon = LoungeIcons.Bluetooth,
                title = stringResource(R.string.sheets_accessories_title),
                subtitle = stringResource(if (screen.open == null && screen.locked != null) R.string.sheets_accessories_subtitle_ask else R.string.sheets_accessories_subtitle),
                lines = if (batteries.isEmpty()) {
                    null
                } else {
                    {
                        batteries.forEach { battery ->
                            Text(
                                stringResource(R.string.sheets_accessories_battery, battery.name, stringResource(R.string.updates_percent, battery.percent)),
                                style = LoungeTheme.typography.caption,
                                color = if (battery.low) colors.ember else colors.dust,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                    }
                },
                onClick = {
                    when {
                        screen.open != null -> runCatching { context.startActivity(screen.open) }
                        screen.locked != null -> askNearby.launch(NEARBY_DEVICES)
                        else -> findInTvSettings(screen.settings)
                    }
                },
            )
        }
        tvSettings?.let { screen ->
            SheetRow(
                icon = SettingsGlyphs.Cog,
                title = stringResource(R.string.sheets_tv_settings_title),
                subtitle = stringResource(R.string.sheets_tv_settings_subtitle),
                onClick = { runCatching { context.startActivity(screen) } },
            )
        }
        SheetRow(
            icon = LoungeIcons.Info,
            title = stringResource(R.string.sheets_diagnostics_title),
            subtitle = stringResource(R.string.sheets_diagnostics_subtitle),
            value = stringResource(if (showDiagnostics) DesignR.string.common_hide else DesignR.string.common_show),
            onClick = { showDiagnostics = !showDiagnostics },
        )
        if (showDiagnostics) {
            diag?.let { d ->
                Column(Modifier.padding(start = 20.dp, top = 4.dp, bottom = 8.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    DiagLine(stringResource(R.string.sheets_diagnostics_items), d.items.toString())
                    DiagLine(stringResource(R.string.sheets_diagnostics_revision), d.revision.toString())
                    DiagLine(stringResource(R.string.sheets_diagnostics_requests), d.ffiCalls.toString())
                    DiagLine(stringResource(R.string.sheets_diagnostics_rebuild), stringResource(R.string.sheets_diagnostics_ms, (d.lastBuildMicros / 1000u).toString()))
                    DiagLine(stringResource(R.string.sheets_diagnostics_saved), stringResource(R.string.sheets_diagnostics_kb, (d.dbBytes / 1024u).toString()))
                    DiagLine(stringResource(R.string.sheets_diagnostics_waiting), d.queueDepth.toString())
                }
            }
        }
        SheetRow(icon = LoungeIcons.Refresh, title = stringResource(R.string.sheets_setup_again), onClick = onSetupAgain, focusRequester = focusFor(Overlay.Onboarding))
        SheetRow(icon = SettingsGlyphs.Sofa, title = stringResource(R.string.sheets_about_title), subtitle = stringResource(R.string.sheets_about_subtitle), onClick = onAbout, focusRequester = focusFor(Overlay.About))
        SheetRow(icon = SettingsGlyphs.Heart, title = stringResource(R.string.sheets_support_title), subtitle = stringResource(R.string.sheets_support_subtitle), onClick = onDonate, focusRequester = focusFor(Overlay.Donate))
    }
    SheetFootnote(stringResource(R.string.sheets_footnote))
}

/**
 * The languages Lounge is written in, each under its own name, after the choice to follow the
 * TV. The one in use carries the dot, and choosing another changes every screen at once.
 */
@Composable
fun LanguageSheet(chosen: String?, onChoose: (String?) -> Unit, firstFocus: FocusRequester) {
    val colors = LoungeTheme.colors
    val sounds = LoungeTheme.sounds
    val inUse = stringResource(R.string.sheets_language_in_use)
    // The TV's own language under its own name, and whether Lounge is written in it.
    val tv = remember(LocalConfiguration.current) { android.content.res.Resources.getSystem().configuration.locales[0] }
    val tvName = remember(tv) { tv.getDisplayLanguage(tv).replaceFirstChar { it.titlecase(tv) } }
    val shippedForTv = remember(tv) { Language.shipped.any { it.tag.substringBefore('-') == tv.language } }
    SheetHeading(stringResource(R.string.sheets_language_title), stringResource(R.string.sheets_language_subtitle))
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        SheetRow(
            icon = LoungeIcons.Tv,
            title = stringResource(R.string.sheets_language_same_as_tv),
            subtitle = if (shippedForTv) tvName else stringResource(R.string.sheets_language_tv_not_shipped, tvName),
            value = inUse.takeIf { chosen == null },
            accent = colors.signal,
            onClick = { sounds?.tick(); onChoose(null) },
            focusRequester = firstFocus.takeIf { chosen == null },
        )
        Language.shipped.forEach { language ->
            SheetRow(
                icon = null,
                title = language.name,
                value = inUse.takeIf { chosen == language.tag },
                accent = colors.signal,
                onClick = { sounds?.tick(); onChoose(language.tag) },
                focusRequester = firstFocus.takeIf { chosen == language.tag },
            )
        }
    }
    SheetFootnote(stringResource(R.string.sheets_language_footnote))
}

/**
 * One of Lounge's optional accesses, as Settings and the setup steps name it: its name, what it is
 * for, how to allow it on Lounge's page in Android's settings, and what is said when no screen can.
 */
internal enum class Access(val key: String, @StringRes val title: Int, @StringRes val what: Int, @StringRes val allow: Int, @StringRes val noScreen: Int) {
    TvListings("tv_listings", R.string.sheets_access_tv_listings, R.string.sheets_access_tv_listings_what, R.string.sheets_access_tv_listings_allow, R.string.sheets_access_tv_listings_no_screen),
    NowPlaying("notifications", R.string.sheets_access_notifications, R.string.sheets_access_notifications_what, R.string.sheets_access_notifications_allow, R.string.sheets_access_notifications_no_screen),
    Usage("usage", R.string.sheets_access_usage, R.string.sheets_access_usage_what, R.string.sheets_access_usage_allow, R.string.sheets_access_usage_no_screen),
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
    @StringRes
    fun note(a: Access): Int {
        val screen = a.screen()
        return when {
            isOn(a) -> a.what
            refused[a] == true -> R.string.sheets_access_wont_ask
            screen != null && !opens(Intent(screen)) -> R.string.sheets_access_no_screen
            else -> a.what
        }
    }

    fun ask(a: Access) {
        if (isOn(a) || leaving) return
        val screen = a.screen()
        when {
            screen != null -> leaveFor(Intent(screen), R.string.sheets_access_hint_find, a)
            refused[a] == true -> leaveFor(
                Intent(android.provider.Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null)),
                a.allow,
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
    private fun leaveFor(intent: Intent, @StringRes hint: Int, a: Access) {
        intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        if (!opens(intent)) {
            say(context.getString(a.noScreen))
            onPowerSetup()
            return
        }
        leaving = true
        say(context.getString(hint))
        scope.launch {
            kotlinx.coroutines.delay(1_500)
            leaving = false
            runCatching { context.startActivity(intent) }.onFailure {
                say(context.getString(a.noScreen))
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
        title = stringResource(access.title),
        subtitle = stringResource(asker.note(access)),
        subtitleLines = WHOLE,
        value = stringResource(if (on) DesignR.string.common_on else R.string.sheets_not_allowed),
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
    val resources = LocalResources.current
    val scope = rememberCoroutineScope()
    var status by remember { mutableStateOf<CompanionStatus?>(null) }
    var pairing by remember { mutableStateOf<PairingInfo?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var forgetFailed by remember { mutableStateOf(false) }
    var forgetting by remember { mutableStateOf(false) }
    var forgotten by remember { mutableStateOf(false) }
    // Starting binds a port and a new code writes its picture, so the server's calls run off the main thread.
    suspend fun newCode() {
        withContext(Dispatchers.IO) { core.companionPairing() }.onSuccess { pairing = it; error = null }.onFailure { pairing = null; error = pairingProblem(resources, it) }
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
                ?.onFailure { error = companionStartProblem(resources, it) }
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

    SheetHeading(stringResource(DesignR.string.common_phone_companion), stringResource(R.string.sheets_companion_subtitle))
    val running = enabled && status?.running == true
    SheetRow(
        icon = LoungeIcons.Phone,
        title = stringResource(DesignR.string.common_phone_companion),
        subtitle = if (enabled) null else stringResource(R.string.sheets_companion_turn_on),
        value = stringResource(if (enabled) DesignR.string.common_on else DesignR.string.common_off),
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
                    coil3.compose.AsyncImage(model = p.qrPngPath, contentDescription = stringResource(R.string.sheets_companion_code_description), modifier = Modifier.fillMaxSize())
                }
                Spacer(Modifier.height(20.dp))
                Text(stringResource(R.string.sheets_companion_or_type), style = LoungeTheme.typography.meta, color = colors.dust)
                Spacer(Modifier.height(6.dp))
                Text(p.code.chunked(3).joinToString(" "), style = LoungeTheme.typography.displayL, color = colors.limelight)
                Spacer(Modifier.height(10.dp))
                Text(s.url?.let { stringResource(R.string.sheets_companion_at, it) } ?: "", style = LoungeTheme.typography.caption, color = colors.dust)
            }
            Spacer(Modifier.height(20.dp))
            SheetRow(
                icon = LoungeIcons.Refresh,
                title = stringResource(R.string.sheets_companion_new_code),
                onClick = { scope.launch { newCode() } },
            )
        } else {
            Text(stringResource(R.string.sheets_companion_starting), style = LoungeTheme.typography.meta, color = colors.dust)
        }
        if (s != null) {
            Spacer(Modifier.height(8.dp))
            // Forgetting asks once more first, and moving away takes the question back. One that
            // could not be saved would undo itself at the next restart, so it stays on offer.
            SheetRow(
                icon = LoungeIcons.Phone,
                title = stringResource(if (forgetting) R.string.sheets_companion_forget_confirm else R.string.sheets_companion_paired),
                titleColor = if (forgetting) colors.ember else colors.limelight,
                subtitle = when {
                    forgetFailed -> stringResource(R.string.sheets_companion_forget_failed)
                    forgetting -> stringResource(R.string.sheets_companion_forget_warning)
                    forgotten && s.paired == 0u -> stringResource(R.string.sheets_companion_forgotten)
                    s.paired > 0u -> stringResource(R.string.sheets_companion_forget_hint)
                    else -> null
                },
                subtitleLines = WHOLE,
                value = if (forgetting) null else if (s.paired == 0u) stringResource(R.string.sheets_companion_paired_none) else s.paired.toString(),
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
        stringResource(R.string.sheets_companion_footnote_network),
        stringResource(R.string.sheets_companion_footnote_home_screen),
    )
}

/** The live code when it was made for the TV's present address, or null when a new one has to be made. */
internal fun CompanionStatus.livePairing(): PairingInfo? = pairing?.takeIf { p -> url?.let { p.url.startsWith(it) } == true }

/** Why the companion could not start, with what to do; the raw reason goes to the log. */
internal fun companionStartProblem(resources: Resources, e: Throwable): String {
    val why = e.reason
    android.util.Log.w("Lounge", "companion: $why")
    return when {
        "in use" in why -> resources.getString(R.string.sheets_companion_port_in_use, CoreClient.COMPANION_PORT.toString())
        else -> resources.getString(R.string.sheets_companion_cant_start)
    }
}

/** Why no pairing code could be made: the core's own sentence when it wrote one, with the raw reason in the log. */
internal fun pairingProblem(resources: Resources, e: Throwable): String {
    val why = e.reason
    android.util.Log.w("Lounge", "companion: $why")
    return if (readsAsSentence(why)) why.trim() else resources.getString(R.string.sheets_companion_no_code)
}

/**
 * Rows other apps publish to the TV provider; each can be shown on the home screen. [channels] is
 * null until read, and nothing takes focus before then.
 */
@Composable
fun ChannelsSheet(channels: List<ChannelInfo>?, onToggle: (ChannelInfo) -> Unit, firstFocus: FocusRequester) {
    val colors = LoungeTheme.colors
    SheetHeading(stringResource(R.string.sheets_channels_title), stringResource(R.string.sheets_channels_heading_subtitle))
    when {
        channels == null -> Text(stringResource(R.string.sheets_loading), style = LoungeTheme.typography.meta, color = colors.dust)
        channels.isEmpty() -> Text(stringResource(R.string.sheets_channels_empty), style = LoungeTheme.typography.meta, color = colors.dust)
        else -> Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            channels.forEachIndexed { i, ch ->
                SheetRow(
                    icon = LoungeIcons.Guide,
                    title = ch.name,
                    subtitle = pluralStringResource(R.plurals.sheets_channels_programmes, ch.programmes.toInt(), ch.programmes.toInt()),
                    value = stringResource(if (ch.enabled) DesignR.string.common_on else DesignR.string.common_off),
                    accent = if (ch.enabled) colors.signal else colors.dust,
                    onClick = { onToggle(ch) },
                    focusRequester = if (i == 0) firstFocus else null,
                )
            }
        }
    }
    SheetFootnote(stringResource(R.string.sheets_channels_footnote))
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

/** Said when Do it now ran and Lounge is still not the home screen. */
@StringRes
internal val HOME_NOT_TAKEN: Int = R.string.sheets_home_not_taken

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
    SheetHeading(
        stringResource(if (isDefaultHome) R.string.sheets_home_is_lounge else R.string.sheets_home_make_lounge),
        stringResource(if (isDefaultHome) R.string.sheets_home_is_lounge_subtitle else R.string.sheets_home_make_subtitle),
    )
    if (!isDefaultHome) {
        SheetRow(
            icon = LoungeIcons.Power,
            title = stringResource(if (powerReady) R.string.sheets_home_do_it_now else R.string.sheets_home_let_lounge),
            value = stringResource(if (powerReady) R.string.sheets_home_through_power_setup else DesignR.string.common_power_setup),
            accent = colors.houselight,
            onClick = if (powerReady) onDoItNow else onPowerSetup,
            focusRequester = firstFocus,
        )
        if (turnOff.isNotEmpty()) {
            Spacer(Modifier.height(12.dp))
            Step(
                pluralStringResource(R.plurals.sheets_home_own_first_title, turnOff.size),
                pluralStringResource(R.plurals.sheets_home_own_first_body, turnOff.size),
                takeHome.joinToString("\n") { it.command },
            )
        }
        Spacer(Modifier.height(20.dp))
        Column(verticalArrangement = Arrangement.spacedBy(20.dp)) {
            Step(
                stringResource(R.string.sheets_home_from_computer_title),
                stringResource(R.string.sheets_home_from_computer_body),
                takeHome.joinToString("\n") { "adb shell ${it.command}" }.ifEmpty { "adb shell cmd package set-home-activity $packageName/tv.lounge.launcher.MainActivity" },
            )
            Step(
                stringResource(R.string.sheets_home_let_android_title),
                stringResource(R.string.sheets_home_let_android_body),
                null,
            )
        }
        Spacer(Modifier.height(24.dp))
        SheetRow(icon = LoungeIcons.Refresh, title = stringResource(R.string.sheets_home_check_again), onClick = onRecheck)
        SheetRow(
            icon = LoungeIcons.Settings,
            title = stringResource(R.string.sheets_home_open_developer_options),
            onClick = { runCatching { context.startActivity(Intent(android.provider.Settings.ACTION_APPLICATION_DEVELOPMENT_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) } },
        )
    } else {
        SheetRow(icon = LoungeIcons.Refresh, title = stringResource(R.string.sheets_home_check_again), onClick = onRecheck, focusRequester = firstFocus)
        if (homesOff.isNotEmpty()) {
            SheetRow(icon = LoungeIcons.Power, title = pluralStringResource(R.plurals.sheets_home_turn_back_on, homesOff.size), value = stringResource(DesignR.string.common_power_setup), onClick = onPowerSetup)
        }
    }
    SheetFootnote(
        when {
            homesOff.isNotEmpty() -> {
                val names = homesOff.reduce { listed, name -> stringResource(R.string.sheets_home_names_and, listed, name) }
                pluralStringResource(R.plurals.sheets_home_footnote_off, homesOff.size, names)
            }
            turnOff.isNotEmpty() -> pluralStringResource(R.plurals.sheets_home_footnote_missing, turnOff.size)
            else -> stringResource(R.string.sheets_home_footnote_stays)
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
