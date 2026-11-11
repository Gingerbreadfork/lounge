package tv.lounge.feature.home

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.slideInHorizontally
import androidx.compose.foundation.background
import androidx.compose.foundation.focusGroup
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.tv.material3.Text
import coil3.compose.AsyncImage
import tv.lounge.core.Tile
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme

/** One menu entry; [detail] is a second line that says what the action does, for the ones that need it. */
data class MenuItem(
    val label: String,
    val icon: androidx.compose.ui.graphics.vector.ImageVector,
    val destructive: Boolean = false,
    val detail: String? = null,
    val onSelect: () -> Unit,
)

/**
 * The long-press menu: a `curtain` popover beside the tile (DESIGN.md §5.4). It grows from the edge
 * facing the tile over `snap`; the room dims with house lights instead of a scrim and the tile
 * stays lit. Back closes it and returns focus to the tile.
 */
@Composable
fun TileMenuPopover(
    tile: Tile,
    anchor: Rect,
    items: List<MenuItem>,
    visibleState: MutableTransitionState<Boolean>,
    onDismiss: () -> Unit,
) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    val firstFocus = remember { FocusRequester() }
    val leaving = !visibleState.targetState
    val density = androidx.compose.ui.platform.LocalDensity.current
    val screenWidth = androidx.compose.ui.platform.LocalWindowInfo.current.containerSize.width

    Box(Modifier.fillMaxSize()) {
        Layout(
            content = {
                // Placed to the right of the tile unless it would run off the screen; the
                // transform origin follows, so the menu always grows out of the tile's side.
                val toRight = with(density) { anchor.right + (24.dp * 2 + MenuWidth).toPx() <= screenWidth - 48.dp.toPx() }
                AnimatedVisibility(
                    visibleState,
                    enter = fadeIn(motion.snap()) +
                        scaleIn(motion.snap(), initialScale = 0.96f, transformOrigin = TransformOrigin(if (toRight) 0f else 1f, 0.15f)) +
                        slideInHorizontally(motion.snap()) { full -> if (toRight) -(full * 8 / 480) else (full * 8 / 480) },
                    exit = fadeOut(motion.timed(90)),
                ) {
                    Column(
                        Modifier
                            .width(MenuWidth)
                            .clip(RoundedCornerShape(16.dp))
                            .background(colors.curtain)
                            .semantics { paneTitle = "Options for ${tile.title}" }
                            .onPreviewKeyEvent { e -> if (e.isBack() && e.type == KeyEventType.KeyUp) { onDismiss(); true } else false }
                            .focusProperties { canFocus = !leaving }
                            .trapFocus(leaving)
                            .focusGroup()
                            // A long menu at large text scrolls rather than running off the screen.
                            .heightIn(max = 900.dp)
                            .verticalScroll(rememberScrollState())
                            .padding(12.dp),
                        verticalArrangement = Arrangement.spacedBy(4.dp),
                    ) {
                        Row(
                            Modifier.padding(horizontal = 12.dp, vertical = 10.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(12.dp),
                        ) {
                            tile.iconPath?.let { icon ->
                                AsyncImage(model = icon, contentDescription = null, contentScale = ContentScale.Fit, modifier = Modifier.size(40.dp).clip(RoundedCornerShape(10.dp)))
                            }
                            Text(
                                tile.title,
                                style = LoungeTheme.typography.title,
                                color = colors.limelight,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                                modifier = Modifier.semantics { heading() },
                            )
                        }
                        items.forEachIndexed { i, item ->
                            SheetRow(
                                icon = item.icon,
                                title = item.label,
                                subtitle = item.detail,
                                subtitleLines = 2,
                                accent = if (item.destructive) colors.ember else colors.dust,
                                onClick = { item.onSelect(); onDismiss() },
                                focusRequester = if (i == 0) firstFocus else null,
                                height = 56.dp,
                            )
                        }
                    }
                }
            },
        ) { measurables, constraints ->
            // Once the exit has finished the popover has no content to place.
            val placeable = measurables.firstOrNull()?.measure(constraints.copy(minWidth = 0, minHeight = 0))
                ?: return@Layout layout(constraints.maxWidth, constraints.maxHeight) {}
            val gap = 24.dp.roundToPx()
            var x = (anchor.right + gap).toInt()
            if (x + placeable.width > constraints.maxWidth - 48.dp.roundToPx()) x = (anchor.left - gap - placeable.width).toInt()
            var y = anchor.top.toInt()
            if (y + placeable.height > constraints.maxHeight - 54.dp.roundToPx()) y = constraints.maxHeight - 54.dp.roundToPx() - placeable.height
            if (y < 54.dp.roundToPx()) y = 54.dp.roundToPx()
            layout(constraints.maxWidth, constraints.maxHeight) { placeable.place(x.coerceAtLeast(0), y) }
        }
    }
    LaunchedEffect(Unit) { firstFocus.requestWhenReady() }
}

/** Wide enough for the longest item, "Pin to the front of Your apps", on one line. */
private val MenuWidth = 480.dp

/** Whether holding OK on [tile] opens a menu: a tile that only opens a screen or the TV's own controls has nothing to pin or hide. */
fun hasMenu(tile: Tile): Boolean = !tile.id.startsWith("control:") && !tile.id.startsWith("search:") && tile.id != "device:tv"

/** What each pin does, in the menu and on the phone, so the word "pin" never stands alone. */
object PinWords {
    const val PIN_APP = "Pin to the front of Your apps"
    const val PIN_APP_DETAIL = "Stays put ahead of recent apps"
    const val UNPIN_APP = "Unpin from Your apps"
    const val UNPIN_APP_DETAIL = "Follows what you open again"
    const val PIN_STAGE = "Pin to the Stage"
    const val PIN_STAGE_DETAIL = "Stays in the big pictures up top"
    const val UNPIN_STAGE = "Unpin from the Stage"
    const val UNPIN_STAGE_DETAIL = "What you watch takes its place"
    const val HIDE_DETAIL = "Off every row until you show it"
    const val SHOW = "Show"
    const val SHOW_DETAIL = "Back on every row it belongs in"
    const val HIDE_ALL_FROM_DETAIL = "Off Continue watching until you show it"
    const val HIDE_ROW = "Hide this row"
    const val HIDE_ROW_DETAIL = "Settings, Rows brings it back"
}

/**
 * Menu items for a tile, by what it is and where it sits. All of them do something real.
 * [rowTitle] names the home row the tile sits in; the caller passes null for the Stage and for
 * search results, which keep their home row ids. An app hidden from every row, which only All
 * apps and search still list, offers Show and what an app always can.
 */
fun menuItemsFor(
    tile: Tile,
    rowId: String,
    rowTitle: String?,
    isPinned: Boolean,
    canMoveLeft: Boolean,
    canMoveRight: Boolean,
    isStagePinned: Boolean,
    onHide: () -> Unit,
    onShow: () -> Unit,
    onHideAllFrom: () -> Unit,
    onHideRow: () -> Unit,
    onPin: () -> Unit,
    onUnpin: () -> Unit,
    onMoveLeft: () -> Unit,
    onMoveRight: () -> Unit,
    onPinStage: () -> Unit,
    onUnpinStage: () -> Unit,
    onAppInfo: () -> Unit,
    onUninstall: () -> Unit,
    onPlay: () -> Unit,
    onDismissNotice: () -> Unit,
    onGuide: () -> Unit,
): List<MenuItem> {
    val isApp = tile.id.startsWith("app:")
    val items = mutableListOf<MenuItem>()
    if (tile.id.startsWith("notice:")) {
        items += MenuItem("Don't show this again", LoungeIcons.Hide, onSelect = onDismissNotice)
        return items
    }
    if (tile.hidden) {
        items += MenuItem(PinWords.SHOW, LoungeIcons.Show, detail = PinWords.SHOW_DETAIL, onSelect = onShow)
        if (isApp) {
            items += MenuItem("App info", LoungeIcons.Info, onSelect = onAppInfo)
            items += MenuItem("Open in Play Store", LoungeIcons.Play, onSelect = onPlay)
            items += MenuItem("Uninstall", LoungeIcons.Trash, destructive = true, onSelect = onUninstall)
        }
        return items
    }
    if (isApp && rowId == "apps") {
        if (isPinned) {
            if (canMoveLeft) items += MenuItem("Move left", LoungeIcons.Move, onSelect = onMoveLeft)
            if (canMoveRight) items += MenuItem("Move right", LoungeIcons.Move, onSelect = onMoveRight)
            items += MenuItem(PinWords.UNPIN_APP, LoungeIcons.Pin, detail = PinWords.UNPIN_APP_DETAIL, onSelect = onUnpin)
        } else {
            items += MenuItem(PinWords.PIN_APP, LoungeIcons.Pin, detail = PinWords.PIN_APP_DETAIL, onSelect = onPin)
        }
    } else if (isApp && rowId == "allapps") {
        items += if (isPinned) {
            MenuItem(PinWords.UNPIN_APP, LoungeIcons.Pin, detail = PinWords.UNPIN_APP_DETAIL, onSelect = onUnpin)
        } else {
            MenuItem(PinWords.PIN_APP, LoungeIcons.Pin, detail = PinWords.PIN_APP_DETAIL, onSelect = onPin)
        }
    }
    if (tile.id.startsWith("chan:")) items += MenuItem("TV guide", LoungeIcons.Guide, onSelect = onGuide)
    if (isStagePinned) {
        items += MenuItem(PinWords.UNPIN_STAGE, LoungeIcons.Pin, detail = PinWords.UNPIN_STAGE_DETAIL, onSelect = onUnpinStage)
    } else if (tile.action.isNotEmpty()) {
        // The Stage is for what can be opened, which a tile to read cannot.
        items += MenuItem(PinWords.PIN_STAGE, LoungeIcons.Pin, detail = PinWords.PIN_STAGE_DETAIL, onSelect = onPinStage)
    }
    items += MenuItem("Hide", LoungeIcons.Hide, detail = PinWords.HIDE_DETAIL, onSelect = onHide)
    // Only something to continue says where it comes from.
    tile.providerName?.let { name ->
        items += MenuItem("Hide everything from $name", LoungeIcons.Hide, detail = PinWords.HIDE_ALL_FROM_DETAIL, onSelect = onHideAllFrom)
    }
    if (rowTitle != null) items += MenuItem(PinWords.HIDE_ROW, LoungeIcons.Rows, detail = PinWords.HIDE_ROW_DETAIL, onSelect = onHideRow)
    if (isApp) {
        items += MenuItem("App info", LoungeIcons.Info, onSelect = onAppInfo)
        items += MenuItem("Open in Play Store", LoungeIcons.Play, onSelect = onPlay)
        items += MenuItem("Uninstall", LoungeIcons.Trash, destructive = true, onSelect = onUninstall)
    }
    return items
}
