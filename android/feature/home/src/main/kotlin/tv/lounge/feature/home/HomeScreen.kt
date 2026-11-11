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
import androidx.compose.foundation.layout.fillMaxWidth
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
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusDirection
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.isSpecified
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onKeyEvent
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.layout.layout
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.repeatOnLifecycle
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import tv.lounge.core.CoreClient
import tv.lounge.core.HomeSnapshot
import tv.lounge.core.LayoutCommand
import tv.lounge.core.StageSpotlight
import tv.lounge.core.Tile
import tv.lounge.core.reason
import tv.lounge.design.FieldSleep
import tv.lounge.design.LightField
import tv.lounge.design.LocalLightState
import tv.lounge.design.LoungeTheme
import tv.lounge.design.LoungeToastHost
import tv.lounge.design.Presence
import tv.lounge.design.RoomLights
import tv.lounge.design.ToastController
import tv.lounge.design.sounds
import tv.lounge.platform.AndroidPlatform
import tv.lounge.platform.Prefs
import tv.lounge.platform.UiCommand
import kotlin.math.roundToInt

private const val IDLE_AFTER_MS = 20_000L
/** Home within this long of the last Home press keeps the strip's selection; later it starts over. */
private const val HOME_KEEPS_PLACE_MS = 10 * 60_000L

/** Set once per process: the lamp warms up on a cold start only, not on every return home. */
private var lampLit = false

/**
 * The home screen: top bar, Stage, rows, and the overlays, over the light field.
 * One screen, one job (DESIGN.md §2).
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun HomeScreen(core: CoreClient, platform: AndroidPlatform, prefs: Prefs, uiPrefs: Prefs.UiPrefs, power: tv.lounge.power.PowerSetup, updates: tv.lounge.update.UpdateManager) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val powerStatus by power.status.collectAsStateWithLifecycle()
    val snapshot by core.snapshots.collectAsStateWithLifecycle()
    val tierA = LoungeTheme.visuals.tierA
    val state = remember { HomeState(tierA) }
    val toasts = remember { ToastController() }
    val light = LocalLightState.current
    val motion = LoungeTheme.motion
    val theme = uiPrefs.theme
    val sounds = uiPrefs.sounds
    val lite = uiPrefs.liteMode
    val reduced = uiPrefs.reducedMotion
    val showMobile = uiPrefs.showMobileApps
    val liveBackdrops = LoungeTheme.visuals.shaderWallpaper && !motion.reducedMotion
    var isDefaultHome by remember { mutableStateOf(isDefaultHome(context)) }
    val uiSounds = LoungeTheme.sounds
    // The sounds arrive after the first frame, so the setting is applied again when they do.
    LaunchedEffect(sounds, uiSounds) { uiSounds?.enabled = sounds }

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
    // Coming back from another app counts as input; the Home key that brings Lounge back never reaches Compose.
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) { state.touched() }
    // The core writes times as the TV's clock shows them; the setting may have changed while away.
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) {
        val is24Hour = android.text.format.DateFormat.is24HourFormat(context)
        scope.launch(Dispatchers.IO) { core.setClock24h(is24Hour) }
    }
    // First run: the persisted snapshot paints first, then the setup steps come up over it. Until
    // they are finished they come back whenever nothing else is open.
    val onboardingDone = rememberUpdatedState(uiPrefs.onboardingDone)
    var setupFinished by remember { mutableStateOf(false) }
    LaunchedEffect(uiPrefs.loaded, uiPrefs.onboardingDone, state.overlay == Overlay.None) {
        if (uiPrefs.loaded && !uiPrefs.onboardingDone && !setupFinished && state.overlay == Overlay.None) {
            delay(900)
            state.overlay = Overlay.Onboarding
        }
    }
    // Followed as a flow: read here, the room going quiet would recompose the whole home.
    LaunchedEffect(light) { snapshotFlow { state.idle }.collect { light.idle = it } }
    RoomLights(light)
    val overlay = state.overlay
    val menu = overlay as? Overlay.TileMenu
    // A menu opened from a search result keeps Search open beneath it.
    val searchOpen = overlay == Overlay.Search || menu?.under == Overlay.Search
    val allAppsOpen = overlay == Overlay.AllApps || menu?.under == Overlay.AllApps
    val homeMenu = menu?.takeIf { it.under == Overlay.None }
    // The rows read the open menu through these, so opening one recomposes only the tile it came from.
    val heldMenu: () -> Overlay.TileMenu? = { state.overlay as? Overlay.TileMenu }
    val heldHomeMenu: () -> Overlay.TileMenu? = { heldMenu()?.takeIf { it.under == Overlay.None } }
    val opaqueOverlay = searchOpen || allAppsOpen || overlay == Overlay.Onboarding || overlay == Overlay.AmbientPreview
    FieldSleep(light, { state.lastInputAt }, opaqueOverlay)
    // Rows off screen are built in idle time and every row is kept once built, so neither the
    // proscenium nor a move between rows composes a row in the middle of a frame.
    val rowIdsNow = rememberUpdatedState(remember(snapshot.rows) { snapshot.rows.map { it.id } })
    val rowsAhead = remember { EveryRowPrefetch { rowIdsNow.value } }
    val listState = rememberLazyListState(prefetchStrategy = rowsAhead)
    // Rows asked for while the window was hidden are never built, so once a frame has been drawn
    // in front they are asked for again.
    val homeOwner = LocalLifecycleOwner.current
    LaunchedEffect(homeOwner, rowsAhead) {
        homeOwner.lifecycle.repeatOnLifecycle(Lifecycle.State.RESUMED) {
            withFrameNanos { }
            rowsAhead.resume()
        }
    }
    // A row that arrives at the top while the Stage is up (an update notice) must show at the
    // top: the list would otherwise keep its first row in place and hide the new one above it.
    val firstRowId = snapshot.rows.firstOrNull()?.id
    LaunchedEffect(firstRowId) {
        if (firstRowId != null && state.region != FocusRegion.Rows && listState.firstVisibleItemIndex <= 1) listState.scrollToItem(0)
    }

    fun say(text: String) = toasts.show(text)

    // The updater's notice tile and the one-time word after an update.
    LaunchedEffect(updates) {
        updates.takeJustUpdatedFrom()?.let { say("Lounge updated to ${updates.currentVersion}") }
        updates.state.collect { s ->
            val waiting = s is tv.lounge.update.UpdateState.NeedsConfirmation ||
                (s is tv.lounge.update.UpdateState.Ready && s.path == tv.lounge.update.InstallPath.NeedsPermission)
            core.setUpdateNotice(if (waiting) s.release?.version?.toString() else null)
        }
    }

    // What the updater says before it hands over to one of Android's screens.
    LaunchedEffect(updates) { updates.hints.collect { say(it) } }

    /** Core actions run off the main thread: a plugin may take a moment to answer. */
    fun perform(action: String, title: String) {
        scope.launch {
            withContext(Dispatchers.IO) { core.perform(action) }.onFailure { e -> say(friendlyError(e, title)) }
        }
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

    /** Focus back on the home where it was, or as near as the current rows allow. */
    suspend fun focusHome() {
        when (state.region) {
            FocusRegion.TopBar -> {
                // The Now Playing chip leaves with its session; Settings sits where it was.
                val chipGone = state.topBar.last === state.topBar.nowPlaying && snapshot.nowPlaying == null
                val last = if (chipGone) state.topBar.settings else state.topBar.last
                if (!last.requestWhenReady(6)) state.topBarFocus.requestWhenReady()
            }
            FocusRegion.Stage -> if (!(state.playbillFocused && state.playbillFocus.requestWhenReady(6))) state.focusStrip()
            FocusRegion.Rows -> {
                val rows = snapshot.rows
                val row = rows.firstOrNull { it.id == state.focusedRowId }
                when {
                    row != null && row.tiles.isNotEmpty() -> state.rowRequester(row.id, state.focusedTileIndex.coerceIn(0, row.tiles.lastIndex)).requestWhenReady()
                    rows.isNotEmpty() -> state.focusRow(rows.first().id)
                    else -> goToStage()
                }
            }
        }
    }

    fun clearOverlays() {
        state.overlay = Overlay.None
        state.opener = null
        state.sheetTrail.clear()
        state.sheetLeft = null
        state.pendingSearch = null
    }

    /** Opens [next] from the home, remembering what had focus so closing it can go back there. */
    fun openOverlay(next: Overlay, from: FocusRequester? = null) {
        if (from != null) {
            state.opener = from
        } else if (state.overlay == Overlay.None) {
            state.opener = state.currentFocus()
        }
        state.sheetTrail.clear()
        state.sheetLeft = null
        state.overlay = next
    }

    /** Closes whatever is open and hands focus back to what opened it (DESIGN.md §6.4). */
    fun closeOverlay(sound: Boolean = true) {
        if (sound) close()
        val back = state.opener?.takeUnless { it === state.topBar.nowPlaying && snapshot.nowPlaying == null }
        clearOverlays()
        scope.launch { if (back == null || !back.requestWhenReady(6)) focusHome() }
    }

    /** One sheet opening another: Back from [next] returns to the sheet that opened it. */
    fun openSheet(next: Overlay) {
        val current = state.overlay
        if (next.isSheet() && current.isSheet()) {
            val back = state.sheetTrail.lastIndexOf(next)
            if (back >= 0) {
                while (state.sheetTrail.size > back) state.sheetTrail.removeAt(state.sheetTrail.lastIndex)
                state.sheetLeft = current
            } else {
                state.sheetTrail += current
                state.sheetLeft = null
            }
        }
        state.overlay = next
    }

    fun perform(tile: Tile) {
        // A tile to read opens nothing, so nothing sounds.
        if (tile.action.isEmpty()) return
        uiSounds?.open()
        // The All apps tile opens a screen of this app's own; no need to go through the core for that.
        if (tile.action == "control:allapps") {
            openOverlay(Overlay.AllApps)
            return
        }
        // So does a suggested plugin: its page is where it is turned on and set up.
        if (tile.action.startsWith("control:plugin:")) {
            openOverlay(Overlay.PluginDetail(tile.action.removePrefix("control:plugin:")))
            return
        }
        perform(tile.action, tile.title)
    }

    // Text from the phone goes to Lounge's focused text field, and only while Lounge is in front.
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val phoneText = remember { MutableSharedFlow<String>(extraBufferCapacity = 8) }
    LaunchedEffect(Unit) {
        platform.commands.collect { cmd ->
            when (cmd) {
                is UiCommand.Toast -> say(cmd.message)
                UiCommand.InstallUpdate -> Unit
                UiCommand.OpenTvControls -> openOverlay(Overlay.TvControls)
                // From the phone: only while Lounge is in front with nothing open, or the screen would wait behind another app.
                UiCommand.OpenAllApps -> if (state.overlay == Overlay.None && lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)) { uiSounds?.open(); openOverlay(Overlay.AllApps) }
                is UiCommand.OpenPlugin -> if (state.overlay == Overlay.None && lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)) { uiSounds?.open(); openOverlay(Overlay.PluginDetail(cmd.id)) }
                is UiCommand.RequestPermission, is UiCommand.Key -> Unit
                is UiCommand.TypeText -> if (lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)) phoneText.tryEmit(cmd.text)
                is UiCommand.Search -> {
                    state.pendingSearch = cmd.query
                    if (state.overlay != Overlay.Search) openOverlay(Overlay.Search)
                }
                // Home from another app: the rows are already home when the window appears. After
                // a while away the Stage opens on its first tile again (DESIGN.md 6.4).
                UiCommand.GoHome -> {
                    state.touched()
                    val now = System.currentTimeMillis()
                    if (now - state.lastHomeAt > HOME_KEEPS_PLACE_MS) {
                        state.stageTileId = null
                        state.stageIndex = 0
                        state.stageChosen = false
                    }
                    state.lastHomeAt = now
                    // Setup steps not yet finished stay up at the step reached.
                    if (state.overlay != Overlay.Onboarding || onboardingDone.value) {
                        clearOverlays()
                        goToStage(snap = true)
                    }
                }
            }
        }
    }

    fun openMenu(tile: Tile, rowId: String, anchor: Rect, returnFocus: FocusRequester) {
        if (!hasMenu(tile)) return
        val resolved = if (anchor != Rect.Zero) anchor else if (light.center.isSpecified) {
            Rect(light.center.x - 100f, light.center.y - 60f, light.center.x + 100f, light.center.y + 60f)
        } else {
            Rect.Zero
        }
        val under = when (state.overlay) {
            Overlay.Search, Overlay.AllApps -> state.overlay
            else -> Overlay.None
        }
        state.overlay = Overlay.TileMenu(tile, rowId, resolved, returnFocus, under)
    }

    /** Where [tileId] sits in the home row [rowId] right now, or -1. */
    fun indexInRow(rowId: String, tileId: String): Int =
        snapshot.rows.firstOrNull { it.id == rowId }?.tiles?.indexOfFirst { it.id == tileId } ?: -1

    val hasStrip = snapshot.stage.strip.isNotEmpty()
    val nowPlaying = snapshot.nowPlaying
    LaunchedEffect(nowPlaying == null) {
        if (nowPlaying == null && state.overlay == Overlay.NowPlaying) closeOverlay(sound = false)
    }

    // A tile hidden or unpinned while it had focus leaves its row: focus moves to its neighbour,
    // or to the row that took its place.
    val rowIds = remember { arrayOf(emptyList<String>()) }
    suspend fun placeRowFocus(before: List<String>) {
        val rowId = state.focusedRowId ?: return
        val tileId = state.focusedTileId ?: return
        when (val next = rowFocusAfter(before, snapshot.rows, rowId, tileId, state.focusedTileIndex)) {
            // Still there, perhaps elsewhere in its row: the index follows it, and the row
            // scrolls to it if it moved out of view.
            is RowFocus.Stays -> {
                state.focusedTileIndex = next.index
                val list = state.rowListState(rowId)
                if (list.layoutInfo.visibleItemsInfo.none { it.key == tileId }) list.scrollToItem(next.index)
            }
            // The new tile is named first, so the way back in is open to it.
            is RowFocus.Neighbour -> {
                state.focusedTileId = snapshot.rows.firstOrNull { it.id == rowId }?.tiles?.getOrNull(next.index)?.id
                state.rowRequester(rowId, next.index).requestWhenReady()
            }
            is RowFocus.NextRow -> {
                state.focusedTileId = null
                state.focusRow(next.rowId)
            }
            RowFocus.Stage -> {
                state.focusedTileId = null
                goToStage()
            }
        }
    }

    /** Whether the row tile that has focus has just left its row, leaving focus nowhere. */
    fun focusedTileGone(): Boolean {
        val id = state.focusedTileId ?: return false
        val row = snapshot.rows.firstOrNull { it.id == state.focusedRowId }
        return row == null || row.tiles.none { it.id == id }
    }
    LaunchedEffect(snapshot) {
        val before = rowIds[0]
        rowIds[0] = snapshot.rows.map { it.id }
        if (state.overlay != Overlay.None) return@LaunchedEffect
        when (state.region) {
            FocusRegion.Rows -> placeRowFocus(before)
            FocusRegion.Stage -> {
                val strip = snapshot.stage.strip
                if (state.playbillFocused || strip.isEmpty()) return@LaunchedEffect
                // Until the viewer moves along the strip, focus stays on its first tile, which may
                // be one that has just arrived ahead of the focused one.
                if (!state.stageChosen) {
                    state.stripRequester(0).requestWhenReady()
                    return@LaunchedEffect
                }
                val id = state.stageTileId ?: return@LaunchedEffect
                if (strip.any { it.id == id }) return@LaunchedEffect
                state.stripRequester(state.stageIndex.coerceIn(0, strip.lastIndex)).requestWhenReady()
            }
            FocusRegion.TopBar -> if (state.topBar.last === state.topBar.nowPlaying && snapshot.nowPlaying == null) focusHome()
        }
    }

    // While an anchored menu is open the room dims around its tile (the rows dim their tiles).
    val roomAlpha = animateFloatAsState(if (homeMenu != null) 0.5f else 1f, motion.settle(), label = "roomAlpha")
    val focusManager = LocalFocusManager.current

    Box(
        Modifier
            .fillMaxSize()
            .onKeyEvent { e ->
                // A d-pad move that finds nothing stays where it is. Left to Compose, a failed
                // search clears focus and starts over from the first element, which from the last
                // row looked like a loop back to the top.
                val direction = when (e.key) {
                    Key.DirectionUp -> FocusDirection.Up
                    Key.DirectionDown -> FocusDirection.Down
                    Key.DirectionLeft -> FocusDirection.Left
                    Key.DirectionRight -> FocusDirection.Right
                    else -> null
                }
                if (direction != null && e.type == KeyEventType.KeyDown) {
                    focusManager.moveFocus(direction)
                    true
                } else {
                    false
                }
            }
            .focusProperties {
                onEnter = {
                    if (hasStrip && state.overlay == Overlay.None && state.region == FocusRegion.Stage) {
                        runCatching { state.stripFocus().requestFocus() }
                    } else if (state.overlay == Overlay.None && state.region == FocusRegion.Rows && focusedTileGone()) {
                        // A focused tile that left its row would let focus come back in at the top
                        // bar; it goes to the tile's neighbour instead.
                        cancelFocusChange()
                        scope.launch { placeRowFocus(rowIds[0]) }
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
                when {
                    e.isBack() && state.overlay == Overlay.None -> {
                        if (e.type == KeyEventType.KeyUp) {
                            when (homeBack(state.region, state.playbillFocused)) {
                                HomeBack.ToStage -> {
                                    close()
                                    goToStage()
                                }
                                HomeBack.ToStrip -> scope.launch { state.focusStrip() }
                                HomeBack.Stay -> Unit
                            }
                        }
                        true
                    }
                    // A controller's Start searches and its Select opens Settings (DESIGN.md §11).
                    (e.key == Key.ButtonStart || e.key == Key.ButtonSelect) && state.overlay == Overlay.None -> {
                        if (e.type == KeyEventType.KeyUp) {
                            uiSounds?.open()
                            openOverlay(if (e.key == Key.ButtonStart) Overlay.Search else Overlay.QuickSettings)
                        }
                        true
                    }
                    else -> false
                }
            },
    ) {
        // A live scene behind the room when one is chosen and this TV can draw it; the still
        // field otherwise, and again when a scene proves too heavy.
        // Read off the main thread; the still field shows until the scene is known.
        val chosenStyle by produceState<ShaderStyle?>(null, uiPrefs.backdrop, liveBackdrops) {
            value = if (!liveBackdrops || uiPrefs.backdrop == Prefs.STILL_BACKDROP) null else withContext(Dispatchers.IO) { ShaderStyles.find(context, core, uiPrefs.backdrop) }
        }
        val liveStyle = chosenStyle
        var liveFailed by remember(uiPrefs.backdrop) { mutableStateOf(false) }
        if (liveStyle != null && !liveFailed && android.os.Build.VERSION.SDK_INT >= 33) {
            LiveBackdrop(liveStyle, light, onUnusable = { why ->
                android.util.Log.w("Lounge", "background: ${liveStyle.id} falls back to the still field: $why")
                liveFailed = true
                // A compile error's own text is for the log, not the screen (DESIGN.md §8).
                say(if ("too heavy" in why) "${liveStyle.name} is too heavy for this TV, so the background is still" else "${liveStyle.name} could not be drawn on this TV, so the background is still")
            })
        } else {
            LightField(light)
        }

        Column(Modifier.fillMaxSize()) {
            TopBar(
                showTvControls = power.hasTvControls,
                showGuide = (snapshot.rows + snapshot.hiddenRows).any { it.id == "live" },
                nowPlaying = nowPlaying,
                focus = state.topBar,
                downFocus = { state.stripFocus() },
                onFocused = { state.region = FocusRegion.TopBar; light.clear() },
                onSearch = { uiSounds?.open(); openOverlay(Overlay.Search) },
                onTvControls = { uiSounds?.open(); openOverlay(Overlay.TvControls) },
                onGuide = { uiSounds?.open(); openOverlay(Overlay.Guide) },
                onNowPlaying = { uiSounds?.open(); openOverlay(Overlay.NowPlaying) },
                onSettings = { uiSounds?.open(); openOverlay(Overlay.QuickSettings) },
                modifier = Modifier.graphicsLayer { alpha = roomAlpha.value },
            )
            Spacer(Modifier.height(34.dp))
            Box(Modifier.fillMaxSize()) {
                // The rows keep one layout whatever the Stage does: a layer translation moves them
                // and a clip hides what sits under the lowered Stage, so neither the proscenium
                // nor the band closing lays them out.
                val density = androidx.compose.ui.platform.LocalDensity.current
                val headerPx = with(density) { tv.lounge.design.Dimens.rowHeaderHeight.toPx() }
                val collapsedPx = with(density) { StageCollapsedHeight.toPx() }
                val belowStage = remember(collapsedPx) { TopInsetShape(collapsedPx) }
                BandRise(state, firstRowId)
                CompositionLocalProvider(LocalBringIntoViewSpec provides pivotSpec(collapsedPx + headerPx)) {
                    LazyColumn(
                        state = listState,
                        // Room for the lowered Stage above the first row, and enough below the last row for it to reach the focus line.
                        contentPadding = PaddingValues(top = StageCollapsedHeight, bottom = 760.dp + StageCollapsedHeight),
                        modifier = Modifier
                            .fillMaxWidth()
                            // Taller than its place by the band, so the rows still reach the foot of the screen once they have risen into it.
                            .tallerBy(collapsedPx.roundToInt())
                            .graphicsLayer {
                                translationY = (state.stageHeight?.value?.toPx() ?: 0f) - collapsedPx * (1f + (state.bandRise?.value ?: 0f))
                                shape = belowStage
                                clip = true
                            }
                            .onFocusChanged { if (it.hasFocus) state.region = FocusRegion.Rows }
                            .onPreviewKeyEvent { e ->
                                // Down from the last row stays there: the lowered Stage's strip
                                // sits below it in layout and must not be found.
                                e.key == Key.DirectionDown && e.type == KeyEventType.KeyDown && state.focusedRowId == snapshot.rows.lastOrNull()?.id
                            }
                            .focusGroup(),
                    ) {
                    items(snapshot.rows, key = { it.id }) { row ->
                        // A row kept off screen, or wholly under the lowered Stage, is told so, and its
                        // tiles then wait to be drawn until it shows.
                        val shown = remember(row.id) { derivedStateOf { listState.layoutInfo.visibleItemsInfo.any { it.key == row.id && it.offset + it.size > 0 } } }
                        ContentRow(
                            row = row,
                            isFocusedRow = { state.focusedRowId == row.id },
                            listState = state.rowListState(row.id),
                            requesterFor = { index -> state.rowRequester(row.id, index) },
                            onTileFocused = { tile, index, _ ->
                                if (state.focusedRowId != row.id) uiSounds?.tick(rowChange = true) else if (state.focusedTileIndex != index) uiSounds?.tick()
                                state.focusedRowId = row.id
                                state.focusedTileIndex = index
                                state.focusedTileId = tile.id
                            },
                            onTileClick = { perform(it) },
                            onTileLongPress = { tile, bounds, fr -> openMenu(tile, row.id, bounds, fr) },
                            modifier = keptOnceMeasured(),
                            heldTileId = { heldHomeMenu()?.tile?.id },
                            heldRowId = { heldHomeMenu()?.rowId },
                            shown = { shown.value },
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
                                scope.launch {
                                    // The first row has to be composed to take focus.
                                    if (listState.firstVisibleItemIndex > 0) listState.scrollToItem(0)
                                    state.focusRow(firstRow.id)
                                }
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
                        onStart = { uiSounds?.open(); openOverlay(Overlay.Onboarding) },
                    )
                }
            }
        }

        // Overlays stay composed through their exit so each can animate out.
        Presence(overlay.takeIf { it.isSheet() }) { ov, visibleState ->
            RightSheet(
                visibleState = visibleState,
                key = ov,
                depth = ov.sheetDepth(),
                onDismiss = {
                    val previous = state.sheetTrail.removeLastOrNull()
                    if (previous != null) {
                        close()
                        state.sheetLeft = ov
                        state.overlay = previous
                    } else {
                        closeOverlay()
                    }
                },
            ) { first, shown ->
                CompositionLocalProvider(LocalPhoneText provides phoneText) {
                    SheetContent(
                        overlay = shown,
                        first = first,
                        cameFrom = state.sheetLeft,
                        core = core,
                        platform = platform,
                        prefs = prefs,
                        uiPrefs = uiPrefs,
                        power = power,
                        updates = updates,
                        powerStatus = powerStatus,
                        theme = theme,
                        sounds = sounds,
                        lite = lite,
                        reduced = reduced,
                        showMobile = showMobile,
                        isDefaultHome = isDefaultHome,
                        onHomeChanged = { isDefaultHome = isDefaultHome(context) },
                        say = ::say,
                        open = { next -> uiSounds?.open(); openSheet(next) },
                        forget = { gone -> state.sheetTrail.removeAll { it == gone } },
                    )
                }
            }
        }

        Presence(if (overlay == Overlay.Guide) Unit else null) { _, visibleState ->
            GuideSheet(
                loadGuide = { from -> core.guide(from, 3u) },
                visibleState = visibleState,
                onTune = { action -> uiSounds?.open(); perform(action, "that channel") },
                onDismiss = { closeOverlay() },
            )
        }

        Presence(nowPlaying.takeIf { overlay == Overlay.NowPlaying }) { np, visibleState ->
            NowPlayingCard(
                np = np,
                visibleState = visibleState,
                onCommand = { cmd -> perform("media:${np.`package`}:$cmd", np.title) },
                onOpenApp = { uiSounds?.open(); runCatching { platform.launchApp(np.`package`, "") }.onFailure { say(friendlyError(it, np.appLabel)) } },
                onDismiss = { closeOverlay() },
            )
        }

        Presence(if (searchOpen) Unit else null) { _, visibleState ->
            CompositionLocalProvider(LocalPhoneText provides phoneText) {
                SearchScreen(
                    snapshot = snapshot,
                    visibleState = visibleState,
                    pendingQuery = state.pendingSearch,
                    onQueryTaken = { state.pendingSearch = null },
                    covered = menu != null,
                    heldTileId = { heldMenu()?.tile?.id },
                    heldRowId = { heldMenu()?.rowId },
                    onTileFocused = { _, _ -> },
                    onTileClick = { perform(it) },
                    onTileLongPress = { tile, rowId, bounds, fr -> openMenu(tile, rowId, bounds, fr) },
                    onClose = { closeOverlay() },
                    searchPlugins = { q -> core.searchPlugins(q) },
                )
            }
        }

        Presence(if (allAppsOpen) Unit else null) { _, visibleState ->
            CompositionLocalProvider(LocalPhoneText provides phoneText) {
                AllAppsScreen(
                    apps = snapshot.allApps,
                    sort = uiPrefs.allAppsSort,
                    onSetSort = { s -> scope.launch { prefs.setAllAppsSort(s) } },
                    visibleState = visibleState,
                    covered = menu != null,
                    heldTileId = { heldMenu()?.tile?.id },
                    onTileClick = { perform(it) },
                    onTileLongPress = { tile, bounds, fr -> openMenu(tile, "allapps", bounds, fr) },
                    onClose = { closeOverlay() },
                )
            }
        }

        // The menu draws over Search or All apps when it opened from one of their tiles.
        Presence(menu) { ov, visibleState ->
            val pkg = ov.tile.id.removePrefix("app:")
            // Search keeps the home's row ids; hiding a row from there would be a surprise.
            val homeRow = if (ov.under == Overlay.Search) null else snapshot.rows.firstOrNull { it.id == ov.rowId }
            TileMenuPopover(
                tile = ov.tile,
                anchor = ov.anchor,
                items = menuItemsFor(
                    tile = ov.tile,
                    rowId = ov.rowId,
                    rowTitle = homeRow?.title,
                    isPinned = ov.tile.pinned,
                    // Moves stay among the pins: hopping past a recent app would pin it unasked.
                    canMoveLeft = indexInRow(ov.rowId, ov.tile.id) > 0,
                    canMoveRight = indexInRow(ov.rowId, ov.tile.id).let { at -> at >= 0 && (homeRow?.tiles?.getOrNull(at + 1)?.pinned == true) },
                    isStagePinned = ov.tile.stagePinned,
                    onHide = { core.command(LayoutCommand.HideItem(ov.tile.id)); say("Hidden from every row") },
                    onShow = { core.command(LayoutCommand.UnhideItem(ov.tile.id)); say("Shown on every row again") },
                    onHideAllFrom = {
                        ov.tile.provider?.let { core.command(LayoutCommand.MuteProvider(it)) }
                        say("Everything from ${ov.tile.providerName} hidden. Settings, Rows brings it back")
                    },
                    onHideRow = {
                        core.command(LayoutCommand.HideRow(ov.rowId))
                        say("${homeRow?.title ?: "Row"} hidden. Settings, Rows brings it back")
                    },
                    onPin = { core.command(LayoutCommand.PinApp(pkg)); say("Pinned to the front of Your apps") },
                    onUnpin = { core.command(LayoutCommand.UnpinApp(pkg)); say("Unpinned from Your apps") },
                    // The tile's place in Your apps now, wherever its menu was opened from.
                    onMoveLeft = {
                        val at = indexInRow(ov.rowId, ov.tile.id)
                        if (at >= 0) core.command(LayoutCommand.MoveApp(pkg, (at - 1).coerceAtLeast(0).toUInt()))
                    },
                    onMoveRight = {
                        val at = indexInRow(ov.rowId, ov.tile.id)
                        if (at >= 0) core.command(LayoutCommand.MoveApp(pkg, (at + 1).toUInt()))
                    },
                    onPinStage = { core.command(LayoutCommand.PinToStage(ov.tile.id)); say("Pinned to the Stage") },
                    onUnpinStage = { core.command(LayoutCommand.UnpinFromStage(ov.tile.id)); say("Unpinned from the Stage") },
                    onAppInfo = { perform("info:${ov.tile.id}", ov.tile.title) },
                    onUninstall = { requestUninstall(context, pkg) },
                    onPlay = { openPlayListing(context, pkg) },
                    onDismissNotice = { core.command(LayoutCommand.DismissNotice(ov.tile.id)) },
                    onGuide = { openOverlay(Overlay.Guide, from = ov.returnFocus) },
                ),
                visibleState = visibleState,
                onDismiss = {
                    if (state.overlay == ov) {
                        close()
                        state.overlay = ov.under
                        scope.launch { ov.returnFocus.requestWhenReady() }
                    }
                },
            )
        }

        Presence(if (overlay == Overlay.AmbientPreview) Unit else null) { _, visibleState ->
            AnimatedVisibility(visibleState, enter = fadeIn(motion.curtain()), exit = fadeOut(motion.settle())) {
                AmbientPreview(core = core, prefs = prefs, onClose = { close(); state.overlay = Overlay.Ambient })
            }
        }

        Presence(if (overlay == Overlay.Onboarding) Unit else null) { _, visibleState ->
            OnboardingScreen(
                core = core,
                prefs = prefs,
                uiPrefs = uiPrefs,
                power = power,
                visibleState = visibleState,
                say = ::say,
                onDone = {
                    close()
                    val again = uiPrefs.onboardingDone
                    setupFinished = true
                    scope.launch { prefs.setOnboardingDone(true) }
                    isDefaultHome = isDefaultHome(context)
                    if (again) {
                        // Run again from Settings, so it ends on the row that started it.
                        state.sheetLeft = Overlay.Onboarding
                        state.overlay = Overlay.QuickSettings
                    } else {
                        clearOverlays()
                        goToStage()
                    }
                },
            )
        }

        LoungeToastHost(toasts, Modifier.align(androidx.compose.ui.Alignment.BottomStart))
    }

    LaunchedEffect(hasStrip) {
        if (hasStrip && state.region == FocusRegion.Stage && state.overlay == Overlay.None) state.focusStrip()
    }
    // An empty home has no strip; its Start button takes focus instead.
    val emptyHome = snapshot.revision > 0uL && snapshot.stage.spotlights.isEmpty()
    LaunchedEffect(emptyHome) {
        if (emptyHome && state.region == FocusRegion.Stage && state.overlay == Overlay.None) state.playbillFocus.requestWhenReady()
    }

    // Any Back the UI didn't consume still means "home": never let it reach the Activity.
    BackHandler(enabled = true) {
        if (state.overlay == Overlay.None) goToStage() else closeOverlay()
    }
}

/** The activity's start time, set by the app module. */
var startupReference: () -> Long = { 0L }

/**
 * Shuts the lowered Stage while focus is below the first row and opens it again on the way up,
 * over `proscenium` like the lowering it continues (DESIGN.md §5.2). It reads which row has focus
 * here, so a move between rows recomposes this and not the home.
 */
@Composable
private fun BandRise(state: HomeState, firstRowId: String?) {
    val away = state.region == FocusRegion.Rows && state.focusedRowId.let { it != null && it != firstRowId }
    val rise = animateFloatAsState(if (away) 1f else 0f, LoungeTheme.motion.proscenium(), label = "bandRise")
    SideEffect { state.bandRise = rise }
    // The focused tile moves on screen with the rows, and its light has to follow.
    val light = LocalLightState.current
    LaunchedEffect(rise) { snapshotFlow { rise.value }.collect { light.sourceMoved() } }
}

/** Measures the content [extra] px taller than its place and lets it run on below. */
private fun Modifier.tallerBy(extra: Int): Modifier = layout { measurable, constraints ->
    val height = constraints.maxHeight + extra
    val placeable = measurable.measure(constraints.copy(minHeight = height, maxHeight = height))
    layout(placeable.width, constraints.maxHeight) { placeable.place(0, 0) }
}

/** Everything below [top]: the part of the rows' layer the lowered Stage does not cover. */
private data class TopInsetShape(val top: Float) : androidx.compose.ui.graphics.Shape {
    override fun createOutline(size: androidx.compose.ui.geometry.Size, layoutDirection: androidx.compose.ui.unit.LayoutDirection, density: androidx.compose.ui.unit.Density) =
        androidx.compose.ui.graphics.Outline.Rectangle(androidx.compose.ui.geometry.Rect(0f, top, size.width, size.height))
}

private fun Overlay.isSheet(): Boolean = when (this) {
    Overlay.QuickSettings, Overlay.Ambient, Overlay.PowerSetup, Overlay.TvControls, Overlay.Channels, Overlay.Rows,
    Overlay.Plugins, is Overlay.PluginDetail, Overlay.HomeRole, Overlay.Companion, Overlay.Updates,
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

/**
 * A failure to open [title] as a sentence with its cause and its fix (DESIGN.md §7.4, §10). The
 * raw reason goes to the log only; one not known here gets a plain sentence, never its own text.
 */
internal fun friendlyError(e: Throwable, title: String): String {
    val m = e.reason
    android.util.Log.w("Lounge", "couldn't open $title: $m")
    return friendlyReason(e, m, title)
}

/** The sentence for [reason], the failure's own words, kept apart from the log so it can be tested. */
internal fun friendlyReason(e: Throwable, reason: String, title: String): String {
    val m = reason.lowercase()
    return when {
        // A plugin's own sentence, and the host's for one that stopped, show as written.
        e is tv.lounge.core.CoreException.Plugin && readsAsSentence(reason) -> reason.trim()
        e is tv.lounge.core.CoreException.DiskFull -> "The TV's storage is full. Free some space in the TV's settings, then try again."
        e is tv.lounge.core.CoreException.UnknownAction -> "$title isn't on this TV any more. Its app may have removed it."
        "no activity found" in m || "activitynotfound" in m || "no launch intent" in m || "nothing can open" in m ->
            "Nothing on this TV can open $title. Is its app still installed?"
        "not installed" in m -> "$title isn't installed on this TV. Install it from the Play Store, then try again."
        "no video player" in m -> "Nothing on this TV can play $title. Install VLC or another video player, then try again."
        "notification access" in m -> "Lounge can't reach what's playing yet. In Settings, turn on Now playing."
        "not playing" in m -> "That app has stopped playing. Open it to start something again."
        "did not answer" in m || "didn't answer" in m -> "$title's source didn't answer. Check that it is running, then try again."
        "busy" in m -> "The TV is busy. Try again in a moment."
        "won't open that link" in m || "not one lounge can open" in m -> "Lounge can't open $title from here. Open it from its app instead."
        "permission" in m || "not allowed" in m || "security" in m -> "Android didn't let Lounge open $title. Open it from its app instead."
        e is tv.lounge.core.CoreException.Plugin -> "$title's plugin stopped partway through. Try again in a moment."
        else -> "Couldn't open $title. Try again in a moment."
    }
}

/** Whether [text] is already a sentence for the screen: a capital first, a full stop or question mark last. */
internal fun readsAsSentence(text: String): Boolean {
    val t = text.trim()
    return t.firstOrNull()?.isUpperCase() == true && (t.endsWith('.') || t.endsWith('?')) && t.length <= 400
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
    cameFrom: Overlay?,
    core: CoreClient,
    platform: AndroidPlatform,
    prefs: Prefs,
    uiPrefs: Prefs.UiPrefs,
    power: tv.lounge.power.PowerSetup,
    updates: tv.lounge.update.UpdateManager,
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
    /** Takes a sheet that no longer has anything to show out of the way back. */
    forget: (Overlay) -> Unit,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val updateState by updates.state.collectAsStateWithLifecycle()
    val updateAuto by updates.autoCheck.collectAsStateWithLifecycle()
    when (overlay) {
        Overlay.QuickSettings -> {
            val backdrops by produceState(emptyList<ShaderStyle>()) { value = withContext(Dispatchers.IO) { ShaderStyles.all(context, core) } }
            val liveBackdrops = LoungeTheme.visuals.shaderWallpaper && !LoungeTheme.motion.reducedMotion
            // Both wait on the core, which may still be loading, so they are read off the main thread.
            val enabledChannels by produceState(0) { value = withContext(Dispatchers.IO) { core.availableChannels().count { it.enabled } } }
            val enabledPlugins by produceState(0) { value = withContext(Dispatchers.IO) { core.plugins().count { it.enabled } } }
            var recentApps by remember { mutableStateOf(-1) }
            // Read again with each snapshot, so a change made on the phone shows here too.
            LaunchedEffect(Unit) { core.snapshots.collect { recentApps = withContext(Dispatchers.IO) { core.recentApps().toInt() } } }
            val rowsSummary by produceState("") {
                value = withContext(Dispatchers.IO) {
                    // Only rows the person hid count.
                    val hidden = core.homeRows().count { !it.shown && it.chosen }
                    if (hidden == 0) "All shown" else if (hidden == 1) "1 hidden" else "$hidden hidden"
                }
            }
            QuickSettingsSheet(
                prefs = prefs,
                theme = theme,
                sounds = sounds,
                lite = lite,
                reducedMotion = reduced,
                showMobileApps = showMobile,
                isDefaultHome = isDefaultHome,
                enabledChannels = enabledChannels,
                enabledPlugins = enabledPlugins,
                diagnostics = { core.diagnostics() },
                onSetTheme = { t -> scope.launch { prefs.setTheme(t) } },
                onSetSounds = { on -> scope.launch { prefs.setSounds(on) } },
                onSetLite = { on -> scope.launch { prefs.setLiteMode(on) } },
                onSetReducedMotion = { on -> scope.launch { prefs.setReducedMotion(on) } },
                onSetShowMobileApps = { on -> scope.launch { prefs.setShowMobileApps(on) } },
                // Settings stays open: Android's dialog comes up over it, and the rows read again on return.
                onRequestPermission = { name -> scope.launch { withContext(Dispatchers.IO) { core.perform("permission:$name") } } },
                say = say,
                onChannels = { open(Overlay.Channels) },
                onRows = { open(Overlay.Rows) },
                rowsSummary = rowsSummary,
                recentApps = if (recentApps < 0) 5 else recentApps,
                onSetRecentApps = { n -> recentApps = n; core.command(LayoutCommand.SetRecentApps(n.toUInt())) },
                onPlugins = { open(Overlay.Plugins) },
                onCompanion = { open(Overlay.Companion) },
                companionOn = uiPrefs.companionEnabled,
                onRecheckHome = { onHomeChanged(); open(Overlay.HomeRole) },
                onPowerSetup = { open(Overlay.PowerSetup) },
                powerStatus = powerStatus,
                onAmbient = { open(Overlay.Ambient) },
                onSetupAgain = { open(Overlay.Onboarding) },
                screensaver = { screensaverName(power, context) },
                onUpdates = { open(Overlay.Updates) },
                updateStatus = updateStatusWord(updateState, updateAuto),
                firstFocus = first,
                cameFrom = cameFrom,
                backdrop = uiPrefs.backdrop,
                backdrops = backdrops,
                liveBackdrops = liveBackdrops,
                onSetBackdrop = { id -> scope.launch { prefs.setBackdrop(id) } },
            )
        }
        Overlay.Updates -> UpdatesSheet(updates = updates, firstFocus = first)
        Overlay.Ambient -> AmbientSheet(
            power = power,
            prefs = prefs,
            core = core,
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
            var channels by remember { mutableStateOf<List<tv.lounge.core.ChannelInfo>?>(null) }
            LaunchedEffect(Unit) { channels = withContext(Dispatchers.IO) { core.availableChannels() } }
            ChannelsSheet(
                channels = channels,
                onToggle = { ch ->
                    core.command(if (ch.enabled) LayoutCommand.DisableChannel(ch.id) else LayoutCommand.EnableChannel(ch.id))
                    channels = channels?.map { if (it.id == ch.id) it.copy(enabled = !it.enabled) else it }
                },
                firstFocus = first,
            )
        }
        Overlay.Rows -> {
            var rows by remember { mutableStateOf<List<tv.lounge.core.RowInfo>?>(null) }
            var hidden by remember { mutableStateOf<List<tv.lounge.core.ItemRef>?>(null) }
            var sources by remember { mutableStateOf<List<tv.lounge.core.SourceInfo>?>(null) }
            // Each change is shown at once; a read that began before a later change is thrown away,
            // so it cannot put the older state back on screen.
            val edits = remember { intArrayOf(0) }
            suspend fun reload() {
                val began = edits[0]
                val (r, h, s) = withContext(Dispatchers.IO) { Triple(core.homeRows(), core.hiddenItems(), core.continueSources()) }
                if (edits[0] == began) {
                    rows = r
                    hidden = h
                    sources = s
                }
            }
            // A change that alters nothing on screen publishes no snapshot, so the list is also
            // read again a moment after each command has had its turn.
            LaunchedEffect(Unit) { core.snapshots.collect { reload() } }
            fun edited() {
                edits[0]++
                scope.launch { kotlinx.coroutines.delay(400); reload() }
            }
            RowsSheet(
                rows = rows,
                hidden = hidden,
                sources = sources,
                onToggle = { row ->
                    val shown = rows?.firstOrNull { it.id == row.id }?.shown ?: row.shown
                    edited()
                    core.command(if (shown) LayoutCommand.HideRow(row.id) else LayoutCommand.ShowRow(row.id))
                    rows = rows?.map { if (it.id == row.id) it.copy(shown = !shown, chosen = true) else it }
                },
                onMove = { row, to ->
                    edited()
                    core.command(LayoutCommand.MoveRow(row.id, to.toUInt()))
                    rows = rows?.let { list ->
                        val moved = list.filter { it.id != row.id }.toMutableList()
                        moved.add(to.coerceIn(0, moved.size), row)
                        moved
                    }
                },
                onResetOrder = {
                    edited()
                    core.command(LayoutCommand.SetRowOrder(emptyList()))
                },
                onShowItem = { item ->
                    edited()
                    core.command(LayoutCommand.UnhideItem(item.id))
                    hidden = hidden?.filter { it.id != item.id }
                },
                onToggleSource = { source ->
                    val shown = sources?.firstOrNull { it.id == source.id }?.shown ?: source.shown
                    edited()
                    core.command(if (shown) LayoutCommand.MuteProvider(source.id) else LayoutCommand.UnmuteProvider(source.id))
                    // One that brings nothing is only listed while it is hidden.
                    sources = sources?.filter { it.id != source.id || shown || it.count > 0u }?.map { if (it.id == source.id) it.copy(shown = !shown) else it }
                },
                firstFocus = first,
            )
        }
        Overlay.Plugins -> PluginsSheet(core = core, onOpen = { id -> open(Overlay.PluginDetail(id)) }, firstFocus = first, cameFrom = cameFrom)
        is Overlay.PluginDetail -> PluginDetailSheet(
            core = core,
            pluginId = overlay.id,
            // The page of a removed plugin has nothing to go back to.
            onRemoved = { open(Overlay.Plugins); forget(overlay) },
            firstFocus = first,
        )
        Overlay.HomeRole -> {
            val takeHome by produceState(emptyList<tv.lounge.power.PowerAction>(), isDefaultHome) { value = withContext(Dispatchers.IO) { power.takeHome() } }
            val homesOff by produceState(emptyList<String>(), isDefaultHome) { value = withContext(Dispatchers.IO) { power.homesOff().map { it.label } } }
            HomeRoleSheet(
                isDefaultHome = isDefaultHome,
                packageName = context.packageName,
                powerReady = powerStatus is tv.lounge.power.PowerStatus.Ready,
                takeHome = takeHome,
                homesOff = homesOff,
                onDoItNow = {
                    scope.launch {
                        runCatching { power.runAll(takeHome.ifEmpty { listOf(power.homeRole) }) }
                            .onSuccess { if (!isDefaultHome(context)) say(HOME_NOT_TAKEN) }
                            .onFailure { say((it as? tv.lounge.power.PrivilegedException)?.message ?: HOME_NOT_TAKEN) }
                        onHomeChanged()
                    }
                },
                onPowerSetup = { open(Overlay.PowerSetup) },
                onRecheck = onHomeChanged,
                firstFocus = first,
            )
        }
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
    onStart: () -> Unit,
) {
    val uiSounds = LoungeTheme.sounds
    val motion = LoungeTheme.motion
    val expanded = state.stageExpanded
    val height = animateDpAsState(if (expanded) StageExpandedHeight else StageCollapsedHeight, motion.proscenium(), label = "stageHeight")
    SideEffect { state.stageHeight = height }
    val closing: () -> Float = remember(state) { { state.bandRise?.value ?: 0f } }
    val light = LocalLightState.current
    LaunchedEffect(height) { snapshotFlow { height.value }.collect { light.sourceMoved() } }
    // Once the viewer has moved along the strip, the selection follows its tile's id, so a snapshot
    // that reorders the strip keeps the playbill on the focused tile; the last index stands in once
    // the tile is gone. Before that it is the first tile.
    val strip = rememberUpdatedState(snapshot.stage.strip)
    val selected = remember {
        derivedStateOf {
            val tiles = strip.value
            val id = state.stageTileId
            val byId = if (id == null) -1 else tiles.indexOfFirst { it.id == id }
            when {
                !state.stageChosen -> 0
                byId >= 0 -> byId
                else -> state.stageIndex.coerceIn(0, (tiles.size - 1).coerceAtLeast(0))
            }
        }
    }
    SideEffect { state.stageIndex = selected.value }
    StageView(
        stage = snapshot.stage,
        selectedIndex = selected.value,
        selected = remember(selected) { object : androidx.compose.runtime.IntState { override val intValue: Int get() = selected.value } },
        expanded = expanded,
        height = height,
        closing = closing,
        indexed = snapshot.revision > 0uL,
        stripRequester = state::stripRequester,
        stripFocus = { state.stripFocus() },
        playbillFocus = state.playbillFocus,
        upFocus = state.topBarFocus,
        onPlaybillFocusChanged = { state.playbillFocused = it },
        onStripFocused = { index, tile ->
            if (state.stageTileId != tile.id || state.focusedRowId != null) uiSounds?.tick()
            state.stageTileId = tile.id
            state.stageIndex = index
            state.focusedRowId = null
        },
        onStripClick = onPerform,
        onStripLongPress = onMenu,
        onStripMoved = { state.stageChosen = true },
        onPrimary = onPrimary,
        onSecondary = onSecondary,
        onBandFocused = onBandFocused,
        onStart = onStart,
    )
}
