package tv.lounge.feature.home

import androidx.compose.animation.core.Animatable
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.layout.LazyLayoutCacheWindow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.runtime.Stable
import androidx.compose.runtime.State
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
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
    /** The languages Lounge is written in, to choose one over the TV's own. */
    data object Language : Overlay
    data object Search : Overlay
    /** Every app, A to Z, with an alphabet to jump by. */
    data object AllApps : Overlay
    data object NowPlaying : Overlay
    /** What apps on the TV have posted, newest first. */
    data object Notifications : Overlay
    /** One notification in full, with what can be done with it. */
    data class NotificationDetail(val key: String) : Overlay
    /** Whether notifications show, and the apps left out of them. */
    data object NotificationSettings : Overlay
    data object Channels : Overlay
    /** The row editor: which rows show and in what order, and what is hidden. */
    data object Rows : Overlay
    data object Guide : Overlay
    data object Plugins : Overlay
    data class PluginDetail(val id: String) : Overlay
    /** A sheet a plugin describes: a room, a thermostat, a list to choose from. */
    data class PluginSheet(val plugin: String, val id: String) : Overlay
    /** A plugin's still picture, full screen and renewed about once a second. */
    data class Live(val plugin: String, val title: String, val url: String) : Overlay
    data object Companion : Overlay
    data object Updates : Overlay
    data object PowerSetup : Overlay
    data object TvControls : Overlay
    data object Ambient : Overlay
    /** The screensaver drawn in-app, until any key. */
    data object AmbientPreview : Overlay
    data object Onboarding : Overlay
    /** What Lounge is, where its code lives and who made it. */
    data object About : Overlay
    /** The ways to give to Lounge's maker. */
    data object Donate : Overlay
}

/**
 * The controls pressed whose new state has not come back yet. Each sits half lit until its
 * tile says something different or [SETTLES_MS] passes.
 */
@Stable
class PendingPresses {
    private class Was(val on: Boolean?, val subtitle: String?, val at: Long)

    // One state for each tile, so a press redraws that tile's glyph and no other.
    private val pressed = HashMap<String, MutableState<Was?>>()

    private fun of(id: String): MutableState<Was?> = pressed.getOrPut(id) { mutableStateOf(null) }

    /** Whether [tile] still says what it said when it was pressed. */
    fun waiting(tile: Tile): Boolean = of(tile.id).value?.let { it.on == tile.on && it.subtitle == tile.subtitle } == true

    fun pressed(tile: Tile, now: Long = System.currentTimeMillis()) {
        if (tile.on != null) of(tile.id).value = Was(tile.on, tile.subtitle, now)
    }

    /** A press that failed changes nothing, so its tile stops waiting at once. */
    fun forget(id: String) {
        pressed[id]?.value = null
    }

    /** Drops every press that has waited long enough. */
    fun expire(now: Long = System.currentTimeMillis()) {
        for (state in pressed.values) {
            if (state.value?.let { now - it.at >= SETTLES_MS } == true) state.value = null
        }
    }

    companion object {
        const val SETTLES_MS = 4_000L
    }
}

val LocalPendingPresses = staticCompositionLocalOf { PendingPresses() }

/** Where focus lives on the home screen. The Stage is expanded only while focus is in it. */
enum class FocusRegion { TopBar, Stage, Rows }

/** One requester per top-bar button, so an overlay can hand focus back to the button that opened it. */
@Stable
class TopBarFocus {
    val search = FocusRequester()
    val guide = FocusRequester()
    val tvControls = FocusRequester()
    val nowPlaying = FocusRequester()
    val notifications = FocusRequester()
    val settings = FocusRequester()
    /** The button that last had focus; not read by composition. */
    var last: FocusRequester = search
    /** How far the button at an end of the bar is nudged, in px, read while drawing. */
    val nudge = Animatable(0f)
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
    /** The controls pressed and not yet heard back from. */
    val pending = PendingPresses()
    /** A query the companion asked us to search for, until the search screen takes it. */
    var pendingSearch by mutableStateOf<String?>(null)

    /** What had focus on the home when the open overlay was opened over it. */
    var opener: FocusRequester? = null
    /** The sheets under the open one, so Back returns to the sheet that opened it. */
    val sheetTrail = ArrayList<Overlay>()

    /** The sheet Back last left, so the sheet under it can focus the row that opened it. */
    var sheetLeft: Overlay? = null

    /** The row focus was last on in each plugin sheet, for coming back to it; not read by composition. */
    val pluginSheetRows = HashMap<Overlay.PluginSheet, String>()

    /** The place in the list of the notification last opened, for when it is gone on the way back; not read by composition. */
    var notificationIndex = 0

    /** Whether the setup steps on show were opened from Settings, and so end back there; not read by composition. */
    var setupFromSettings = false

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

    /** How far the lowered Stage has shut, 0 open and 1 gone, for the rows and the Stage to follow in draw. */
    var bandRise: State<Float>? = null

    fun touched() {
        lastInputAt = System.currentTimeMillis()
        idle = false
    }
}
