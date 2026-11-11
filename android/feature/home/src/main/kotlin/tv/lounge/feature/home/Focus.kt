package tv.lounge.feature.home

import android.os.SystemClock
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
import androidx.compose.runtime.snapshots.Snapshot
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
import androidx.compose.ui.node.SemanticsModifierNode
import androidx.compose.ui.node.invalidateSemantics
import androidx.compose.ui.semantics.SemanticsPropertyReceiver
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.onLongClick
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
 * Builds every row that is off screen in idle time, nearest the viewport first and one at a time,
 * since requests made together are not run in the order they were made. Together with
 * [keptOnceMeasured] the whole home is built soon after it opens, so a move between rows builds
 * nothing. A change in [rows] starts over, which costs nothing for a row already built.
 */
@OptIn(ExperimentalFoundationApi::class)
class EveryRowPrefetch(private val rows: () -> List<String>) : LazyListPrefetchStrategy {
    private var asked: List<String> = emptyList()
    /** The rows still to ask for, nearest first. */
    private val waiting = ArrayDeque<Int>()
    private var building: LazyLayoutPrefetchState.PrefetchHandle? = null
    private var buildingIndex = -1
    private var buildingSince = 0L
    private var list: LazyListPrefetchScope? = null

    /** Asks again for a row the scheduler let lie while the window was hidden. For when it is back in front. */
    fun resume() {
        buildingSince = 0L
        list?.keepBuilding()
    }

    override fun LazyListPrefetchScope.onScroll(delta: Float, layoutInfo: LazyListLayoutInfo) {
        list = this
        keepBuilding()
    }

    override fun LazyListPrefetchScope.onVisibleItemsUpdated(layoutInfo: LazyListLayoutInfo) {
        list = this
        val visible = layoutInfo.visibleItemsInfo
        if (visible.isEmpty()) return
        // Read unobserved: the list is measured again whenever its rows change anyway.
        val ids = Snapshot.withoutReadObservation { rows() }
        if (ids != asked) {
            asked = ids
            building?.cancel()
            building = null
            waiting.clear()
            var below = visible.last().index + 1
            var above = visible.first().index - 1
            while (below < layoutInfo.totalItemsCount || above >= 0) {
                if (below < layoutInfo.totalItemsCount) waiting += below++
                if (above >= 0) waiting += above--
            }
        }
        keepBuilding()
    }

    /**
     * Asks for the next row once the one before it is built. A request the scheduler has let lie
     * is asked for again, which also starts the scheduler.
     */
    private fun LazyListPrefetchScope.keepBuilding() {
        val now = SystemClock.uptimeMillis()
        if (building != null && now - buildingSince < STALLED_MS) return
        val index = if (building != null) buildingIndex else waiting.removeFirstOrNull() ?: return
        buildingIndex = index
        buildingSince = now
        building = schedulePrefetch(index) {
            if (this.index == buildingIndex) {
                building = null
                keepBuilding()
            }
        }
    }

    override fun NestedPrefetchScope.onNestedPrefetch(firstVisibleItemIndex: Int) = Unit

    private companion object {
        /** How long a request may lie before it is asked for again. */
        const val STALLED_MS = 2_000L
    }
}

private val selectKeys = setOf(Key.DirectionCenter, Key.Enter, Key.NumPadEnter, Key.ButtonA)
private val menuKeys = setOf(Key.Menu, Key.ButtonY)
private val backKeys = setOf(Key.Back, Key.Escape, Key.ButtonB)

/**
 * D-pad centre for tiles: a short press selects, and holding it, or the Menu or controller Y key,
 * is the hold. It reads the repeated KeyDowns in a node, so a recomposition mid-press keeps the
 * press. Accessibility services are offered the press as [clickLabel] unless the tile [opens]
 * nothing, and the hold only when [longPressLabel] names it.
 */
fun Modifier.tilePress(onClick: () -> Unit, onLongPress: () -> Unit, clickLabel: String? = null, longPressLabel: String? = null, opens: Boolean = true): Modifier =
    this then TilePressElement(onClick, onLongPress, clickLabel, longPressLabel, opens)

private class TilePressElement(
    val onClick: () -> Unit,
    val onLongPress: () -> Unit,
    val clickLabel: String?,
    val longPressLabel: String?,
    val opens: Boolean,
) : ModifierNodeElement<TilePressNode>() {
    override fun create() = TilePressNode(onClick, onLongPress, clickLabel, longPressLabel, opens)

    override fun update(node: TilePressNode) {
        node.press = onClick
        node.hold = onLongPress
        if (node.clickLabel != clickLabel || node.longPressLabel != longPressLabel || node.opens != opens) {
            node.clickLabel = clickLabel
            node.longPressLabel = longPressLabel
            node.opens = opens
            node.invalidateSemantics()
        }
    }

    override fun equals(other: Any?): Boolean =
        other is TilePressElement && other.onClick === onClick && other.onLongPress === onLongPress &&
            other.clickLabel == clickLabel && other.longPressLabel == longPressLabel && other.opens == opens

    override fun hashCode(): Int = (((31 * onClick.hashCode() + onLongPress.hashCode()) * 31 + clickLabel.hashCode()) * 31 + longPressLabel.hashCode()) * 31 + opens.hashCode()
}

private class TilePressNode(
    var press: () -> Unit,
    var hold: () -> Unit,
    var clickLabel: String?,
    var longPressLabel: String?,
    var opens: Boolean,
) : Modifier.Node(), KeyInputModifierNode, FocusEventModifierNode, SemanticsModifierNode {
    override fun SemanticsPropertyReceiver.applySemantics() {
        if (opens) onClick(label = clickLabel) { press(); true }
        longPressLabel?.let { label -> onLongClick(label = label) { hold(); true } }
    }

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
                hold(); true
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
                        hold()
                    }
                }
                true
            }
            isSelect && event.type == KeyEventType.KeyUp -> {
                val armed = downAt != 0L
                if (!longFired && armed) press()
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

/** Back, Escape or a controller's B; never a press the system cancelled, as after a long press it took. */
fun KeyEvent.isBack(): Boolean = key in backKeys && !nativeKeyEvent.isCanceled &&
    (nativeKeyEvent.flags and android.view.KeyEvent.FLAG_CANCELED_LONG_PRESS) == 0

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

/** Focus the selected strip tile; if it is not composed, the first tile; with no strip, the playbill's button. */
suspend fun HomeState.focusStrip(): Boolean =
    stripFocus().requestWhenReady(15) || stripRequester(0).requestWhenReady(15) || playbillFocus.requestWhenReady(4)

/** Focus a row at its entry tile; if that tile is not composed, the first one. */
suspend fun HomeState.focusRow(rowId: String): Boolean =
    rowEntryRequester(rowId).requestWhenReady(8) || rowRequester(rowId, 0).requestWhenReady(8)

/**
 * Which way a d-pad press at an end of the top bar would leave it: -1 for Left off the first
 * button, 1 for Right off the last, 0 for a press that stays inside. The bar nudges instead of
 * letting focus leave sideways (DESIGN.md §5.3).
 */
internal fun topBarEdge(key: Key, focused: Any?, first: Any, last: Any): Int = when {
    key == Key.DirectionLeft && focused === first -> -1
    key == Key.DirectionRight && focused === last -> 1
    else -> 0
}

/** What Back does on the home with nothing open over it (DESIGN.md §6.4). */
enum class HomeBack {
    /** From a row or the top bar: up to the Stage, focus on the strip. */
    ToStage,
    /** From the playbill's buttons: down to the strip they belong to. */
    ToStrip,
    /** From the strip: nothing, the home is already home. */
    Stay,
}

internal fun homeBack(region: FocusRegion, playbillFocused: Boolean): HomeBack = when (region) {
    FocusRegion.Rows, FocusRegion.TopBar -> HomeBack.ToStage
    FocusRegion.Stage -> if (playbillFocused) HomeBack.ToStrip else HomeBack.Stay
}

/** Where focus goes when the rows change under a focused tile, as when it is hidden from its menu. */
sealed interface RowFocus {
    /** Still in its row, now at [index]. */
    data class Stays(val index: Int) : RowFocus

    /** Gone from a row that still has tiles: the one now at its place, or the last. */
    data class Neighbour(val index: Int) : RowFocus

    /** Its row went too: the row now in that row's place. */
    data class NextRow(val rowId: String) : RowFocus

    /** No rows are left: the Stage. */
    data object Stage : RowFocus
}

internal fun rowFocusAfter(before: List<String>, rows: List<tv.lounge.core.Row>, rowId: String, tileId: String, index: Int): RowFocus {
    val row = rows.firstOrNull { it.id == rowId }
    val at = row?.tiles?.indexOfFirst { it.id == tileId } ?: -1
    return when {
        at >= 0 -> RowFocus.Stays(at)
        row != null && row.tiles.isNotEmpty() -> RowFocus.Neighbour(index.coerceIn(0, row.tiles.lastIndex))
        rows.isNotEmpty() -> RowFocus.NextRow(rows[before.indexOf(rowId).coerceIn(0, rows.lastIndex)].id)
        else -> RowFocus.Stage
    }
}
