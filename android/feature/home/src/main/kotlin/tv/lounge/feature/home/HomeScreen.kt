package tv.lounge.feature.home

import android.content.Intent
import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.animate
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.focusGroup
import androidx.compose.foundation.gestures.LocalBringIntoViewSpec
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.isSpecified
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import tv.lounge.core.CoreClient
import tv.lounge.core.HomeSnapshot
import tv.lounge.core.LayoutCommand
import tv.lounge.core.StageSpotlight
import tv.lounge.core.Tile
import tv.lounge.design.FieldSleep
import tv.lounge.design.LightField
import tv.lounge.design.LocalLightState
import tv.lounge.design.LoungeTheme
import tv.lounge.design.LoungeVisuals
import tv.lounge.design.LoungeToastHost
import tv.lounge.design.Presence
import tv.lounge.design.ToastController
import tv.lounge.design.sounds
import tv.lounge.platform.AndroidPlatform
import tv.lounge.platform.Prefs
import tv.lounge.platform.UiCommand

private const val IDLE_AFTER_MS = 20_000L

/** Set once per process: the lamp warms up on a cold start only, not on every return home. */
private var lampLit = false

/**
 * The home screen: top bar, Stage, rows, and the overlays, over the light field.
 * One screen, one job (DESIGN.md §2).
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun HomeScreen(core: CoreClient, platform: AndroidPlatform, prefs: Prefs, uiPrefs: Prefs.UiPrefs, power: tv.lounge.power.PowerSetup) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val powerStatus by power.status.collectAsStateWithLifecycle()
    val snapshot by core.snapshots.collectAsStateWithLifecycle()
    val tierA = LoungeTheme.visuals === LoungeVisuals.TierA
    val state = remember { HomeState(tierA) }
    val toasts = remember { ToastController() }
    val light = LocalLightState.current
    val motion = LoungeTheme.motion
    val theme = uiPrefs.theme
    val sounds = uiPrefs.sounds
    val lite = uiPrefs.liteMode
    val reduced = uiPrefs.reducedMotion
    val showMobile = uiPrefs.showMobileApps
    var isDefaultHome by remember { mutableStateOf(isDefaultHome(context)) }
    val uiSounds = LoungeTheme.sounds
    LaunchedEffect(sounds) { uiSounds?.enabled = sounds }

    var composedOnce by remember { mutableStateOf(false) }
    SideEffect {
        if (!composedOnce) {
            composedOnce = true
            android.util.Log.i("Lounge", "startup: home composed at +${android.os.SystemClock.uptimeMillis() - startupReference()} ms after activity start")
        }
    }

    // The lamp warms up: content is there from the first frame, only the light ramps in.
    LaunchedEffect(Unit) {
        if (lampLit || motion.reducedMotion) {
            light.warmth = 1f
            return@LaunchedEffect
        }
        lampLit = true
        light.warmth = 0f
        animate(0f, 1f, animationSpec = motion.lamp()) { v, _ -> light.warmth = v }
    }
    LaunchedEffect(Unit) {
        snapshotFlow { state.lastInputAt }.collectLatest {
            delay(IDLE_AFTER_MS)
            state.idle = true
        }
    }
    // First run: the persisted snapshot paints first, then the setup steps come up over it.
    LaunchedEffect(uiPrefs.loaded, uiPrefs.onboardingDone) {
        if (uiPrefs.loaded && !uiPrefs.onboardingDone && state.overlay == Overlay.None) {
            delay(900)
            state.overlay = Overlay.Onboarding
        }
    }
    LaunchedEffect(state.idle) { light.idle = state.idle }
    val overlay = state.overlay
    val opaqueOverlay = overlay == Overlay.Search || overlay == Overlay.Onboarding || overlay == Overlay.AmbientPreview
    FieldSleep(light, { state.lastInputAt }, opaqueOverlay)
    // The rows the lowered Stage reveals are composed in idle frames while it is still up, so
    // the proscenium never composes a row in the middle of a frame.
    val listState = rememberLazyListState(prefetchStrategy = remember(tierA) { RowsAheadPrefetch(rowsAhead = if (tierA) 3 else 2, tilesPerRow = 8) })
    // A row that arrives at the top while the Stage is up (an update notice) must show at the
    // top: the list would otherwise keep its first row in place and hide the new one above it.
    val firstRowId = snapshot.rows.firstOrNull()?.id
    LaunchedEffect(firstRowId) {
        if (firstRowId != null && state.region != FocusRegion.Rows && listState.firstVisibleItemIndex <= 1) listState.scrollToItem(0)
    }

    fun say(text: String) = toasts.show(text)

    /** Core actions run off the main thread: a plugin may take a moment to answer. */
    fun perform(action: String, title: String) {
        scope.launch {
            withContext(Dispatchers.IO) { core.perform(action) }.onFailure { e -> say(friendlyError(e, title)) }
        }
    }

    fun perform(tile: Tile) {
        uiSounds?.open()
        perform(tile.action, tile.title)
    }

    fun close() {
        uiSounds?.close()
    }

    /** Home means the Stage: expand it, bring the rows back to the top and move focus there. */
    fun goToStage(snap: Boolean = false) {
        state.region = FocusRegion.Stage
        state.focusedRowId = null
        scope.launch { state.focusStrip() }
        if (listState.firstVisibleItemIndex > 0 || listState.firstVisibleItemScrollOffset > 0) {
            scope.launch { if (snap) listState.scrollToItem(0) else listState.animateScrollToItem(0) }
        }
    }

    LaunchedEffect(Unit) {
        platform.commands.collect { cmd ->
            when (cmd) {
                is UiCommand.Toast -> say(cmd.message)
                UiCommand.OpenTvControls -> state.overlay = Overlay.TvControls
                is UiCommand.RequestPermission, is UiCommand.Key, is UiCommand.TypeText -> Unit
                is UiCommand.Search -> { state.pendingSearch = cmd.query; state.overlay = Overlay.Search }
                // Home from another app: the rows are already home when the window appears.
                UiCommand.GoHome -> { state.overlay = Overlay.None; goToStage(snap = true) }
            }
        }
    }

    fun openMenu(tile: Tile, rowId: String, anchor: Rect, returnFocus: FocusRequester) {
        val resolved = if (anchor != Rect.Zero) anchor else if (light.center.isSpecified) {
            Rect(light.center.x - 100f, light.center.y - 60f, light.center.x + 100f, light.center.y + 60f)
        } else {
            Rect.Zero
        }
        state.overlay = Overlay.TileMenu(tile, rowId, resolved, returnFocus)
    }

    val hasStrip = snapshot.stage.strip.isNotEmpty()
    val nowPlaying = snapshot.nowPlaying
    LaunchedEffect(nowPlaying == null) {
        if (nowPlaying == null && state.overlay == Overlay.NowPlaying) state.overlay = Overlay.None
    }
    val menu = overlay as? Overlay.TileMenu
    // While an anchored menu is open the room dims around its tile (the rows dim their tiles).
    val roomAlpha = animateFloatAsState(if (menu != null) 0.5f else 1f, motion.settle(), label = "roomAlpha")

    Box(
        Modifier
            .fillMaxSize()
            .focusProperties {
                onEnter = {
                    if (hasStrip && state.overlay == Overlay.None && state.region == FocusRegion.Stage) {
                        runCatching { state.stripFocus().requestFocus() }
                    }
                }
            }
            .focusGroup()
            .onPreviewKeyEvent { e ->
                if (e.type == KeyEventType.KeyDown) {
                    state.touched()
                    uiSounds?.repeating = e.nativeKeyEvent.repeatCount > 0
                } else if (e.type == KeyEventType.KeyUp) {
                    uiSounds?.repeating = false
                }
                if (e.isBack() && state.overlay == Overlay.None) {
                    if (e.type == KeyEventType.KeyUp) {
                        when (state.region) {
                            FocusRegion.Rows, FocusRegion.TopBar -> {
                                close()
                                goToStage()
                            }
                            FocusRegion.Stage -> if (state.playbillFocused) scope.launch { state.focusStrip() }
                        }
                    }
                    true
                } else {
                    false
                }
            },
    ) {
        LightField(light)

        Column(Modifier.fillMaxSize()) {
            TopBar(
                showTvControls = power.hasTvControls,
                showGuide = snapshot.rows.any { it.id == "live" },
                nowPlaying = nowPlaying,
                idle = state.idle,
                firstFocus = state.topBarFocus,
                downFocus = { state.stripFocus() },
                onFocused = { state.region = FocusRegion.TopBar; light.clear(); uiSounds?.tick() },
                onSearch = { uiSounds?.open(); state.overlay = Overlay.Search },
                onTvControls = { uiSounds?.open(); state.overlay = Overlay.TvControls },
                onGuide = { uiSounds?.open(); state.overlay = Overlay.Guide },
                onNowPlaying = { uiSounds?.open(); state.overlay = Overlay.NowPlaying },
                onSettings = { uiSounds?.open(); state.overlay = Overlay.QuickSettings },
                modifier = Modifier.graphicsLayer { alpha = roomAlpha.value },
            )
            Spacer(Modifier.height(34.dp))
            Box(Modifier.fillMaxSize()) {
                // The rows keep one layout whatever the Stage does: a layer translation moves them
                // and a clip hides what sits under the lowered Stage, so the proscenium never lays
                // them out.
                val density = androidx.compose.ui.platform.LocalDensity.current
                val headerPx = with(density) { tv.lounge.design.Dimens.rowHeaderHeight.toPx() }
                val collapsedPx = with(density) { StageCollapsedHeight.toPx() }
                val belowStage = remember(collapsedPx) { TopInsetShape(collapsedPx) }
                CompositionLocalProvider(LocalBringIntoViewSpec provides pivotSpec(collapsedPx + headerPx)) {
                    LazyColumn(
                        state = listState,
                        // Room for the lowered Stage above the first row, and enough below the last row for it to reach the focus line.
                        contentPadding = PaddingValues(top = StageCollapsedHeight, bottom = 760.dp),
                        modifier = Modifier
                            .fillMaxSize()
                            .graphicsLayer {
                                translationY = (state.stageHeight?.value?.toPx() ?: 0f) - collapsedPx
                                shape = belowStage
                                clip = true
                            }
                            .onFocusChanged { if (it.hasFocus) state.region = FocusRegion.Rows }
                            .focusGroup(),
                    ) {
                    items(snapshot.rows, key = { it.id }) { row ->
                        ContentRow(
                            row = row,
                            isFocusedRow = { state.focusedRowId == row.id },
                            idle = state.idle,
                            listState = state.rowListState(row.id),
                            requesterFor = { index -> state.rowRequester(row.id, index) },
                            onTileFocused = { _, index, _ ->
                                if (state.focusedRowId != row.id) uiSounds?.tick(rowChange = true) else if (state.focusedTileIndex != index) uiSounds?.tick()
                                state.focusedRowId = row.id
                                state.focusedTileIndex = index
                            },
                            onTileClick = { perform(it) },
                            onTileLongPress = { tile, bounds, fr -> openMenu(tile, row.id, bounds, fr) },
                            heldTileId = menu?.tile?.id,
                        )
                    }
                }
                }
                // Not a focus group: the box overlaps the rows, so as a group it would never count
                // as above a focused row. The band and the strip are the candidates instead.
                Box(
                    Modifier
                        .graphicsLayer { alpha = roomAlpha.value }
                        .onFocusChanged { if (it.hasFocus) state.region = FocusRegion.Stage }
                        .onPreviewKeyEvent { e ->
                            // Down from the strip enters the first row at its leftmost tile, like the
                            // system launcher, instead of whichever tile happens to sit underneath.
                            val firstRow = snapshot.rows.firstOrNull()
                            if (e.key == Key.DirectionDown && e.type == KeyEventType.KeyDown && !state.playbillFocused && firstRow != null) {
                                scope.launch { state.focusRow(firstRow.id) }
                                true
                            } else {
                                false
                            }
                        },
                ) {
                    StageSlot(
                        snapshot = snapshot,
                        state = state,
                        onPerform = { perform(it) },
                        onMenu = { tile, bounds, fr -> openMenu(tile, "stage", bounds, fr) },
                        onPrimary = { sp -> uiSounds?.open(); perform(sp.primary.action, sp.title) },
                        onSecondary = { sp -> uiSounds?.open(); sp.secondary?.let { s -> perform(s.action, sp.title) } },
                        onBandFocused = { uiSounds?.tick(); goToStage() },
                    )
                }
            }
        }

        // Overlays stay composed through their exit so each can animate out.
        Presence(menu) { ov, visibleState ->
            val pkg = ov.tile.id.removePrefix("app:")
            TileMenuPopover(
                tile = ov.tile,
                anchor = ov.anchor,
                items = menuItemsFor(
                    tile = ov.tile,
                    rowId = ov.rowId,
                    isPinned = ov.tile.pinned,
                    isStagePinned = ov.tile.stagePinned,
                    onHide = { core.command(LayoutCommand.HideItem(ov.tile.id)); say("Hidden from ${rowTitle(snapshot, ov.rowId)}") },
                    onPin = { core.command(LayoutCommand.PinApp(pkg)); say("Pinned to Your apps") },
                    onUnpin = { core.command(LayoutCommand.UnpinApp(pkg)) },
                    onMoveLeft = { core.command(LayoutCommand.MoveApp(pkg, (state.focusedTileIndex - 1).coerceAtLeast(0).toUInt())) },
                    onMoveRight = { core.command(LayoutCommand.MoveApp(pkg, (state.focusedTileIndex + 1).toUInt())) },
                    onPinStage = { core.command(LayoutCommand.PinToStage(ov.tile.id)); say("Pinned to the Stage") },
                    onUnpinStage = { core.command(LayoutCommand.UnpinFromStage(ov.tile.id)) },
                    onAppInfo = { perform("info:${ov.tile.id}", ov.tile.title) },
                    onUninstall = { requestUninstall(context, pkg) },
                    onPlay = { openPlayListing(context, pkg) },
                    onDismissNotice = { core.command(LayoutCommand.DismissNotice(ov.tile.id)) },
                    onGuide = { state.overlay = Overlay.Guide },
                ),
                visibleState = visibleState,
                onDismiss = {
                    if (state.overlay == ov) {
                        close()
                        state.overlay = Overlay.None
                        scope.launch { ov.returnFocus.requestWhenReady() }
                    }
                },
            )
        }

        Presence(overlay.takeIf { it.isSheet() }) { ov, visibleState ->
            RightSheet(
                visibleState = visibleState,
                key = ov,
                depth = ov.sheetDepth(),
                onDismiss = {
                    close()
                    when (ov) {
                        Overlay.QuickSettings, Overlay.TvControls -> {
                            state.overlay = Overlay.None
                            scope.launch { state.topBarFocus.requestWhenReady() }
                        }
                        is Overlay.PluginDetail -> state.overlay = Overlay.Plugins
                        else -> state.overlay = Overlay.QuickSettings
                    }
                },
            ) { first ->
                SheetContent(
                    overlay = ov,
                    first = first,
                    core = core,
                    platform = platform,
                    prefs = prefs,
                    uiPrefs = uiPrefs,
                    power = power,
                    powerStatus = powerStatus,
                    theme = theme,
                    sounds = sounds,
                    lite = lite,
                    reduced = reduced,
                    showMobile = showMobile,
                    isDefaultHome = isDefaultHome,
                    onHomeChanged = { isDefaultHome = isDefaultHome(context) },
                    say = ::say,
                    open = { next -> uiSounds?.open(); state.overlay = next },
                    closeAll = { close(); state.overlay = Overlay.None },
                )
            }
        }

        Presence(if (overlay == Overlay.Guide) Unit else null) { _, visibleState ->
            GuideSheet(
                core = core,
                visibleState = visibleState,
                onTune = { action -> uiSounds?.open(); perform(action, "that channel") },
                onDismiss = { close(); state.overlay = Overlay.None; scope.launch { state.topBarFocus.requestWhenReady() } },
            )
        }

        Presence(nowPlaying.takeIf { overlay == Overlay.NowPlaying }) { np, visibleState ->
            NowPlayingCard(
                np = np,
                visibleState = visibleState,
                onCommand = { cmd -> perform("media:${np.`package`}:$cmd", np.title) },
                onOpenApp = { uiSounds?.open(); runCatching { platform.launchApp(np.`package`, "") }.onFailure { say(friendlyError(it, np.appLabel)) } },
                onDismiss = { close(); state.overlay = Overlay.None; scope.launch { state.topBarFocus.requestWhenReady() } },
            )
        }

        Presence(if (overlay == Overlay.Search) Unit else null) { _, visibleState ->
            SearchScreen(
                snapshot = snapshot,
                idle = state.idle,
                visibleState = visibleState,
                initialQuery = state.pendingSearch,
                onTileFocused = { _, _ -> },
                onTileClick = { perform(it) },
                onTileLongPress = { tile, rowId, bounds, fr -> openMenu(tile, rowId, bounds, fr) },
                onClose = { close(); state.pendingSearch = ""; state.overlay = Overlay.None; scope.launch { state.topBarFocus.requestWhenReady() } },
            )
        }

        Presence(if (overlay == Overlay.AmbientPreview) Unit else null) { _, visibleState ->
            AnimatedVisibility(visibleState, enter = fadeIn(motion.curtain()), exit = fadeOut(motion.settle())) {
                AmbientPreview(core = core, onClose = { close(); state.overlay = Overlay.Ambient })
            }
        }

        Presence(if (overlay == Overlay.Onboarding) Unit else null) { _, visibleState ->
            OnboardingScreen(
                core = core,
                prefs = prefs,
                uiPrefs = uiPrefs,
                power = power,
                visibleState = visibleState,
                onDone = {
                    close()
                    scope.launch { prefs.setOnboardingDone(true) }
                    isDefaultHome = isDefaultHome(context)
                    state.overlay = Overlay.None
                    goToStage()
                },
            )
        }

        LoungeToastHost(toasts, Modifier.align(androidx.compose.ui.Alignment.BottomStart))
    }

    LaunchedEffect(hasStrip) {
        if (hasStrip && state.region == FocusRegion.Stage && state.overlay == Overlay.None) state.focusStrip()
    }

    // Any Back the UI didn't consume still means "home": never let it reach the Activity.
    BackHandler(enabled = true) {
        if (state.overlay == Overlay.None) goToStage() else { close(); state.overlay = Overlay.None }
    }
}

/** The activity's start time, set by the app module. */
var startupReference: () -> Long = { 0L }

/** Everything below [top]: the part of the rows' layer the lowered Stage does not cover. */
private data class TopInsetShape(val top: Float) : androidx.compose.ui.graphics.Shape {
    override fun createOutline(size: androidx.compose.ui.geometry.Size, layoutDirection: androidx.compose.ui.unit.LayoutDirection, density: androidx.compose.ui.unit.Density) =
        androidx.compose.ui.graphics.Outline.Rectangle(androidx.compose.ui.geometry.Rect(0f, top, size.width, size.height))
}

private fun Overlay.isSheet(): Boolean = when (this) {
    Overlay.QuickSettings, Overlay.Ambient, Overlay.PowerSetup, Overlay.TvControls, Overlay.Channels,
    Overlay.Plugins, is Overlay.PluginDetail, Overlay.HomeRole, Overlay.Companion,
    -> true
    else -> false
}

/** How deep a sheet sits, for the direction of the content shift between sheets. */
private fun Overlay.sheetDepth(): Int = when (this) {
    Overlay.QuickSettings, Overlay.TvControls -> 0
    is Overlay.PluginDetail, Overlay.PowerSetup -> 2
    else -> 1
}

private fun rowTitle(snapshot: HomeSnapshot, rowId: String): String =
    snapshot.rows.firstOrNull { it.id == rowId }?.title ?: if (rowId == "stage") "the Stage" else "the home screen"

/** Errors as sentences a viewer can act on, never a stack trace or an intent dump (DESIGN.md §8). */
internal fun friendlyError(e: Throwable, title: String): String {
    val m = e.message.orEmpty()
    return when {
        "No Activity found" in m || "ActivityNotFound" in m -> "Nothing on this TV can open $title. Is its app still installed?"
        "not installed" in m -> "$title isn't installed on this TV."
        "did not answer" in m -> "$title's source didn't answer. Check that it is running, then try again."
        "permission" in m.lowercase() -> "Lounge needs permission for that. Settings has the switch."
        m.isBlank() || m.length > 120 -> "Couldn't open $title."
        else -> m
    }
}

internal fun hasTvListings(context: android.content.Context): Boolean =
    androidx.core.content.ContextCompat.checkSelfPermission(context, "android.permission.READ_TV_LISTINGS") ==
        android.content.pm.PackageManager.PERMISSION_GRANTED

internal fun hasNotificationAccess(context: android.content.Context): Boolean =
    androidx.core.app.NotificationManagerCompat.getEnabledListenerPackages(context).contains(context.packageName)

internal fun isDefaultHome(context: android.content.Context): Boolean {
    val intent = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_HOME)
    val resolved = context.packageManager.resolveActivity(intent, android.content.pm.PackageManager.MATCH_DEFAULT_ONLY)
    return resolved?.activityInfo?.packageName == context.packageName
}

/** What each right sheet shows; one function so the panel can crossfade between them. */
@Composable
private fun SheetContent(
    overlay: Overlay,
    first: FocusRequester,
    core: CoreClient,
    platform: AndroidPlatform,
    prefs: Prefs,
    uiPrefs: Prefs.UiPrefs,
    power: tv.lounge.power.PowerSetup,
    powerStatus: tv.lounge.power.PowerStatus,
    theme: Prefs.Theme,
    sounds: Boolean,
    lite: Boolean?,
    reduced: Boolean?,
    showMobile: Boolean,
    isDefaultHome: Boolean,
    onHomeChanged: () -> Unit,
    say: (String) -> Unit,
    open: (Overlay) -> Unit,
    closeAll: () -> Unit,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    when (overlay) {
        Overlay.QuickSettings -> {
            val tvListings = remember { hasTvListings(context) }
            val notifications = remember { hasNotificationAccess(context) }
            val enabledChannels = remember { core.availableChannels().count { it.enabled } }
            val enabledPlugins = remember { core.plugins().count { it.enabled } }
            QuickSettingsSheet(
                prefs = prefs,
                theme = theme,
                sounds = sounds,
                lite = lite,
                reducedMotion = reduced,
                showMobileApps = showMobile,
                isDefaultHome = isDefaultHome,
                tvListings = tvListings,
                notifications = notifications,
                enabledChannels = enabledChannels,
                enabledPlugins = enabledPlugins,
                diagnostics = { core.diagnostics() },
                onSetTheme = { t -> scope.launch { prefs.setTheme(t) } },
                onSetSounds = { on -> scope.launch { prefs.setSounds(on) } },
                onSetLite = { on -> scope.launch { prefs.setLiteMode(on) } },
                onSetReducedMotion = { on -> scope.launch { prefs.setReducedMotion(on) } },
                onSetShowMobileApps = { on -> scope.launch { prefs.setShowMobileApps(on) } },
                onRequestPermission = { name -> closeAll(); core.perform("permission:$name") },
                onChannels = { open(Overlay.Channels) },
                onPlugins = { open(Overlay.Plugins) },
                onCompanion = { open(Overlay.Companion) },
                companionOn = uiPrefs.companionEnabled,
                onRecheckHome = { onHomeChanged(); open(Overlay.HomeRole) },
                onPowerSetup = { open(Overlay.PowerSetup) },
                powerStatus = powerStatus,
                onAmbient = { open(Overlay.Ambient) },
                onSetupAgain = { open(Overlay.Onboarding) },
                ambientOn = remember { power.currentScreensaver?.contains(context.packageName) == true },
                firstFocus = first,
            )
        }
        Overlay.Ambient -> AmbientSheet(
            power = power,
            prefs = prefs,
            onPowerSetup = { open(Overlay.PowerSetup) },
            onPreview = { open(Overlay.AmbientPreview) },
            firstFocus = first,
        )
        Overlay.PowerSetup -> PowerSetupSheet(power = power, onHomeRoleChanged = onHomeChanged, firstFocus = first)
        Overlay.TvControls -> TvControlsSheet(
            power = power,
            deviceName = android.os.Build.MODEL,
            onKey = { key -> runCatching { platform.sendKey(key) } },
            onPowerSetup = { open(Overlay.PowerSetup) },
            firstFocus = first,
        )
        Overlay.Channels -> {
            var channels by remember { mutableStateOf(core.availableChannels()) }
            ChannelsSheet(
                channels = channels,
                onToggle = { ch ->
                    core.command(if (ch.enabled) LayoutCommand.DisableChannel(ch.id) else LayoutCommand.EnableChannel(ch.id))
                    channels = channels.map { if (it.id == ch.id) it.copy(enabled = !it.enabled) else it }
                },
                firstFocus = first,
            )
        }
        Overlay.Plugins -> PluginsSheet(core = core, onOpen = { id -> open(Overlay.PluginDetail(id)) }, firstFocus = first)
        is Overlay.PluginDetail -> PluginDetailSheet(
            core = core,
            pluginId = overlay.id,
            onRemoved = { open(Overlay.Plugins) },
            firstFocus = first,
        )
        Overlay.HomeRole -> HomeRoleSheet(
            isDefaultHome = isDefaultHome,
            packageName = context.packageName,
            powerReady = powerStatus is tv.lounge.power.PowerStatus.Ready,
            onDoItNow = {
                scope.launch {
                    runCatching { power.run(power.homeRole) }
                        .onSuccess { r -> if (!r.ok) say(r.output.ifBlank { "That did not work" }) }
                        .onFailure { say(it.message ?: "That did not work") }
                    onHomeChanged()
                }
            },
            onPowerSetup = { open(Overlay.PowerSetup) },
            onRecheck = onHomeChanged,
            firstFocus = first,
        )
        Overlay.Companion -> CompanionSheet(
            core = core,
            enabled = uiPrefs.companionEnabled,
            onSetEnabled = { on -> scope.launch { prefs.setCompanionEnabled(on) } },
            firstFocus = first,
        )
        else -> Unit
    }
}

/**
 * The Stage's slot: the only place that reads the selection and expansion, so moving along the
 * strip recomposes this small function and not the whole home screen.
 */
@Composable
private fun StageSlot(
    snapshot: HomeSnapshot,
    state: HomeState,
    onPerform: (Tile) -> Unit,
    onMenu: (Tile, Rect, FocusRequester) -> Unit,
    onPrimary: (StageSpotlight) -> Unit,
    onSecondary: (StageSpotlight) -> Unit,
    onBandFocused: () -> Unit,
) {
    val uiSounds = LoungeTheme.sounds
    val motion = LoungeTheme.motion
    val expanded = state.stageExpanded
    val height = animateDpAsState(if (expanded) StageExpandedHeight else StageCollapsedHeight, motion.proscenium(), label = "stageHeight")
    SideEffect { state.stageHeight = height }
    val light = LocalLightState.current
    LaunchedEffect(height) { snapshotFlow { height.value }.collect { light.sourceMoved() } }
    val count = snapshot.stage.strip.size
    val selected = remember(count) { derivedStateOf { state.stageIndex.coerceIn(0, (count - 1).coerceAtLeast(0)) } }
    StageView(
        stage = snapshot.stage,
        selectedIndex = selected.value,
        selected = remember(selected) { object : androidx.compose.runtime.IntState { override val intValue: Int get() = selected.value } },
        expanded = expanded,
        height = height,
        idle = state.idle,
        indexed = snapshot.revision > 0uL,
        stripRequester = state::stripRequester,
        stripFocus = { state.stripFocus() },
        playbillFocus = state.playbillFocus,
        upFocus = state.topBarFocus,
        onPlaybillFocusChanged = { state.playbillFocused = it },
        onStripFocused = { index, _ ->
            if (state.stageIndex != index || state.focusedRowId != null) uiSounds?.tick()
            state.stageIndex = index
            state.focusedRowId = null
        },
        onStripClick = onPerform,
        onStripLongPress = onMenu,
        onPrimary = onPrimary,
        onSecondary = onSecondary,
        onBandFocused = onBandFocused,
    )
}
