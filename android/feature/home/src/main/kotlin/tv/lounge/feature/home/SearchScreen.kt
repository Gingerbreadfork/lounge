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
import androidx.compose.runtime.rememberUpdatedState
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
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import androidx.tv.material3.Icon
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
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
 * recently opened row fills the space. It searches everything in the snapshot, and the search
 * plugins answer a moment later with a row each.
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
    /** A query from the phone, handed back through [onQueryTaken] once it is in the field. */
    pendingQuery: String? = null,
    onQueryTaken: () -> Unit = {},
    /** A tile's menu sits over the screen. */
    covered: Boolean = false,
    heldTileId: String? = null,
    /** Rows from the search plugins for a query; may take seconds. */
    searchPlugins: suspend (String) -> List<CoreRow> = { emptyList() },
) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    var field by rememberSaveable(stateSaver = TextFieldValue.Saver) { mutableStateOf(TextFieldValue(pendingQuery.orEmpty(), TextRange(pendingQuery.orEmpty().length))) }
    val query = field.text
    val fieldFocus = remember { FocusRequester() }
    var fieldFocused by remember { mutableStateOf(false) }
    var focusedRow by remember { mutableStateOf<String?>(null) }
    val lastResult = remember { LastResult() }
    val requesters = remember { HashMap<String, ArrayList<FocusRequester>>() }
    fun requester(rowId: String, index: Int): FocusRequester {
        val list = requesters.getOrPut(rowId) { ArrayList() }
        while (list.size <= index) list += FocusRequester()
        return list[index]
    }
    val leaving = !visibleState.targetState
    val interactive = rememberUpdatedState(!leaving && !covered)

    fun setQuery(text: String) {
        field = TextFieldValue(text, TextRange(text.length))
    }

    LaunchedEffect(pendingQuery) {
        val q = pendingQuery ?: return@LaunchedEffect
        setQuery(q)
        onQueryTaken()
        fieldFocus.requestWhenReady()
    }
    PhoneTextInput(active = !leaving && !covered) { text -> setQuery(field.text + text) }

    val rise = with(androidx.compose.ui.platform.LocalDensity.current) { 10.dp.roundToPx() }
    val local: List<CoreRow> = remember(query, snapshot) { search(snapshot, query) }
    // The plugins are asked once typing pauses; their last answer stays until the next arrives.
    var fromPlugins by remember { mutableStateOf(emptyList<CoreRow>()) }
    LaunchedEffect(query) {
        val q = query.trim()
        if (q.length < 2) {
            fromPlugins = emptyList()
            return@LaunchedEffect
        }
        delay(PLUGIN_SEARCH_PAUSE_MS)
        fromPlugins = withContext(Dispatchers.IO) { searchPlugins(q) }
    }
    val results: List<CoreRow> = remember(local, fromPlugins) { local + fromPlugins.filter { p -> local.none { it.id == p.id } } }
    val recent: List<CoreRow> = remember(snapshot) { recentlyOpened(snapshot) }
    val shown = if (query.isBlank()) recent else results

    // A result hidden or unpinned from its menu leaves the list: focus goes to its neighbour, or
    // the row that took its place, or the field.
    val shownIds = remember { arrayOf(emptyList<String>()) }
    LaunchedEffect(shown) {
        val before = shownIds[0]
        shownIds[0] = shown.map { it.id }
        val id = lastResult.tileId ?: return@LaunchedEffect
        if (!interactive.value) return@LaunchedEffect
        val row = shown.firstOrNull { it.id == lastResult.rowId }
        if (row != null && row.tiles.any { it.id == id }) return@LaunchedEffect
        val target = when {
            row != null && row.tiles.isNotEmpty() -> requester(row.id, lastResult.index.coerceIn(0, row.tiles.lastIndex))
            shown.isNotEmpty() -> requester(shown[before.indexOf(lastResult.rowId).coerceIn(0, shown.lastIndex)].id, 0)
            else -> fieldFocus
        }
        target.requestWhenReady()
    }

    AnimatedVisibility(visibleState, enter = fadeIn(tween(180)), exit = fadeOut(motion.settle())) {
        Column(
            Modifier
                .fillMaxSize()
                .background(colors.velvet)
                .onPreviewKeyEvent { e ->
                    if (e.isBack() && e.type == KeyEventType.KeyUp && !fieldFocused) { onClose(); true } else false
                }
                .trapFocus(leaving)
                .focusGroup()
                .focusProperties { if (leaving) canFocus = false },
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
                                    value = field,
                                    onValueChange = { field = it },
                                    singleLine = true,
                                    textStyle = LoungeTheme.typography.displayL.copy(color = colors.limelight),
                                    cursorBrush = SolidColor(colors.houselight),
                                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                                    keyboardActions = KeyboardActions(onSearch = { }),
                                    modifier = Modifier
                                        .fillMaxWidth()
                                        .focusRequester(fieldFocus)
                                        .onFocusChanged {
                                            fieldFocused = it.isFocused
                                            if (it.isFocused) lastResult.tileId = null
                                        },
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
                        requesterFor = { index -> requester(row.id, index) },
                        onTileFocused = { tile, index, bounds ->
                            focusedRow = row.id
                            lastResult.rowId = row.id
                            lastResult.tileId = tile.id
                            lastResult.index = index
                            onTileFocused(tile, bounds)
                        },
                        onTileClick = onTileClick,
                        // A plugin's result is not on the home, so there is nothing to pin or hide.
                        onTileLongPress = { tile, bounds, fr -> if (!tile.id.startsWith("search:")) onTileLongPress(tile, row.id, bounds, fr) },
                        heldTileId = heldTileId,
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

private const val PLUGIN_SEARCH_PAUSE_MS = 350L

/** The result that last had focus; cleared when the field takes it. */
private class LastResult {
    var rowId: String? = null
    var tileId: String? = null
    var index = 0
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
