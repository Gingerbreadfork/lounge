package tv.lounge.feature.home

import androidx.compose.animation.core.AnimationSpec
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.gestures.BringIntoViewSpec
import androidx.compose.foundation.lazy.LazyListLayoutInfo
import androidx.compose.foundation.lazy.LazyListPrefetchScope
import androidx.compose.foundation.lazy.LazyListPrefetchStrategy
import androidx.compose.foundation.lazy.layout.LazyLayoutPrefetchState
import androidx.compose.foundation.lazy.layout.NestedPrefetchScope
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.staticCompositionLocalOf
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.emptyFlow
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusDirection
import androidx.compose.ui.focus.FocusEventModifierNode
import androidx.compose.ui.focus.FocusState
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEvent
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.KeyInputModifierNode
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.type
import androidx.compose.ui.node.ModifierNodeElement
import tv.lounge.design.LoungeTheme

/**
 * Pivot scrolling: the focused item moves to a fixed position (the focus line) and the list
 * scrolls under it, so the eye never hunts (DESIGN.md §5.3).
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun pivotSpec(pivotPx: Float): BringIntoViewSpec {
    val spec: AnimationSpec<Float> = LoungeTheme.motion.travel()
    return remember(pivotPx, spec) {
        object : BringIntoViewSpec {
            override val scrollAnimationSpec: AnimationSpec<Float> = spec
            override fun calculateScrollDistance(offset: Float, size: Float, containerSize: Float): Float {
                val distance = offset - pivotPx
                return if (kotlin.math.abs(distance) < 0.5f) 0f else distance
            }
        }
    }
}

/**
 * Keeps the next [rowsAhead] rows below the viewport composed and measured before any scroll,
 * [tilesPerRow] tiles each. The default strategy prefetches one item only once a scroll has begun,
 * which is too late when the Stage lowers.
 */
@OptIn(ExperimentalFoundationApi::class)
class RowsAheadPrefetch(private val rowsAhead: Int, private val tilesPerRow: Int) : LazyListPrefetchStrategy {
    private val handles = LinkedHashMap<Int, LazyLayoutPrefetchState.PrefetchHandle>()

    override fun LazyListPrefetchScope.onScroll(delta: Float, layoutInfo: LazyListLayoutInfo) {
        fill(layoutInfo)
    }

    override fun LazyListPrefetchScope.onVisibleItemsUpdated(layoutInfo: LazyListLayoutInfo) {
        fill(layoutInfo)
    }

    override fun NestedPrefetchScope.onNestedPrefetch(firstVisibleItemIndex: Int) {
        repeat(tilesPerRow) { schedulePrefetch(firstVisibleItemIndex + it) }
    }

    private fun LazyListPrefetchScope.fill(layoutInfo: LazyListLayoutInfo) {
        val visible = layoutInfo.visibleItemsInfo
        if (visible.isEmpty()) return
        val last = visible.last().index
        val wanted = (last + 1..last + rowsAhead).filter { it < layoutInfo.totalItemsCount }.toSet()
        val gone = handles.keys.filter { it !in wanted }
        for (i in gone) handles.remove(i)?.cancel()
        for (i in wanted) if (i !in handles) handles[i] = schedulePrefetch(i)
    }
}

private val selectKeys = setOf(Key.DirectionCenter, Key.Enter, Key.NumPadEnter, Key.ButtonA)
private val menuKeys = setOf(Key.Menu, Key.ButtonY)
private val backKeys = setOf(Key.Back, Key.Escape, Key.ButtonB)

/**
 * D-pad centre for tiles: a short press selects; holding it, or the Menu or controller Y key,
 * opens the tile menu. `combinedClickable` cannot see key long-presses, so this reads the repeated
 * KeyDowns. The press lives in a node, so a recomposition between KeyDown and KeyUp keeps it.
 */
fun Modifier.tilePress(onClick: () -> Unit, onLongPress: () -> Unit): Modifier = this then TilePressElement(onClick, onLongPress)

private class TilePressElement(val onClick: () -> Unit, val onLongPress: () -> Unit) : ModifierNodeElement<TilePressNode>() {
    override fun create() = TilePressNode(onClick, onLongPress)

    override fun update(node: TilePressNode) {
        node.onClick = onClick
        node.onLongPress = onLongPress
    }

    override fun equals(other: Any?): Boolean =
        other is TilePressElement && other.onClick === onClick && other.onLongPress === onLongPress

    override fun hashCode(): Int = 31 * onClick.hashCode() + onLongPress.hashCode()
}

private class TilePressNode(var onClick: () -> Unit, var onLongPress: () -> Unit) : Modifier.Node(), KeyInputModifierNode, FocusEventModifierNode {
    private var downAt = 0L
    private var longFired = false

    override fun onFocusEvent(focusState: FocusState) {
        if (!focusState.isFocused) {
            downAt = 0L
            longFired = false
        }
    }

    override fun onPreKeyEvent(event: KeyEvent): Boolean {
        val isSelect = event.key in selectKeys
        val isMenu = event.key in menuKeys
        return when {
            isMenu && event.type == KeyEventType.KeyUp -> {
                onLongPress(); true
            }
            isMenu && event.type == KeyEventType.KeyDown -> true
            isSelect && event.type == KeyEventType.KeyDown -> {
                val now = System.currentTimeMillis()
                when {
                    event.nativeKeyEvent.repeatCount == 0 -> {
                        downAt = now
                        longFired = false
                    }
                    // A repeat whose first press landed elsewhere (the tile whose menu just opened) is not ours.
                    downAt == 0L -> Unit
                    // The system flags the repeat at the person's own long-press time; failing that, 450 ms.
                    !longFired && (event.nativeKeyEvent.isLongPress || now - downAt >= LONG_PRESS_MS) -> {
                        longFired = true
                        onLongPress()
                    }
                }
                true
            }
            isSelect && event.type == KeyEventType.KeyUp -> {
                val armed = downAt != 0L
                if (!longFired && armed) onClick()
                downAt = 0L
                longFired = false
                armed
            }
            else -> false
        }
    }

    override fun onKeyEvent(event: KeyEvent): Boolean = false
}

private const val LONG_PRESS_MS = 450L

private val Directional = setOf(FocusDirection.Up, FocusDirection.Down, FocusDirection.Left, FocusDirection.Right, FocusDirection.Next, FocusDirection.Previous)

/**
 * Keeps the d-pad inside an overlay, so focus never walks onto the home behind it. Goes before the
 * overlay's focus group; requests from code still leave, and so does everything once [leaving].
 */
fun Modifier.trapFocus(leaving: Boolean): Modifier = focusProperties {
    onExit = { if (!leaving && requestedFocusDirection in Directional) cancelFocusChange() }
}

fun KeyEvent.isBack(): Boolean = key in backKeys

/** Text typed on the phone, for the Lounge text field that has focus. */
val LocalPhoneText = staticCompositionLocalOf<Flow<String>> { emptyFlow() }

/** While [active], hands each piece of text typed on the phone to [onText]. */
@Composable
fun PhoneTextInput(active: Boolean, onText: (String) -> Unit) {
    val texts = LocalPhoneText.current
    val latest = rememberUpdatedState(onText)
    LaunchedEffect(active, texts) {
        if (active) texts.collect { latest.value(it) }
    }
}

/**
 * Request focus once the target is attached. Lazy lists compose their children a frame or two
 * after the request would otherwise fire, so retry across frames instead of failing silently.
 */
suspend fun androidx.compose.ui.focus.FocusRequester.requestWhenReady(maxFrames: Int = 30): Boolean {
    repeat(maxFrames) {
        val granted = try {
            requestFocus()
        } catch (_: IllegalStateException) {
            false
        }
        if (granted) return true
        androidx.compose.runtime.withFrameNanos { }
    }
    android.util.Log.w("Lounge", "focus request not granted after $maxFrames frames")
    return false
}

/** Focus the selected strip tile; if it is not composed, the first tile. */
suspend fun HomeState.focusStrip(): Boolean =
    stripFocus().requestWhenReady(15) || stripRequester(0).requestWhenReady(15)

/** Focus a row at its entry tile; if that tile is not composed, the first one. */
suspend fun HomeState.focusRow(rowId: String): Boolean =
    rowEntryRequester(rowId).requestWhenReady(8) || rowRequester(rowId, 0).requestWhenReady(8)
