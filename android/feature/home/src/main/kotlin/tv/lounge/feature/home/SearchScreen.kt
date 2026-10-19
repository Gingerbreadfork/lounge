package tv.lounge.feature.home

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import androidx.tv.material3.Icon
import androidx.tv.material3.Text
import tv.lounge.core.HomeSnapshot
import tv.lounge.core.Row as CoreRow
import tv.lounge.core.Tile
import tv.lounge.design.Dimens
import tv.lounge.design.Hairline
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme

/**
 * Search (DESIGN.md §5.5): the query in display type, results as ordinary rows updated per
 * keystroke. The screen fades up and results reflow rather than jump; before anything is typed the
 * recently opened row fills the space. It searches everything in the snapshot.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun SearchScreen(
    snapshot: HomeSnapshot,
    idle: Boolean,
    visibleState: MutableTransitionState<Boolean>,
    onTileFocused: (Tile, Rect) -> Unit,
    onTileClick: (Tile) -> Unit,
    onTileLongPress: (Tile, String, Rect, FocusRequester) -> Unit,
    onClose: () -> Unit,
    initialQuery: String = "",
) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    var query by rememberSaveable { mutableStateOf(initialQuery) }
    val fieldFocus = remember { FocusRequester() }
    var fieldFocused by remember { mutableStateOf(false) }
    var focusedRow by remember { mutableStateOf<String?>(null) }
    val leaving = !visibleState.targetState

    val rise = with(androidx.compose.ui.platform.LocalDensity.current) { 10.dp.roundToPx() }
    val results: List<CoreRow> = remember(query, snapshot) { search(snapshot, query) }
    val recent: List<CoreRow> = remember(snapshot) { recentlyOpened(snapshot) }
    val shown = if (query.isBlank()) recent else results

    AnimatedVisibility(visibleState, enter = fadeIn(tween(180)), exit = fadeOut(motion.settle())) {
        Column(
            Modifier
                .fillMaxSize()
                .background(colors.velvet)
                .focusProperties { canFocus = !leaving }
                .onPreviewKeyEvent { e ->
                    if (e.isBack() && e.type == KeyEventType.KeyUp && !fieldFocused) { onClose(); true } else false
                },
        ) {
            AnimatedVisibility(
                visibleState,
                enter = fadeIn(tween(180, delayMillis = 60)) + slideInVertically(tween(180, delayMillis = 60)) { rise },
                exit = fadeOut(motion.settle()),
            ) {
                Row(
                    Modifier.padding(start = Dimens.safeX, end = Dimens.safeX, top = Dimens.safeY).fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.Top,
                ) {
                    Column(Modifier.width(1100.dp), verticalArrangement = Arrangement.spacedBy(18.dp)) {
                        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(22.dp)) {
                            Icon(LoungeIcons.Search, contentDescription = null, tint = colors.dust, modifier = Modifier.size(40.dp))
                            Box(Modifier.weight(1f)) {
                                if (query.isEmpty()) {
                                    Text("Search your TV", style = LoungeTheme.typography.displayL, color = colors.dust)
                                }
                                BasicTextField(
                                    value = query,
                                    onValueChange = { query = it },
                                    singleLine = true,
                                    textStyle = LoungeTheme.typography.displayL.copy(color = colors.limelight),
                                    cursorBrush = SolidColor(colors.houselight),
                                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                                    keyboardActions = KeyboardActions(onSearch = { }),
                                    modifier = Modifier
                                        .fillMaxWidth()
                                        .focusRequester(fieldFocus)
                                        .onFocusChanged { fieldFocused = it.isFocused },
                                )
                            }
                        }
                        Hairline(Modifier.fillMaxWidth())
                        Text(
                            when {
                                query.isBlank() -> "Type with the remote. Results update as you type."
                                results.isEmpty() -> "Nothing matches \"$query\" yet."
                                else -> "Press down to open the first result."
                            },
                            style = LoungeTheme.typography.meta,
                            color = colors.dust,
                        )
                    }
                }
            }
            Spacer(Modifier.height(48.dp))
            // The keyboard takes the bottom of the screen; rows scroll above it rather than behind it.
            LazyColumn(Modifier.fillMaxSize().imePadding()) {
                items(shown, key = { it.id }) { row ->
                    ContentRow(
                        row = row,
                        isFocusedRow = { focusedRow == row.id },
                        idle = idle,
                        onTileFocused = { tile, _, bounds -> focusedRow = row.id; onTileFocused(tile, bounds) },
                        onTileClick = onTileClick,
                        onTileLongPress = { tile, bounds, fr -> onTileLongPress(tile, row.id, bounds, fr) },
                        modifier = Modifier.animateItem(
                            fadeInSpec = tween(120),
                            placementSpec = tween(motion.travelMs, easing = motion.travelEasing),
                            fadeOutSpec = tween(90),
                        ),
                    )
                }
            }
        }
    }
    LaunchedEffect(Unit) { fieldFocus.requestWhenReady() }
}

private fun search(snapshot: HomeSnapshot, query: String): List<CoreRow> {
    val q = query.trim().lowercase()
    if (q.isEmpty()) return emptyList()
    val tokens = q.split(Regex("\\s+")).filter { it.isNotEmpty() }
    fun matches(tile: Tile): Boolean {
        val hay = (tile.title + " " + (tile.subtitle ?: "")).lowercase()
        return tokens.all { hay.contains(it) }
    }
    val seen = HashSet<String>()
    return snapshot.rows.mapNotNull { row ->
        val hits = row.tiles.filter { matches(it) && seen.add(it.id) }
        if (hits.isEmpty()) null else row.copy(tiles = hits)
    }
}

/** What was opened lately: the head of Continue watching, then the apps most used, as one row each. */
private fun recentlyOpened(snapshot: HomeSnapshot): List<CoreRow> {
    val out = ArrayList<CoreRow>(2)
    snapshot.rows.firstOrNull { it.id == "continue" && it.tiles.isNotEmpty() && !it.tiles.first().id.startsWith("notice:") }?.let {
        out += it.copy(id = "recent:continue", title = "Recently watched", tiles = it.tiles.take(6))
    }
    snapshot.rows.firstOrNull { it.id == "apps" && it.tiles.isNotEmpty() }?.let {
        out += it.copy(id = "recent:apps", title = "Your apps", tiles = it.tiles.take(6))
    }
    return out
}
