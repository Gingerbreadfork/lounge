package tv.lounge.feature.home

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.layout.LazyLayoutCacheWindow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.runtime.Stable
import androidx.compose.runtime.State
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.geometry.Rect
import tv.lounge.core.Tile

/** Which overlay, if any, is open. Overlays answer one question each (DESIGN.md §2). */
sealed interface Overlay {
    data object None : Overlay
    /** A tile's menu; [under] is the overlay it opened over, which stays open beneath it. */
    data class TileMenu(val tile: Tile, val rowId: String, val anchor: Rect, val returnFocus: FocusRequester, val under: Overlay = None) : Overlay
    data object QuickSettings : Overlay
    data object HomeRole : Overlay
    data object Search : Overlay
    data object NowPlaying : Overlay
    data object Channels : Overlay
    data object Guide : Overlay
    data object Plugins : Overlay
    data class PluginDetail(val id: String) : Overlay
    data object Companion : Overlay
    data object Updates : Overlay
    data object PowerSetup : Overlay
    data object TvControls : Overlay
    data object Ambient : Overlay
    /** The screensaver drawn in-app, until any key. */
    data object AmbientPreview : Overlay
    data object Onboarding : Overlay
}

/** Where focus lives on the home screen. The Stage is expanded only while focus is in it. */
enum class FocusRegion { TopBar, Stage, Rows }

/** One requester per top-bar button, so an overlay can hand focus back to the button that opened it. */
class TopBarFocus {
    val search = FocusRequester()
    val guide = FocusRequester()
    val tvControls = FocusRequester()
    val nowPlaying = FocusRequester()
    val settings = FocusRequester()
    /** The button that last had focus. */
    var last: FocusRequester = search
}

@OptIn(ExperimentalFoundationApi::class)
@Stable
class HomeState(private val tierA: Boolean = true) {
    var region by mutableStateOf(FocusRegion.Stage)
    /** The Stage's selection: the strip tile's id, with its last known index for when the id is gone. */
    var stageTileId by mutableStateOf<String?>(null)
    var stageIndex by mutableIntStateOf(0)
    /**
     * Whether the viewer has moved along the strip since the Stage last opened on its first tile.
     * Until then the first tile leads, even one that arrives after the home appears.
     */
    var stageChosen by mutableStateOf(false)
    var focusedRowId by mutableStateOf<String?>(null)
    var focusedTileIndex by mutableIntStateOf(-1)
    /** The focused row tile's id; not read by composition. */
    var focusedTileId: String? = null
    var overlay by mutableStateOf<Overlay>(Overlay.None)
    var idle by mutableStateOf(false)
    var lastInputAt by mutableStateOf(System.currentTimeMillis())
    /** When Home was last pressed with Lounge in front; not read by composition. */
    var lastHomeAt: Long = System.currentTimeMillis()

    val playbillFocus = FocusRequester()
    val topBar = TopBarFocus()
    val topBarFocus: FocusRequester get() = topBar.search
    var playbillFocused by mutableStateOf(false)
    /** A query the companion asked us to search for, until the search screen takes it. */
    var pendingSearch by mutableStateOf<String?>(null)

    /** What had focus on the home when the open overlay was opened over it. */
    var opener: FocusRequester? = null
    /** The sheets under the open one, so Back returns to the sheet that opened it. */
    val sheetTrail = ArrayList<Overlay>()

    /** The sheet Back last left, so the sheet under it can focus the row that opened it. */
    var sheetLeft: Overlay? = null

    /** The requester for whatever has focus on the home now. */
    fun currentFocus(): FocusRequester? = when (region) {
        FocusRegion.TopBar -> topBar.last
        FocusRegion.Stage -> if (playbillFocused) playbillFocus else stripFocus()
        FocusRegion.Rows -> focusedRowId?.let { rowRequester(it, focusedTileIndex.coerceAtLeast(0)) }
    }

    private val stripRequesters = ArrayList<FocusRequester>()

    /** Stable requester for strip tile [index]; instances never change, so they stay attached. */
    fun stripRequester(index: Int): FocusRequester {
        while (stripRequesters.size <= index) stripRequesters += FocusRequester()
        return stripRequesters[index]
    }

    /** Entering the Stage lands on the selected strip tile, not the first one. */
    fun stripFocus(): FocusRequester = stripRequester(stageIndex)

    private val rowRequesters = HashMap<String, ArrayList<FocusRequester>>()
    private val rowListStates = HashMap<String, LazyListState>()

    fun rowRequester(rowId: String, index: Int): FocusRequester {
        val list = rowRequesters.getOrPut(rowId) { ArrayList() }
        while (list.size <= index) list += FocusRequester()
        return list[index]
    }

    /** Each row keeps its own scroll position, so a row re-entered is where it was left; it also
     *  keeps a couple of tiles composed past each edge so Left and Right never compose in the frame. */
    fun rowListState(rowId: String): LazyListState = rowListStates.getOrPut(rowId) {
        LazyListState(LazyLayoutCacheWindow(ahead = if (tierA) 820.dp else 420.dp, behind = 420.dp), 0, 0)
    }

    /** Entering a row from above lands on its leftmost fully visible tile (where the pivot left it). */
    fun rowEntryRequester(rowId: String): FocusRequester {
        val list = rowListState(rowId)
        val first = list.firstVisibleItemIndex + if (list.firstVisibleItemScrollOffset > 8) 1 else 0
        return rowRequester(rowId, first)
    }

    val stageExpanded: Boolean get() = region != FocusRegion.Rows

    /** The Stage's animated height, published by the Stage slot so the rows can follow it in draw. */
    var stageHeight: State<Dp>? = null

    fun touched() {
        lastInputAt = System.currentTimeMillis()
        idle = false
    }
}
