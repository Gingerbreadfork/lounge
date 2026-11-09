package tv.lounge.feature.home

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.dp
import androidx.tv.material3.Text
import coil3.compose.AsyncImage
import kotlinx.coroutines.launch
import tv.lounge.core.ItemRef
import tv.lounge.core.RowInfo
import tv.lounge.core.SourceInfo
import tv.lounge.design.Hairline
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme
import tv.lounge.design.sounds

/**
 * The row editor: every row the home could show, top to bottom, with what each holds, and under
 * them the tiles hidden from every row and where Continue watching comes from. OK shows or hides
 * a row or a source; holding OK picks a row up, Up and Down move it, OK sets it down. [rows],
 * [hidden] and [sources] are null until read.
 */
@Composable
fun RowsSheet(
    rows: List<RowInfo>?,
    hidden: List<ItemRef>?,
    sources: List<SourceInfo>?,
    onToggle: (RowInfo) -> Unit,
    onMove: (RowInfo, Int) -> Unit,
    onResetOrder: () -> Unit,
    onShowItem: (ItemRef) -> Unit,
    onToggleSource: (SourceInfo) -> Unit,
    firstFocus: FocusRequester,
) {
    val colors = LoungeTheme.colors
    val sounds = LoungeTheme.sounds
    val scope = rememberCoroutineScope()
    var moving by remember { mutableStateOf<String?>(null) }
    // One requester per row and per hidden item, by id, so focus can be asked back after a move
    // or a removal whatever the list looks like then.
    val requesters = remember { HashMap<String, FocusRequester>() }
    fun requesterFor(id: String) = requesters.getOrPut(id) { FocusRequester() }
    val resetFocus = remember { FocusRequester() }

    SheetHeading("Home screen rows", "OK shows or hides a row. Hold OK to pick one up, then Up and Down move it.")
    when {
        rows == null -> Text("Loading…", style = LoungeTheme.typography.meta, color = colors.dust)
        else -> Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            rows.forEachIndexed { i, row ->
                key(row.id) {
                    val isMoving = moving == row.id
                    val own = requesterFor(row.id)
                    val focusedNow = remember { booleanArrayOf(false) }
                    SheetRow(
                        icon = rowIcon(row.id),
                        title = row.title,
                        subtitle = row.detail,
                        value = when {
                            isMoving -> "Moving"
                            !row.shown -> "Hidden"
                            row.count == 0u -> "Empty"
                            else -> "Shown"
                        },
                        accent = when {
                            isMoving -> colors.houselight
                            row.shown && row.count > 0u -> colors.signal
                            else -> colors.dust
                        },
                        onClick = {
                            if (isMoving) {
                                sounds?.close()
                                moving = null
                            } else {
                                sounds?.tick()
                                onToggle(row)
                            }
                        },
                        onLongClick = {
                            sounds?.open()
                            moving = if (isMoving) null else row.id
                        },
                        onCycle = if (isMoving) null else { _ -> sounds?.tick(); onToggle(row) },
                        focusRequester = if (i == 0) firstFocus else null,
                        modifier = Modifier
                            .focusRequester(own)
                            .onFocusChanged {
                                focusedNow[0] = it.hasFocus
                                // A row set down elsewhere stops moving, once the frame has settled
                                // and focus has not come straight back to it.
                                if (!it.hasFocus && isMoving) {
                                    scope.launch {
                                        withFrameNanos { }
                                        if (moving == row.id && !focusedNow[0]) moving = null
                                    }
                                }
                            }
                            .onPreviewKeyEvent { e ->
                                if (!isMoving || e.type != KeyEventType.KeyDown) return@onPreviewKeyEvent false
                                val to = when (e.key) {
                                    Key.DirectionUp -> i - 1
                                    Key.DirectionDown -> i + 1
                                    else -> return@onPreviewKeyEvent false
                                }
                                if (to in rows.indices) {
                                    sounds?.tick()
                                    onMove(row, to)
                                    // The node moves with its key; asked again after the frame in case focus did not follow.
                                    scope.launch {
                                        withFrameNanos { }
                                        own.requestWhenReady()
                                    }
                                }
                                true
                            },
                    )
                }
            }
            Spacer(Modifier.height(8.dp))
            SheetRow(
                icon = LoungeIcons.Refresh,
                title = "Put the rows back in their usual order",
                subtitle = "Keeps what you have hidden",
                onClick = { sounds?.tick(); moving = null; onResetOrder() },
                focusRequester = resetFocus,
            )
        }
    }

    Spacer(Modifier.height(24.dp))
    Hairline(Modifier.fillMaxWidth())
    SheetGroup("Hidden from every row")
    when {
        hidden == null -> Unit
        hidden.isEmpty() -> Text("Nothing is hidden. Hold OK on any tile and choose Hide.", style = LoungeTheme.typography.meta, color = colors.dust)
        else -> Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            hidden.forEachIndexed { i, item ->
                key(item.id) {
                    SheetRow(
                        icon = if (item.iconPath == null) LoungeIcons.Hide else null,
                        leading = item.iconPath?.let { path ->
                            { AsyncImage(model = path, contentDescription = null, contentScale = ContentScale.Fit, modifier = Modifier.size(28.dp).clip(RoundedCornerShape(7.dp))) }
                        },
                        title = item.title,
                        value = "Show",
                        onClick = {
                            sounds?.tick()
                            // The row goes at once, so focus moves to its neighbour, or back up to the reset row.
                            val next = hidden.getOrNull(i + 1) ?: hidden.getOrNull(i - 1)
                            onShowItem(item)
                            scope.launch {
                                withFrameNanos { }
                                (next?.let { requesterFor(it.id) } ?: resetFocus).requestWhenReady()
                            }
                        },
                        focusRequester = requesterFor(item.id),
                    )
                }
            }
        }
    }
    Spacer(Modifier.height(24.dp))
    SheetGroup("Continue watching comes from")
    when {
        sources == null -> Unit
        sources.isEmpty() -> Text("Nothing to continue yet. Apps and plugins are listed here once they bring something.", style = LoungeTheme.typography.meta, color = colors.dust)
        else -> Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            sources.forEachIndexed { i, source ->
                key(source.id) {
                    SheetRow(
                        icon = if (source.iconPath == null) LoungeIcons.Play else null,
                        leading = source.iconPath?.let { path ->
                            { AsyncImage(model = path, contentDescription = null, contentScale = ContentScale.Fit, modifier = Modifier.size(28.dp).clip(RoundedCornerShape(7.dp))) }
                        },
                        title = source.title,
                        subtitle = when (source.count) {
                            0u -> "Nothing to continue now"
                            1u -> "1 title"
                            else -> "${source.count} titles"
                        },
                        value = if (source.shown) "Shown" else "Hidden",
                        accent = if (source.shown && source.count > 0u) colors.signal else colors.dust,
                        onClick = {
                            sounds?.tick()
                            // One that brings nothing leaves the list once it is shown again, so
                            // focus moves to its neighbour, or back up to the reset row.
                            val leaving = !source.shown && source.count == 0u
                            val next = sources.getOrNull(i + 1) ?: sources.getOrNull(i - 1)
                            onToggleSource(source)
                            if (leaving) {
                                scope.launch {
                                    withFrameNanos { }
                                    (next?.let { requesterFor("from:${it.id}") } ?: resetFocus).requestWhenReady()
                                }
                            }
                        },
                        onCycle = if (!source.shown && source.count == 0u) null else { _ -> sounds?.tick(); onToggleSource(source) },
                        focusRequester = requesterFor("from:${source.id}"),
                    )
                }
            }
        }
    }
    SheetFootnote("A row with nothing in it stays off the screen until it has something to show.")
    // The list can arrive after the sheet's own request for focus has given up.
    LaunchedEffect(rows != null) { if (!rows.isNullOrEmpty()) firstFocus.requestWhenReady() }
}

/** The glyph a row carries in the editor, by what it holds. */
private fun rowIcon(rowId: String) = when {
    rowId == "continue" -> LoungeIcons.Play
    rowId == "games" -> LoungeIcons.Controller
    rowId == "live" -> LoungeIcons.Tv
    rowId == "tvcontrols" -> LoungeIcons.Hdmi
    rowId.startsWith("channel:") -> LoungeIcons.Guide
    rowId.startsWith("plugin:") -> LoungeIcons.Rows
    else -> LoungeIcons.Apps
}
