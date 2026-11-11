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
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.semantics
import kotlinx.coroutines.launch
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
import tv.lounge.core.RowSize
import tv.lounge.core.TileKind
import tv.lounge.core.Tile
import tv.lounge.design.Dimens
import tv.lounge.design.Hairline
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme

/**
 * Search (DESIGN.md §5.5): the query in display type, results as ordinary rows updated per
 * keystroke. The screen fades up and results reflow rather than jump; before anything is typed
 * Continue watching and Your apps fill the space. It searches everything in the snapshot, best
 * matches first, and the search plugins answer a moment later with a row each. The keyboard's
 * search key puts the keyboard away and lands on the first result.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun SearchScreen(
    snapshot: HomeSnapshot,
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
    heldTileId: () -> String? = { null },
    heldRowId: () -> String? = { null },
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
    val keyboard = LocalSoftwareKeyboardController.current
    val scope = rememberCoroutineScope()

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

    AnimatedVisibility(visibleState, enter = fadeIn(motion.timed(180)), exit = fadeOut(motion.settle())) {
        Column(
            Modifier
                .fillMaxSize()
                .background(colors.velvet)
                .semantics { paneTitle = "Search" }
                .onPreviewKeyEvent { e ->
                    if (e.isBack() && e.type == KeyEventType.KeyUp && !fieldFocused) { onClose(); true } else false
                }
                .trapFocus(leaving)
                .focusGroup()
                .focusProperties { if (leaving) canFocus = false },
        ) {
            AnimatedVisibility(
                visibleState,
                enter = fadeIn(motion.timed(180, delayMs = 60)) + slideInVertically(motion.timed(180, delayMs = 60)) { rise },
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
                                    // The keyboard keeps Down while it is up, so its search key is the way to the results.
                                    keyboardActions = KeyboardActions(onSearch = {
                                        keyboard?.hide()
                                        shown.firstOrNull()?.let { row -> scope.launch { requester(row.id, 0).requestWhenReady() } }
                                    }),
                                    modifier = Modifier
                                        .fillMaxWidth()
                                        .focusRequester(fieldFocus)
                                        .onFocusChanged {
                                            fieldFocused = it.isFocused
                                            if (it.isFocused) lastResult.tileId = null
                                        }
                                        .semantics { contentDescription = "Search your TV" },
                                )
                            }
                        }
                        Hairline(Modifier.fillMaxWidth())
                        Text(
                            when {
                                query.isBlank() -> "Type with the remote. Results update as you type."
                                results.isEmpty() -> "Nothing matches “$query” yet."
                                else -> "The keyboard's search key, or Down, takes you to the results."
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
                        heldRowId = heldRowId,
                        modifier = Modifier.animateItem(
                            fadeInSpec = motion.timed(120),
                            placementSpec = tween(motion.travelMs, easing = motion.travelEasing),
                            fadeOutSpec = motion.timed(90),
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

/**
 * Every row the snapshot can answer from, best matches first within each, and the rows in order of
 * their best match. Your apps and every app make one row called Apps, which leads when an app's
 * name starts with what was typed. Hidden rows and hidden apps are searched too: hiding takes
 * something off the home, not out of reach.
 */
internal fun search(snapshot: HomeSnapshot, query: String): List<CoreRow> {
    val tokens = searchWords(query)
    if (tokens.isEmpty()) return emptyList()
    val home = snapshot.rows + snapshot.hiddenRows
    val apps = (home.filter { it.id == "apps" }.flatMap { it.tiles } + snapshot.allApps).distinctBy { it.id }
    val sources = buildList {
        add(CoreRow(id = "allapps", title = "Apps", kind = TileKind.APP_BANNER, size = RowSize.M, badge = null, tiles = apps, note = null))
        continueRow(snapshot, home.firstOrNull { it.id == "continue" })?.let(::add)
        addAll(home.filter { it.id != "apps" && it.id != "continue" && !it.id.startsWith("notice:") })
    }
    val seen = HashSet<String>()
    val found = sources.mapNotNull { row ->
        // A tile that only opens search or another screen would lead away from the result, and
        // one that opens nothing is no result at all.
        val hits = row.tiles
            .mapNotNull { t -> matchScore(t.title, t.subtitle, tokens)?.takeIf { t.action.isNotEmpty() && !t.action.startsWith("control:") && seen.add(t.id) }?.let { t to it } }
            .sortedByDescending { it.second }
        if (hits.isEmpty()) null else row.copy(tiles = hits.map { it.first }) to hits.first().second
    }
    val appsLead = found.firstOrNull { it.first.id == "allapps" }?.second == TITLE_START
    return found.sortedWith(compareByDescending<Pair<CoreRow, Int>> { appsLead && it.first.id == "allapps" }.thenByDescending { it.second }).map { it.first }
}

private const val TITLE_START = 3
private const val WORD_START = 2
private const val ANYWHERE = 1

private val wordBreak = Regex("[^\\p{L}\\p{N}]+")
private val episodeWord = Regex("[se]\\d+|s\\d+e\\d+")
private val accents = Regex("[\\p{Mn}'’]+")

/** Lower case without accents or apostrophes, so "pokemon" finds "Pokémon" and "greys" finds "Grey's". */
private fun plain(text: String): String = accents.replace(java.text.Normalizer.normalize(text.lowercase(), java.text.Normalizer.Form.NFD), "")

/** What was typed, as the words to look for. */
internal fun searchWords(query: String): List<String> = plain(query).split(wordBreak).filter { it.isNotEmpty() }

/**
 * How well a tile matches every word typed, or null when one matches nothing: 3 when its title
 * starts with what was typed, 2 when each word starts a word of the title, 1 when a word sits
 * inside the title or starts a word of the second line. Episode codes such as "S5" never match,
 * and a single letter never matches the second line.
 */
internal fun matchScore(title: String, subtitle: String?, tokens: List<String>): Int? {
    val t = plain(title)
    val titleWords = t.split(wordBreak).filter { it.isNotEmpty() }
    val lineWords = subtitle?.let { plain(it).split(wordBreak).filter { w -> w.isNotEmpty() && !episodeWord.matches(w) } }.orEmpty()
    if (t.startsWith(tokens.joinToString(" ")) || titleWords.joinToString(" ").startsWith(tokens.joinToString(" "))) return TITLE_START
    var score = WORD_START
    for (token in tokens) {
        score = minOf(
            score,
            when {
                titleWords.any { it.startsWith(token) } -> WORD_START
                t.contains(token) -> ANYWHERE
                token.length > 1 && lineWords.any { it.startsWith(token) } -> ANYWHERE
                else -> return null
            },
        )
    }
    return score
}

/**
 * Continue watching as one row: what the Stage strip has to continue, then the row after it, which
 * starts where the strip leaves off.
 */
private fun continueRow(snapshot: HomeSnapshot, row: CoreRow?): CoreRow? {
    val tiles = (snapshot.stage.strip.filter { it.provider != null } + row?.tiles.orEmpty().filter { !it.id.startsWith("notice:") }).distinctBy { it.id }
    if (tiles.isEmpty()) return null
    return CoreRow(id = "continue", title = "Continue watching", kind = TileKind.LANDSCAPE, size = RowSize.M, badge = null, tiles = tiles, note = null)
}

/** Before anything is typed: the head of Continue watching, then Your apps, as one row each. */
private fun recentlyOpened(snapshot: HomeSnapshot): List<CoreRow> {
    val out = ArrayList<CoreRow>(2)
    continueRow(snapshot, snapshot.rows.firstOrNull { it.id == "continue" })?.let {
        out += it.copy(id = "recent:continue", tiles = it.tiles.take(6))
    }
    snapshot.rows.firstOrNull { it.id == "apps" && it.tiles.any { t -> !t.id.startsWith("control:") } }?.let {
        out += it.copy(id = "recent:apps", title = "Your apps", tiles = it.tiles.filter { t -> !t.id.startsWith("control:") }.take(6))
    }
    return out
}
