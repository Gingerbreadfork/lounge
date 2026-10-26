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
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRestorer
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import androidx.tv.material3.Icon
import androidx.tv.material3.Text
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.launch
import tv.lounge.core.Badge
import tv.lounge.core.Row as CoreRow
import tv.lounge.core.Tile
import tv.lounge.core.TileKind
import tv.lounge.design.Dimens
import tv.lounge.design.Lights
import tv.lounge.design.LiveDot
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme
import tv.lounge.design.glowRoom

/**
 * A content row: a header with an optional badge and a pivot-scrolling tile list that remembers its
 * focused child (DESIGN.md §5.3). The header follows the light, the list keeps glow room, the
 * focused tile nudges at either end, and the room dims while a menu is open on [heldTileId] in
 * the row [heldRowId].
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun ContentRow(
    row: CoreRow,
    isFocusedRow: () -> Boolean,
    idle: Boolean,
    listState: LazyListState = rememberLazyListState(),
    requesterFor: (Int) -> FocusRequester,
    onTileFocused: (Tile, Int, Rect) -> Unit,
    onTileClick: (Tile) -> Unit,
    onTileLongPress: (Tile, Rect, FocusRequester) -> Unit,
    modifier: Modifier = Modifier,
    heldTileId: String? = null,
    heldRowId: String? = null,
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
    // The header follows the light without recomposing the row: an Animatable fed by the state.
    val headerAlpha = remember { Animatable(if (focusedRow.value) 1f else 0.62f) }
    LaunchedEffect(focusedRow) { snapshotFlow { focusedRow.value }.collectLatest { headerAlpha.animateTo(if (it) 1f else 0.62f, motion.settle()) } }
    val nudge = remember { Animatable(0f) }
    val roomDim = heldTileId != null

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
                .graphicsLayer { alpha = headerAlpha.value },
            verticalAlignment = Alignment.Top,
            horizontalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Text(row.title, style = LoungeTheme.typography.headline, color = colors.limelight)
            when (row.badge) {
                Badge.LIVE -> LiveDot(idle, Modifier.padding(top = 15.dp))
                Badge.CONTROLLER -> Icon(LoungeIcons.Controller, contentDescription = "Controller connected", tint = colors.dust, modifier = Modifier.padding(top = 5.dp).size(30.dp))
                else -> Unit
            }
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
                            val px = 6.dp.value * (safeXPx / Dimens.safeX.value)
                            if (e.key == Key.DirectionLeft && focusedIndex.value == 0) nudgeBy(-px)
                            if (e.key == Key.DirectionRight && focusedIndex.value == row.tiles.lastIndex) nudgeBy(px)
                        }
                        false
                    }
                    .focusRestorer()
                    .focusGroup(),
            ) {
                itemsIndexed(row.tiles, key = { _, t -> t.id }) { index, tile ->
                    // The same app can sit in several rows; only the one the menu came from is held.
                    val held = heldTileId == tile.id && heldRowId == row.id
                    // Each tile's light is a derived State: moving focus along the row changes
                    // neighbours' brightness without recomposing a single tile.
                    val lights = remember(index, roomDim) {
                        derivedStateOf {
                            val f = focusedIndex.value
                            when {
                                roomDim -> Lights.Dimmed
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
                            idle = idle,
                            rowName = row.title,
                            focusRequester = requesterFor(index),
                            onFocused = { focused, bounds ->
                                if (focused) {
                                    focusedId.value = tile.id
                                    onTileFocused(tile, index, bounds)
                                }
                            },
                            onClick = { onTileClick(tile) },
                            onLongPress = { bounds -> onTileLongPress(tile, bounds, requesterFor(index)) },
                            held = held,
                            nudge = nudgeRead,
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
