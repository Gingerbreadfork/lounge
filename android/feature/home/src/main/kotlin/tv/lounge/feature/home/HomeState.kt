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
    data class TileMenu(val tile: Tile, val rowId: String, val anchor: Rect, val returnFocus: FocusRequester) : Overlay
    data object QuickSettings : Overlay
    data object HomeRole : Overlay
    data object Search : Overlay
    data object NowPlaying : Overlay
    data object Channels : Overlay
    data object Guide : Overlay
    data object Plugins : Overlay
    data class PluginDetail(val id: String) : Overlay
    data object Companion : Overlay
    data object PowerSetup : Overlay
    data object TvControls : Overlay
    data object Ambient : Overlay
    /** The screensaver drawn in-app, until any key. */
    data object AmbientPreview : Overlay
    data object Onboarding : Overlay
}

/** Where focus lives on the home screen. The Stage is expanded only while focus is in it. */
enum class FocusRegion { TopBar, Stage, Rows }

@OptIn(ExperimentalFoundationApi::class)
@Stable
class HomeState(private val tierA: Boolean = true) {
    var region by mutableStateOf(FocusRegion.Stage)
    var stageIndex by mutableIntStateOf(0)
    var focusedRowId by mutableStateOf<String?>(null)
    var focusedTileIndex by mutableIntStateOf(-1)
    var overlay by mutableStateOf<Overlay>(Overlay.None)
    var idle by mutableStateOf(false)
    var lastInputAt by mutableStateOf(System.currentTimeMillis())

    val playbillFocus = FocusRequester()
    val topBarFocus = FocusRequester()
    var playbillFocused by mutableStateOf(false)
    /** A query the companion asked us to search for when the search screen next opens. */
    var pendingSearch by mutableStateOf("")

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
