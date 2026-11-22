package tv.lounge.feature.home

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import tv.lounge.core.Tile
import tv.lounge.core.TileKind
import tv.lounge.design.LoungeGlyphs

/** A control tile: how it is drawn, what holding OK offers and how a press waits for its answer. */
@RunWith(RobolectricTestRunner::class)
class ControlsTest {
    private val resources get() = RuntimeEnvironment.getApplication().resources

    private fun lamp(on: Boolean? = false, subtitle: String? = "Off") =
        testTile("plugin:h:lamp", title = "Lamp", subtitle = subtitle, kind = TileKind.COMPACT, action = "open:plugin:h:lamp", glyph = "light", on = on, more = "h:lamp")

    private fun menu(tile: Tile, stagePinned: Boolean = false): List<String> = menuItemsFor(
        tile = tile,
        rowId = "plugin:h:here",
        rowTitle = "Den",
        isPinned = false,
        canMoveLeft = false,
        canMoveRight = false,
        isStagePinned = stagePinned,
        onHide = {},
        onShow = {},
        onHideAllFrom = {},
        onHideRow = {},
        onPin = {},
        onUnpin = {},
        onMoveLeft = {},
        onMoveRight = {},
        onPinStage = {},
        onUnpinStage = {},
        onAppInfo = {},
        onUninstall = {},
        onPlay = {},
        onDismissNotice = {},
        onGuide = {},
    ).map { resources.getString(it.label) }

    @Test
    fun aGlyphIsLitByItsStateAndHalfLitWhileItWaits() {
        assertEquals(1f, glyphLevel(on = true, pending = false))
        assertEquals(0f, glyphLevel(on = false, pending = false))
        assertEquals(0f, glyphLevel(on = null, pending = false))
        assertEquals(0.5f, glyphLevel(on = true, pending = true))
        assertEquals(0.5f, glyphLevel(on = false, pending = true))
    }

    @Test
    fun everyGlyphNameDrawsAndAnUnknownOneDoesNot() {
        assertTrue(LoungeGlyphs.names.isNotEmpty())
        for (name in LoungeGlyphs.names) assertNotNull(name, LoungeGlyphs.named(name))
        assertNull(LoungeGlyphs.named("hovercraft"))
        assertNull(LoungeGlyphs.named(null))
        assertNotNull(LoungeGlyphs.Thing)
    }

    @Test
    fun aPressWaitsUntilItsTileSaysSomethingNew() {
        val presses = PendingPresses()
        val off = lamp()
        assertFalse(presses.waiting(off))
        presses.pressed(off, now = 0)
        assertTrue(presses.waiting(off))
        // The new state ends the wait, and so does a new second line on the same state.
        assertFalse(presses.waiting(lamp(on = true, subtitle = "On")))
        assertFalse(presses.waiting(lamp(on = false, subtitle = "Not answering")))
        // Pressed again while on, it waits to go off.
        val on = lamp(on = true, subtitle = "On")
        presses.pressed(on, now = 500)
        assertTrue(presses.waiting(on))
        assertFalse(presses.waiting(off))
    }

    @Test
    fun aPressThatFailedOrGrewOldStopsWaiting() {
        val presses = PendingPresses()
        val off = lamp()
        presses.pressed(off, now = 0)
        presses.forget(off.id)
        assertFalse(presses.waiting(off))

        presses.pressed(off, now = 0)
        presses.expire(now = PendingPresses.SETTLES_MS - 1)
        assertTrue(presses.waiting(off))
        presses.expire(now = PendingPresses.SETTLES_MS)
        assertFalse(presses.waiting(off))

        // A scene has no state to wait for.
        val scene = lamp(on = null, subtitle = null)
        presses.pressed(scene, now = 0)
        assertFalse(presses.waiting(scene))
    }

    @Test
    fun aControlOffersAdjustFirstAndNoPlaceOnTheStage() {
        assertEquals(listOf("Adjust", "Hide", "Hide this row"), menu(lamp()))
        // Without a sheet of its own it has nothing to adjust.
        assertEquals(listOf("Hide", "Hide this row"), menu(lamp().copy(more = null)))
        // One pinned to the Stage before it became a control can still be taken off it.
        assertEquals(listOf("Adjust", "Unpin from the Stage", "Hide", "Hide this row"), menu(lamp(), stagePinned = true))
        // Anything else that opens keeps its place on the Stage.
        assertTrue("Pin to the Stage" in menu(testTile("plugin:h:film", kind = TileKind.LANDSCAPE)))
    }

    @Test
    fun aPluginsResultAlreadyFoundOnTheHomeIsShownOnce() {
        val home = testRow("plugin:h:here", lamp(), title = "Den", kind = TileKind.COMPACT)
        val same = testTile("search:h:lamp", title = "Lamp", kind = TileKind.COMPACT, glyph = "light", on = false)
        val other = testTile("search:h:porch", title = "Porch lamp", kind = TileKind.COMPACT, glyph = "light", on = false)
        val merged = mergedResults(listOf(home), listOf(testRow("search:plugin:h", same, other, title = "Home")))
        assertEquals(listOf(listOf("plugin:h:lamp"), listOf("search:h:porch")), merged.map { row -> row.tiles.map { it.id } })
        // A plugin with nothing new to add brings no row.
        assertEquals(listOf("plugin:h:here"), mergedResults(listOf(home), listOf(testRow("search:plugin:h", same))).map { it.id })
        assertEquals(listOf("search:plugin:h"), mergedResults(emptyList(), listOf(testRow("search:plugin:h", same))).map { it.id })
    }

    @Test
    fun searchFindsATileThatOpensASheetAndNotOneThatLeadsAway() {
        val room = testTile("plugin:h:den", title = "Den", kind = TileKind.COMPACT, action = "control:sheet:h:room:den", glyph = "home")
        val allApps = testTile("control:allapps", title = "Den of apps", action = "control:allapps")
        val snapshot = testSnapshot(rows = listOf(testRow("plugin:h:rooms", room, allApps, kind = TileKind.COMPACT)))
        assertEquals(listOf("plugin:h:den"), search(snapshot, "den", testSearchTitles).flatMap { it.tiles }.map { it.id })
    }
}
