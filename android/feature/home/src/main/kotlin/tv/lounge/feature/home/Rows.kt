package tv.lounge.feature.home

import androidx.compose.animation.core.Animatable
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.focusGroup
import androidx.compose.foundation.gestures.LocalBringIntoViewSpec
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRestorer
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.layout.LocalPinnableContainer
import androidx.compose.ui.layout.PinnableContainer
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import android.hardware.input.InputManager
import android.view.InputDevice
import androidx.tv.material3.Icon
import androidx.tv.material3.Text
import kotlinx.coroutines.launch
import tv.lounge.core.Badge
import tv.lounge.core.Row as CoreRow
import tv.lounge.core.Tile
import tv.lounge.core.TileKind
import tv.lounge.design.Dimens
import tv.lounge.design.Eased
import tv.lounge.design.Lights
import tv.lounge.design.LiveDot
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme
import tv.lounge.design.glowRoom
import tv.lounge.design.lightState

/**
 * Keeps a list that is showing its start showing it when items arrive ahead of its first one.
 * Left alone, a lazy list holds on to its first visible item and the new ones open off screen.
 */
@Composable
internal fun KeepStart(state: LazyListState, firstKey: Any?) {
    val shown = remember(state) { arrayOf(firstKey) }
    // Runs before the list measures the new items, so the position read here is still the old one.
    SideEffect {
        if (firstKey != shown[0] && shown[0] != null && state.firstVisibleItemIndex == 0 && state.firstVisibleItemScrollOffset == 0) {
            state.requestScrollToItem(0)
        }
        shown[0] = firstKey
    }
}

/**
 * Keeps a lazy list's item composed and laid out from its first measure on, so coming back to it
 * builds nothing. An item built ahead of time is measured last, so it is finished before it is
 * kept. For the item's root, from the item's own content.
 */
@Composable
internal fun keptOnceMeasured(): Modifier {
    val container = LocalPinnableContainer.current ?: return Modifier
    val pin = remember(container) { arrayOfNulls<PinnableContainer.PinnedHandle>(1) }
    DisposableEffect(container) {
        onDispose {
            pin[0]?.release()
            pin[0] = null
        }
    }
    return Modifier.onSizeChanged { if (pin[0] == null) pin[0] = container.pin() }
}

/**
 * A content row: a header with an optional badge and a pivot-scrolling tile list that remembers its
 * focused child (DESIGN.md §5.3). The header follows the light, the list keeps glow room, the
 * focused tile nudges at either end, and the room dims while a menu is open on [heldTileId] in
 * the row [heldRowId]. Both are read inside the tiles' light, so a menu opening or closing
 * recomposes only the tile it belongs to. [shown] is false while the row is kept off screen, and
 * its tiles then follow no light.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun ContentRow(
    row: CoreRow,
    isFocusedRow: () -> Boolean,
    listState: LazyListState = rememberLazyListState(),
    requesterFor: (Int) -> FocusRequester,
    onTileFocused: (Tile, Int, Rect) -> Unit,
    onTileClick: (Tile) -> Unit,
    onTileLongPress: (Tile, Rect, FocusRequester) -> Unit,
    modifier: Modifier = Modifier,
    heldTileId: () -> String? = { null },
    heldRowId: () -> String? = { null },
    shown: () -> Boolean = { true },
) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    val scope = rememberCoroutineScope()
    // Followed by id, so a tile the row re-sorts keeps its neighbours' light and its nudge.
    val focusedId = remember(row.id) { mutableStateOf<String?>(null) }
    val tiles = rememberUpdatedState(row.tiles)
    val focusedIndex = remember(row.id) { derivedStateOf { tiles.value.indexOfFirst { it.id == focusedId.value } } }
    val focusedRow = remember(isFocusedRow) { derivedStateOf { isFocusedRow() } }
    val size = tileSize(row.kind, row.size)
    val safeXPx = with(LocalDensity.current) { Dimens.safeX.toPx() }
    // The header follows the light without recomposing the row: its alpha eases as the layer reads it.
    val light = LoungeTheme.lightState
    val headerAlpha = remember { Eased(Snapshot.withoutReadObservation { if (focusedRow.value) 1f else 0.62f }) }
    val nudge = remember { Animatable(0f) }
    val heldTile = rememberUpdatedState(heldTileId)
    val heldRow = rememberUpdatedState(heldRowId)
    val roomDim = remember { derivedStateOf { heldTile.value() != null } }
    KeepStart(listState, row.tiles.firstOrNull()?.id)

    val nudgePx = 6.dp.value * (safeXPx / Dimens.safeX.value)

    fun nudgeBy(px: Float) {
        if (motion.reducedMotion) return
        scope.launch {
            nudge.animateTo(px, motion.snap())
            nudge.animateTo(0f, motion.settle())
        }
    }

    Column(modifier.fillMaxWidth()) {
        Row(
            Modifier
                .padding(start = Dimens.safeX)
                .height(Dimens.rowHeaderHeight)
                .graphicsLayer {
                    alpha = headerAlpha.toward(if (focusedRow.value) 1f else 0.62f, light, motion.settleMs, motion.settleEasing)
                    // Nothing in a header overlaps, so its alpha needs no layer of its own.
                    compositingStrategy = CompositingStrategy.ModulateAlpha
                },
            verticalAlignment = Alignment.Top,
            horizontalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Text(row.title, style = LoungeTheme.typography.headline, color = colors.limelight, modifier = Modifier.alignByBaseline())
            when (row.badge) {
                Badge.LIVE -> RowLiveDot(Modifier.padding(top = 15.dp))
                Badge.CONTROLLER -> ControllerGlyph(Modifier.padding(top = 5.dp).size(30.dp))
                else -> Unit
            }
            // Beside the header, so the row's height and the focus line stay as they are.
            row.note?.let { Text(it, style = LoungeTheme.typography.meta, color = colors.dust, maxLines = 1, modifier = Modifier.alignByBaseline()) }
        }
        CompositionLocalProvider(LocalBringIntoViewSpec provides pivotSpec(safeXPx)) {
            LazyRow(
                state = listState,
                contentPadding = PaddingValues(start = Dimens.safeX, end = Dimens.safeX, top = Dimens.glowRoom, bottom = Dimens.glowRoom),
                horizontalArrangement = Arrangement.spacedBy(Dimens.gutter),
                modifier = Modifier
                    .fillMaxWidth()
                    .glowRoom()
                    .onPreviewKeyEvent { e ->
                        if (e.type == KeyEventType.KeyDown && e.nativeKeyEvent.repeatCount == 0) {
                            if (e.key == Key.DirectionLeft && focusedIndex.value == 0) nudgeBy(-nudgePx)
                            if (e.key == Key.DirectionRight && focusedIndex.value == row.tiles.lastIndex) nudgeBy(nudgePx)
                        }
                        false
                    }
                    .focusRestorer()
                    .focusGroup(),
            ) {
                itemsIndexed(row.tiles, key = { _, t -> t.id }) { index, tile ->
                    // The same app can sit in several rows; only the one the menu came from is held.
                    val held = remember(tile.id, row.id) { derivedStateOf { heldTile.value() == tile.id && heldRow.value() == row.id } }
                    // Each tile's light is a derived State: moving focus along the row, or the room
                    // dimming for a menu, changes brightness without recomposing a single tile.
                    val lights = remember(index) {
                        derivedStateOf {
                            val f = focusedIndex.value
                            when {
                                roomDim.value -> Lights.Dimmed
                                !focusedRow.value -> Lights.OtherRow
                                f == -1 -> Lights.Rested
                                kotlin.math.abs(index - f) == 1 -> Lights.Neighbour
                                else -> Lights.Rested
                            }
                        }
                    }
                    val nudgeRead: () -> Float = remember(index) { { if (focusedIndex.value == index) nudge.value else 0f } }
                    Column {
                        TileView(
                            tile = tile,
                            kind = row.kind,
                            size = size,
                            lights = lights,
                            rowName = row.title,
                            focusRequester = requesterFor(index),
                            onFocused = { focused, bounds ->
                                if (focused) {
                                    focusedId.value = tile.id
                                    onTileFocused(tile, index, bounds)
                                }
                            },
                            // A tile to read answers OK with the nudge that says there is no further to go.
                            onClick = { if (tile.action.isEmpty()) nudgeBy(nudgePx) else onTileClick(tile) },
                            onLongPress = { bounds -> onTileLongPress(tile, bounds, requesterFor(index)) },
                            held = held.value,
                            nudge = nudgeRead,
                            shown = shown,
                            // A badge the row wears by its title is not repeated on every tile.
                            badged = tile.badge != row.badge,
                        )
                        if (row.kind == TileKind.POSTER) {
                            val showTitle = remember(index) { derivedStateOf { focusedRow.value && focusedIndex.value == index } }
                            FocusedTitle(visible = showTitle.value, title = tile.title, width = size.width)
                        }
                    }
                }
            }
        }
        Spacer(Modifier.height(Dimens.rowGap))
    }
}

/** A row header's live dot, reading idle on its own so the row around it never recomposes for it. */
@Composable
private fun RowLiveDot(modifier: Modifier) = LiveDot(LoungeTheme.lightState.idle, modifier)

/** The Games header's controller glyph, there only while a controller is connected (DESIGN.md §5.3). */
@Composable
private fun ControllerGlyph(modifier: Modifier) {
    if (rememberControllerConnected()) {
        Icon(LoungeIcons.Controller, contentDescription = "Controller connected", tint = LoungeTheme.colors.dust, modifier = modifier)
    }
}

/** Whether a game controller is connected, followed as controllers come and go. */
@Composable
internal fun rememberControllerConnected(): Boolean {
    val context = LocalContext.current
    var connected by remember { mutableStateOf(false) }
    DisposableEffect(context) {
        val input = context.getSystemService(InputManager::class.java)
        fun check() {
            connected = InputDevice.getDeviceIds().any { id -> InputDevice.getDevice(id)?.let(::isController) == true }
        }
        val listener = object : InputManager.InputDeviceListener {
            override fun onInputDeviceAdded(deviceId: Int) = check()
            override fun onInputDeviceRemoved(deviceId: Int) = check()
            override fun onInputDeviceChanged(deviceId: Int) = check()
        }
        check()
        input?.registerInputDeviceListener(listener, null)
        onDispose { input?.unregisterInputDeviceListener(listener) }
    }
    return connected
}

/** A gamepad with sticks; a TV remote with game buttons is not one. */
private fun isController(device: InputDevice): Boolean =
    !device.isVirtual &&
        (device.sources and InputDevice.SOURCE_GAMEPAD) == InputDevice.SOURCE_GAMEPAD &&
        (device.sources and InputDevice.SOURCE_JOYSTICK) == InputDevice.SOURCE_JOYSTICK
