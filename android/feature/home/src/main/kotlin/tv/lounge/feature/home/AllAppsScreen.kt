package tv.lounge.feature.home

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.focusGroup
import androidx.compose.foundation.focusable
import androidx.compose.foundation.gestures.LocalBringIntoViewSpec
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.itemsIndexed
import androidx.compose.foundation.lazy.grid.rememberLazyGridState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.tv.material3.Icon
import androidx.tv.material3.Text
import kotlinx.coroutines.launch
import tv.lounge.core.RowSize
import tv.lounge.core.Tile
import tv.lounge.core.TileKind
import tv.lounge.design.Dimens
import tv.lounge.design.Lights
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme
import tv.lounge.design.glowRoom
import tv.lounge.design.sounds

/**
 * Every app, A to Z, as a grid of banners with an alphabet down the left to jump by
 * (DESIGN.md §5.5). Left from the first column reaches the alphabet; Up and Down on it scroll the
 * grid a letter at a time; Right or OK drops into that letter's first app. Holding OK on an app
 * opens the same menu as on the home, and a hidden or pinned app moves out from under focus.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun AllAppsScreen(
    apps: List<Tile>,
    visibleState: MutableTransitionState<Boolean>,
    covered: Boolean,
    heldTileId: () -> String?,
    onTileClick: (Tile) -> Unit,
    onTileLongPress: (Tile, Rect, FocusRequester) -> Unit,
    onClose: () -> Unit,
) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    val sounds = LoungeTheme.sounds
    val scope = rememberCoroutineScope()
    val leaving = !visibleState.targetState
    val gridState = rememberLazyGridState()
    val size = tileSize(TileKind.APP_BANNER, RowSize.S)

    val requesters = remember { ArrayList<FocusRequester>() }
    fun requester(index: Int): FocusRequester {
        while (requesters.size <= index) requesters += FocusRequester()
        return requesters[index]
    }
    val letterRequesters = remember { HashMap<Char, FocusRequester>() }
    fun letterRequester(c: Char) = letterRequesters.getOrPut(c) { FocusRequester() }

    val letters = remember(apps) { apps.map { letterOf(it.title) } }
    val firstOfLetter = remember(letters) { letters.withIndex().groupBy({ it.value }, { it.index }).mapValues { it.value.first() } }
    var focusedIndex by remember { mutableIntStateOf(-1) }
    var focusedTileId by remember { mutableStateOf<String?>(null) }
    var focusedLetter by remember { mutableStateOf<Char?>(null) }
    // The letter the grid is at, or the one chosen on the alphabet.
    val currentLetter = focusedLetter ?: letters.getOrNull(focusedIndex)
    val held = rememberUpdatedState(heldTileId)
    val roomDim = remember { derivedStateOf { held.value() != null } }

    // An app hidden or pinned from its menu leaves its place: focus goes to the tile that took
    // it, or the last one.
    LaunchedEffect(apps) {
        val id = focusedTileId ?: return@LaunchedEffect
        if (leaving || covered || apps.any { it.id == id }) return@LaunchedEffect
        if (apps.isEmpty()) return@LaunchedEffect
        requester(focusedIndex.coerceIn(0, apps.lastIndex)).requestWhenReady()
    }

    val rise = with(LocalDensity.current) { 10.dp.roundToPx() }
    val pivotPx = with(LocalDensity.current) { Dimens.glowRoom.toPx() }

    AnimatedVisibility(visibleState, enter = fadeIn(tween(180)), exit = fadeOut(motion.settle())) {
        Box(
            Modifier
                .fillMaxSize()
                .background(colors.velvet)
                .onPreviewKeyEvent { e -> if (e.isBack() && e.type == KeyEventType.KeyUp) { onClose(); true } else false }
                .trapFocus(leaving)
                .focusGroup()
                .focusProperties { if (leaving) canFocus = false },
        ) {
            // The grid comes first so the heading paints over it: scrolled rows pass under a fade
            // rather than through the words.
            Row(Modifier.fillMaxSize().padding(start = Dimens.safeX, end = Dimens.safeX, top = HeaderHeight + FadeHeight)) {
                // The alphabet: letters with apps are live, the one the grid is at glows.
                Column(Modifier.width(RailWidth).focusGroup(), horizontalAlignment = Alignment.Start) {
                    for (c in ALPHABET) {
                        val first = firstOfLetter[c]
                        val live = first != null
                        var letterFocused by remember { mutableStateOf(false) }
                        Box(
                            Modifier
                                .size(40.dp, 30.dp)
                                .then(
                                    if (live) {
                                        Modifier
                                            .focusRequester(letterRequester(c))
                                            .onFocusChanged {
                                                letterFocused = it.isFocused
                                                if (it.isFocused) {
                                                    if (focusedLetter != c) sounds?.tick()
                                                    focusedLetter = c
                                                    scope.launch { gridState.animateScrollToItem(first!!) }
                                                }
                                            }
                                            .onPreviewKeyEvent { e ->
                                                val enter = e.key == Key.DirectionRight || e.key == Key.DirectionCenter || e.key == Key.Enter
                                                if (enter && e.type == KeyEventType.KeyDown) {
                                                    requester(first!!).requestFocus()
                                                    true
                                                } else {
                                                    false
                                                }
                                            }
                                            .focusable()
                                            .semantics { contentDescription = "Apps starting with $c" }
                                    } else {
                                        Modifier
                                    },
                                )
                                .background(if (letterFocused) colors.curtainEdge else colors.velvet.copy(alpha = 0f), CircleShape),
                            contentAlignment = Alignment.Center,
                        ) {
                            Text(
                                c.toString(),
                                style = LoungeTheme.typography.caption,
                                color = when {
                                    letterFocused -> colors.limelight
                                    c == currentLetter -> colors.houselight
                                    else -> colors.dust
                                },
                                textAlign = TextAlign.Center,
                                modifier = Modifier.alpha(if (live) 1f else 0.3f),
                            )
                        }
                    }
                }
                if (apps.isEmpty()) {
                    // The trap needs something to hold, and Back needs somewhere to arrive from.
                    Box(Modifier.size(1.dp).focusRequester(requester(0)).focusable())
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                        Icon(LoungeIcons.Apps, contentDescription = null, tint = colors.dust, modifier = Modifier.size(32.dp))
                        Text("Apps appear here once they are installed.", style = LoungeTheme.typography.meta, color = colors.dust)
                    }
                } else {
                    // The focused row sits at the top of the grid, so moving down never hunts.
                    CompositionLocalProvider(LocalBringIntoViewSpec provides pivotSpec(pivotPx)) {
                        LazyVerticalGrid(
                            columns = GridCells.Fixed(COLUMNS),
                            state = gridState,
                            horizontalArrangement = Arrangement.spacedBy(Dimens.gutter),
                            verticalArrangement = Arrangement.spacedBy(Dimens.gutter),
                            contentPadding = PaddingValues(top = Dimens.glowRoom, bottom = 420.dp),
                            modifier = Modifier
                                .fillMaxSize()
                                .glowRoom()
                                .focusGroup(),
                        ) {
                            itemsIndexed(apps, key = { _, t -> t.id }) { index, tile ->
                                val heldHere = remember(tile.id) { derivedStateOf { held.value() == tile.id } }
                                val lights = remember { derivedStateOf { if (roomDim.value) Lights.Dimmed else Lights.Rested } }
                                TileView(
                                    tile = tile,
                                    kind = TileKind.APP_BANNER,
                                    size = size,
                                    lights = lights,
                                    rowName = "All apps",
                                    focusRequester = requester(index),
                                    onFocused = { focused, _ ->
                                        if (focused) {
                                            if (focusedIndex != index) sounds?.tick()
                                            focusedIndex = index
                                            focusedTileId = tile.id
                                            focusedLetter = null
                                        }
                                    },
                                    onClick = { onTileClick(tile) },
                                    onLongPress = { bounds -> onTileLongPress(tile, bounds, requester(index)) },
                                    held = heldHere.value,
                                    // Left from the first column lands on the alphabet at this app's letter.
                                    modifier = Modifier.onPreviewKeyEvent { e ->
                                        if (index % COLUMNS == 0 && e.key == Key.DirectionLeft && e.type == KeyEventType.KeyDown) {
                                            letterRequester(letters[index]).requestFocus()
                                            true
                                        } else {
                                            false
                                        }
                                    },
                                )
                            }
                        }
                    }
                }
            }
            AnimatedVisibility(
                visibleState,
                enter = fadeIn(tween(180, delayMillis = 60)) + slideInVertically(tween(180, delayMillis = 60)) { rise },
                exit = fadeOut(motion.settle()),
            ) {
                Column(Modifier.fillMaxWidth()) {
                    Column(
                        Modifier.fillMaxWidth().height(HeaderHeight).background(colors.velvet).padding(start = Dimens.safeX, end = Dimens.safeX, top = Dimens.safeY),
                        verticalArrangement = Arrangement.spacedBy(10.dp),
                    ) {
                        Text("All apps", style = LoungeTheme.typography.displayL, color = colors.limelight)
                        Text(
                            when (apps.size) {
                                0 -> "Nothing here yet."
                                1 -> "1 app. Hold OK on it to pin it to Your apps or hide it."
                                else -> "${apps.size} apps, A to Z. Hold OK on one to pin it to Your apps or hide it. Left of the first column, the alphabet jumps."
                            },
                            style = LoungeTheme.typography.meta,
                            color = colors.dust,
                        )
                    }
                    Box(Modifier.fillMaxWidth().height(FadeHeight).background(Brush.verticalGradient(0f to colors.velvet, 1f to colors.velvet.copy(alpha = 0f))))
                }
            }
        }
    }
    LaunchedEffect(Unit) { requester(0).requestWhenReady() }
}

private const val COLUMNS = 6
private val RailWidth = 72.dp
private val HeaderHeight = 150.dp
private val FadeHeight = 40.dp
private val ALPHABET: List<Char> = listOf('#') + ('A'..'Z')

/** The alphabet entry a title files under: its first letter, or # for a digit or symbol. */
private fun letterOf(title: String): Char {
    val c = title.trim().firstOrNull()?.uppercaseChar() ?: '#'
    return if (c in 'A'..'Z') c else '#'
}
