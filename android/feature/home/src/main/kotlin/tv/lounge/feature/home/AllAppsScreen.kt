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
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
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
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.tv.material3.Icon
import androidx.tv.material3.Text
import kotlinx.coroutines.launch
import tv.lounge.core.RowSize
import tv.lounge.core.Tile
import tv.lounge.core.TileKind
import tv.lounge.design.Dimens
import tv.lounge.design.Hairline
import tv.lounge.design.Lights
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme
import tv.lounge.design.Swap
import tv.lounge.design.focusRing
import tv.lounge.design.glowRoom
import tv.lounge.design.sounds
import tv.lounge.platform.Prefs

/**
 * Every app as a grid of banners (DESIGN.md §5.5), sorted A to Z, by when they were opened or by
 * when they arrived, and narrowed by typing. Up from the grid reaches the sort and the search
 * field; Left from the first column reaches the alphabet, which Up and Down scroll by and Right
 * or OK drop in from. Holding OK on an app opens the home's menu.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun AllAppsScreen(
    apps: List<Tile>,
    sort: String,
    onSetSort: (String) -> Unit,
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

    var field by rememberSaveable(stateSaver = TextFieldValue.Saver) { mutableStateOf(TextFieldValue("")) }
    val query = field.text.trim()
    val fieldFocus = remember { FocusRequester() }
    var fieldFocused by remember { mutableStateOf(false) }
    PhoneTextInput(active = !leaving && !covered) { text -> field = TextFieldValue(field.text + text, TextRange(field.text.length + text.length)) }

    val shown: List<Tile> = remember(apps, sort, query) { arrange(apps, sort, query) }
    val alphabetical = sort == Prefs.SORT_AZ && query.isEmpty()

    val requesters = remember { ArrayList<FocusRequester>() }
    fun requester(index: Int): FocusRequester {
        while (requesters.size <= index) requesters += FocusRequester()
        return requesters[index]
    }
    val letterRequesters = remember { HashMap<Char, FocusRequester>() }
    fun letterRequester(c: Char) = letterRequesters.getOrPut(c) { FocusRequester() }

    val letters = remember(shown) { shown.map { letterOf(it.title) } }
    val firstOfLetter = remember(letters) { letters.withIndex().groupBy({ it.value }, { it.index }).mapValues { it.value.first() } }
    var focusedIndex by remember { mutableIntStateOf(-1) }
    var focusedTileId by remember { mutableStateOf<String?>(null) }
    var focusedLetter by remember { mutableStateOf<Char?>(null) }
    // The letter the grid is at, or the one chosen on the alphabet.
    val currentLetter = focusedLetter ?: letters.getOrNull(focusedIndex)
    val held = rememberUpdatedState(heldTileId)
    val roomDim = remember { derivedStateOf { held.value() != null } }

    // A new order or a narrower list starts from its top.
    LaunchedEffect(sort, query) { gridState.scrollToItem(0) }
    // An app hidden or pinned from its menu leaves its place: focus goes to the tile that took
    // it, or the last one.
    LaunchedEffect(shown) {
        val id = focusedTileId ?: return@LaunchedEffect
        if (leaving || covered || fieldFocused || shown.any { it.id == id }) return@LaunchedEffect
        if (shown.isEmpty()) return@LaunchedEffect
        requester(focusedIndex.coerceIn(0, shown.lastIndex)).requestWhenReady()
    }

    val rise = with(LocalDensity.current) { 10.dp.roundToPx() }
    val pivotPx = with(LocalDensity.current) { Dimens.glowRoom.toPx() }

    AnimatedVisibility(visibleState, enter = fadeIn(tween(180)), exit = fadeOut(motion.settle())) {
        Box(
            Modifier
                .fillMaxSize()
                .background(colors.velvet)
                .onPreviewKeyEvent { e ->
                    if (!e.isBack() || e.type != KeyEventType.KeyUp) return@onPreviewKeyEvent false
                    // Back takes the filter away first; the keyboard, when it is up, has already taken its own Back.
                    if (query.isNotEmpty()) field = TextFieldValue("") else onClose()
                    true
                }
                .trapFocus(leaving)
                .focusGroup()
                .focusProperties { if (leaving) canFocus = false },
        ) {
            // The grid comes first so the heading paints over it: scrolled rows pass under a fade
            // rather than through the words. The keyboard covers the grid's lower rows rather than
            // resizing it, so the rows above stay where they were when it goes.
            Row(Modifier.fillMaxSize().padding(start = Dimens.safeX, end = Dimens.safeX, top = HeaderHeight + FadeHeight)) {
                // The alphabet: letters with apps are live, the one the grid is at glows. It only
                // means something A to Z, so it fades for the other orders and while narrowed.
                Box(Modifier.width(RailWidth)) {
                    androidx.compose.animation.AnimatedVisibility(visible = alphabetical, modifier = Modifier, enter = fadeIn(tween(180)), exit = fadeOut(tween(120))) {
                        Column(Modifier.focusGroup(), horizontalAlignment = Alignment.Start) {
                            for (c in ALPHABET) {
                                val first = firstOfLetter[c]
                                val live = first != null && alphabetical
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
                                        modifier = Modifier.alpha(if (first != null) 1f else 0.3f),
                                    )
                                }
                            }
                        }
                    }
                }
                if (shown.isEmpty()) {
                    // Something must hold focus while there is nothing to show.
                    Box(Modifier.size(1.dp).focusRequester(requester(0)).focusable())
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                        Icon(LoungeIcons.Apps, contentDescription = null, tint = colors.dust, modifier = Modifier.size(32.dp))
                        Text(
                            if (query.isEmpty()) "Apps appear here once they are installed." else "No app is called “$query”. Back clears the search.",
                            style = LoungeTheme.typography.meta,
                            color = colors.dust,
                        )
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
                            // The glow room first: inside it the grid fills the height, so a short
                            // list is not centred in the room it did not fill.
                            modifier = Modifier
                                .glowRoom()
                                .fillMaxSize()
                                .focusGroup(),
                        ) {
                            itemsIndexed(shown, key = { _, t -> t.id }) { index, tile ->
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
                                        if (alphabetical && index % COLUMNS == 0 && e.key == Key.DirectionLeft && e.type == KeyEventType.KeyDown) {
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
                    Row(
                        Modifier.fillMaxWidth().height(HeaderHeight).background(colors.velvet).padding(start = Dimens.safeX, end = Dimens.safeX, top = Dimens.safeY),
                        horizontalArrangement = Arrangement.SpaceBetween,
                        verticalAlignment = Alignment.Top,
                    ) {
                        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                            Text("All apps", style = LoungeTheme.typography.displayL, color = colors.limelight)
                            Text(
                                when {
                                    apps.isEmpty() -> "Nothing here yet."
                                    query.isNotEmpty() && shown.isEmpty() -> "Nothing matches “$query”."
                                    query.isNotEmpty() -> "${shown.size} of ${apps.size} match “$query”."
                                    else -> "${apps.size} ${if (apps.size == 1) "app" else "apps"}, ${sortLine(sort)}."
                                },
                                style = LoungeTheme.typography.meta,
                                color = colors.dust,
                            )
                            Text(
                                if (alphabetical) "Hold OK on an app for its menu. Up for search and sorting, left of the grid for the alphabet." else "Hold OK on an app for its menu. Up for search and sorting.",
                                style = LoungeTheme.typography.caption,
                                color = colors.dust,
                            )
                        }
                        // The sort and the search: one pill that cycles, one field that narrows.
                        Row(Modifier.padding(top = 8.dp).focusGroup(), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
                            SortPill(sort = sort, onSetSort = { s -> sounds?.tick(); onSetSort(s) })
                            SearchField(
                                field = field,
                                onChange = { field = it },
                                focusRequester = fieldFocus,
                                onFocused = { fieldFocused = it },
                                // The field keeps Down for itself, so the grid is reached by hand: the app last focused, or the first.
                                onDown = { if (shown.isNotEmpty()) requester(focusedIndex.coerceIn(0, shown.lastIndex)).requestFocus() },
                            )
                        }
                    }
                    Box(Modifier.fillMaxWidth().height(FadeHeight).background(Brush.verticalGradient(0f to colors.velvet, 1f to colors.velvet.copy(alpha = 0f))))
                }
            }
        }
    }
    LaunchedEffect(Unit) { requester(0).requestWhenReady() }
}

/** The order, as a pill: OK cycles through the three; Left and Right move on, since the field sits beside it. */
@Composable
private fun SortPill(sort: String, onSetSort: (String) -> Unit) {
    val colors = LoungeTheme.colors
    var focused by remember { mutableStateOf(false) }
    val at = SORTS.indexOf(sort).coerceAtLeast(0)
    fun next() = onSetSort(SORTS[(at + 1) % SORTS.size])
    Row(
        Modifier
            .clip(RoundedCornerShape(24.dp))
            .background(if (focused) colors.curtainEdge else colors.curtain)
            .focusRing(colors, { focused }, 24.dp)
            .onFocusChanged { focused = it.isFocused }
            .tilePress({ next() }, { next() })
            .focusable()
            .semantics { role = Role.Button; contentDescription = "Sorted ${sortLabel(sort)}. OK changes the order" }
            .height(48.dp)
            .padding(horizontal = 18.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Icon(LoungeIcons.Rows, contentDescription = null, tint = if (focused) colors.houselight else colors.dust, modifier = Modifier.size(22.dp))
        Swap(sortLabel(sort), rise = 0.dp, shiftX = 8.dp, durationMs = 180) { label ->
            Text(label, style = LoungeTheme.typography.title, color = colors.limelight, maxLines = 1)
        }
        // A quiet cue that OK turns it over.
        Text("\u21bb", style = LoungeTheme.typography.meta, color = if (focused) colors.houselight else colors.dust)
    }
}

/** The search field: a glyph, the typed text in `title`, and a hairline that lights with focus. */
@Composable
private fun SearchField(field: TextFieldValue, onChange: (TextFieldValue) -> Unit, focusRequester: FocusRequester, onFocused: (Boolean) -> Unit, onDown: () -> Unit) {
    val colors = LoungeTheme.colors
    var focused by remember { mutableStateOf(false) }
    Column(
        Modifier
            .width(440.dp)
            .onPreviewKeyEvent { e ->
                if (e.key == Key.DirectionDown && e.type == KeyEventType.KeyDown) { onDown(); true } else false
            },
    ) {
        Row(Modifier.height(48.dp).padding(horizontal = 6.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Icon(LoungeIcons.Search, contentDescription = null, tint = if (focused) colors.houselight else colors.dust, modifier = Modifier.size(24.dp))
            Box(Modifier.weight(1f)) {
                if (field.text.isEmpty()) Text("Find an app", style = LoungeTheme.typography.title, color = colors.dust, maxLines = 1, overflow = TextOverflow.Ellipsis)
                BasicTextField(
                    value = field,
                    onValueChange = onChange,
                    singleLine = true,
                    textStyle = LoungeTheme.typography.title.copy(color = colors.limelight),
                    cursorBrush = SolidColor(colors.houselight),
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                    keyboardActions = KeyboardActions(onSearch = { }),
                    modifier = Modifier
                        .fillMaxWidth()
                        .focusRequester(focusRequester)
                        .onFocusChanged { focused = it.isFocused; onFocused(it.isFocused) },
                )
            }
        }
        Box(Modifier.fillMaxWidth().height(2.dp).background(if (focused) colors.houselight else colors.curtainEdge))
    }
}

private const val COLUMNS = 6
private val RailWidth = 72.dp
private val HeaderHeight = 196.dp
private val FadeHeight = 40.dp
private val ALPHABET: List<Char> = listOf('#') + ('A'..'Z')
private val SORTS = listOf(Prefs.SORT_AZ, Prefs.SORT_OPENED, Prefs.SORT_INSTALLED)

private fun sortLabel(sort: String) = when (sort) {
    Prefs.SORT_OPENED -> "Recently opened"
    Prefs.SORT_INSTALLED -> "Recently installed"
    else -> "A to Z"
}

private fun sortLine(sort: String) = when (sort) {
    Prefs.SORT_OPENED -> "most recently opened first"
    Prefs.SORT_INSTALLED -> "newest first"
    else -> "A to Z"
}

/** The apps in the chosen order, narrowed to those whose name has every word typed. */
private fun arrange(apps: List<Tile>, sort: String, query: String): List<Tile> {
    val words = query.lowercase().split(Regex("\\s+")).filter { it.isNotEmpty() }
    val matching = if (words.isEmpty()) apps else apps.filter { t -> val name = t.title.lowercase(); words.all { name.contains(it) } }
    val byName = compareBy<Tile> { it.title.lowercase() }
    return when (sort) {
        // Never opened sorts after everything that was, by name.
        Prefs.SORT_OPENED -> matching.sortedWith(compareByDescending<Tile> { it.lastUsedMs ?: Long.MIN_VALUE }.then(byName))
        Prefs.SORT_INSTALLED -> matching.sortedWith(compareByDescending<Tile> { it.addedMs ?: Long.MIN_VALUE }.then(byName))
        else -> matching.sortedWith(byName)
    }
}

/** The alphabet entry a title files under: its first letter, or # for a digit or symbol. */
private fun letterOf(title: String): Char {
    val c = title.trim().firstOrNull()?.uppercaseChar() ?: '#'
    return if (c in 'A'..'Z') c else '#'
}
