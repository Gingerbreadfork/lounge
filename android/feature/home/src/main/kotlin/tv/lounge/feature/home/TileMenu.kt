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
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.tv.material3.Text
import coil3.compose.AsyncImage
import tv.lounge.core.Tile
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme

data class MenuItem(val label: String, val icon: androidx.compose.ui.graphics.vector.ImageVector, val destructive: Boolean = false, val onSelect: () -> Unit)

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

    Box(Modifier.fillMaxSize()) {
        Layout(
            content = {
                // Placed to the right of the tile unless it would run off the screen; the
                // transform origin follows, so the menu always grows out of the tile's side.
                val toRight = anchor.right + 24.dp.value * 2 + 360.dp.value <= 1920f - 48f
                AnimatedVisibility(
                    visibleState,
                    enter = fadeIn(motion.snap()) +
                        scaleIn(motion.snap(), initialScale = 0.96f, transformOrigin = TransformOrigin(if (toRight) 0f else 1f, 0.15f)) +
                        slideInHorizontally(motion.snap()) { full -> if (toRight) -(full * 8 / 360) else (full * 8 / 360) },
                    exit = fadeOut(androidx.compose.animation.core.tween(90)),
                ) {
                    Column(
                        Modifier
                            .width(360.dp)
                            .clip(RoundedCornerShape(16.dp))
                            .background(colors.curtain)
                            .onPreviewKeyEvent { e -> if (e.isBack() && e.type == KeyEventType.KeyUp) { onDismiss(); true } else false }
                            .focusProperties { canFocus = !leaving }
                            .focusGroup()
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
                            )
                        }
                        items.forEachIndexed { i, item ->
                            SheetRow(
                                icon = item.icon,
                                title = item.label,
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

/** Menu items for a tile, by what it is and where it sits. All of them do something real. */
fun menuItemsFor(
    tile: Tile,
    rowId: String,
    isPinned: Boolean,
    isStagePinned: Boolean,
    onHide: () -> Unit,
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
    if (isApp && rowId == "apps") {
        if (isPinned) {
            items += MenuItem("Move left", LoungeIcons.Move, onSelect = onMoveLeft)
            items += MenuItem("Move right", LoungeIcons.Move, onSelect = onMoveRight)
            items += MenuItem("Unpin from Your apps", LoungeIcons.Pin, onSelect = onUnpin)
        } else {
            items += MenuItem("Pin to Your apps", LoungeIcons.Pin, onSelect = onPin)
        }
    } else if (isApp && rowId == "allapps") {
        items += MenuItem(if (isPinned) "Unpin from Your apps" else "Pin to Your apps", LoungeIcons.Pin, onSelect = if (isPinned) onUnpin else onPin)
    }
    if (tile.id.startsWith("chan:")) items += MenuItem("TV guide", LoungeIcons.Tv, onSelect = onGuide)
    items += if (isStagePinned) MenuItem("Unpin from Stage", LoungeIcons.Pin, onSelect = onUnpinStage) else MenuItem("Pin to Stage", LoungeIcons.Pin, onSelect = onPinStage)
    items += MenuItem("Hide", LoungeIcons.Hide, onSelect = onHide)
    if (isApp) {
        items += MenuItem("App info", LoungeIcons.Info, onSelect = onAppInfo)
        items += MenuItem("Open in Play Store", LoungeIcons.Play, onSelect = onPlay)
        items += MenuItem("Uninstall", LoungeIcons.Trash, destructive = true, onSelect = onUninstall)
    }
    return items
}
