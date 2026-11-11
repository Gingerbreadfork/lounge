package tv.lounge.feature.home

import androidx.compose.ui.input.key.Key
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import tv.lounge.core.CoreException
import tv.lounge.core.GuideProgramme

/** The focus decisions the home makes, and the words it says, as plain rules. */
class FocusRulesTest {
    private val first = Any()
    private val middle = Any()
    private val last = Any()

    @Test
    fun theTopBarsEndsKeepFocusInTheBar() {
        assertEquals(-1, topBarEdge(Key.DirectionLeft, first, first, last))
        assertEquals(1, topBarEdge(Key.DirectionRight, last, first, last))
        assertEquals(0, topBarEdge(Key.DirectionRight, first, first, last))
        assertEquals(0, topBarEdge(Key.DirectionLeft, middle, first, last))
        assertEquals(0, topBarEdge(Key.DirectionDown, first, first, last))
    }

    @Test
    fun backFromEachRegion() {
        assertEquals(HomeBack.ToStage, homeBack(FocusRegion.Rows, playbillFocused = false))
        assertEquals(HomeBack.ToStage, homeBack(FocusRegion.TopBar, playbillFocused = false))
        assertEquals(HomeBack.ToStrip, homeBack(FocusRegion.Stage, playbillFocused = true))
        assertEquals(HomeBack.Stay, homeBack(FocusRegion.Stage, playbillFocused = false))
    }

    @Test
    fun aHiddenTileHandsFocusToItsNeighbour() {
        val a = testTile("a")
        val b = testTile("b")
        val c = testTile("c")
        val before = listOf("continue", "apps", "games")
        // Still there, moved along its row.
        assertEquals(RowFocus.Stays(0), rowFocusAfter(before, listOf(testRow("apps", b, a)), "apps", "b", 1))
        // Gone: the tile that took its place, or the last when it was the last.
        assertEquals(RowFocus.Neighbour(1), rowFocusAfter(before, listOf(testRow("apps", a, c)), "apps", "b", 1))
        assertEquals(RowFocus.Neighbour(1), rowFocusAfter(before, listOf(testRow("apps", a, b)), "apps", "c", 2))
        // Its row went too: the row now in that row's place.
        val rows = listOf(testRow("continue", a), testRow("games", c))
        assertEquals(RowFocus.NextRow("games"), rowFocusAfter(before, rows, "apps", "b", 0))
        // Nothing left: the Stage.
        assertEquals(RowFocus.Stage, rowFocusAfter(before, emptyList(), "apps", "b", 0))
    }

    @Test
    fun searchPutsTheBestMatchesFirst() {
        val stan = testTile("app:stan", "Stan")
        val store = testTile("app:store", "Google Play Store")
        val testaments = testTile("wn:1", "The Testaments", subtitle = "S1 E7, Jellyfin", kind = tv.lounge.core.TileKind.LANDSCAPE)
        val crystal = testTile("wn:2", "Crystal Lake", subtitle = "S5 E16, Jellyfin", kind = tv.lounge.core.TileKind.LANDSCAPE)
        val snapshot = testSnapshot(rows = listOf(testRow("continue", testaments, crystal, title = "Continue watching")), allApps = listOf(store, stan))

        val rows = search(snapshot, "st")
        assertEquals(listOf("allapps", "continue"), rows.map { it.id })
        assertEquals("Apps", rows[0].title)
        assertEquals(listOf("Stan", "Google Play Store"), rows[0].tiles.map { it.title })

        // Episode codes never match, and neither does a single letter on the second line.
        assertTrue(search(snapshot, "e16").isEmpty())
        assertEquals(null, matchScore("Crystal Lake", "S5 E16, Jellyfin", searchWords("j")))
        assertEquals(1, matchScore("Crystal Lake", "S5 E16, Jellyfin", searchWords("jelly")))
        // Title first, then a word of it, then anywhere in it.
        assertEquals(3, matchScore("The Bear", null, searchWords("the be")))
        assertEquals(2, matchScore("The Bear", null, searchWords("bear")))
        assertEquals(1, matchScore("The Bear", null, searchWords("ear")))
        assertEquals(3, matchScore("Pokémon", null, searchWords("pokemon")))
        assertEquals(2, matchScore("Grey's Anatomy", null, searchWords("anatomy greys")))
        assertNull(matchScore("The Bear", null, searchWords("bear xyz")))
    }

    @Test
    fun searchFindsWhatTheStageShowsAndHiddenApps() {
        val onStage = testTile("wn:1", "Severance", kind = tv.lounge.core.TileKind.LANDSCAPE, provider = "tv.apple")
        val hidden = testTile("app:x", "Xbox", hidden = true)
        val snapshot = testSnapshot(strip = listOf(onStage), allApps = listOf(hidden))
        assertEquals(listOf("continue"), search(snapshot, "sev").map { it.id })
        assertEquals(listOf("app:x"), search(snapshot, "xb").single().tiles.map { it.id })
    }

    @Test
    fun failuresAreSentencesWithTheirFix() {
        val launch = friendlyReason(CoreException.Platform("No launch intent for com.netflix.ninja"), "No launch intent for com.netflix.ninja", "Netflix")
        assertEquals("Nothing on this TV can open Netflix. Is its app still installed?", launch)
        val unknown = CoreException.UnknownAction("open:wn:42")
        assertEquals("Alien: Earth isn't on this TV any more. Its app may have removed it.", friendlyReason(unknown, "no such item or action: open:wn:42", "Alien: Earth"))
        // A plugin's sentence and the host's show as written, whatever words they hold.
        for (written in listOf(
            "Kodi has no episodes of this show. Update Kodi's library, then try again.",
            "Kodi stopped before it could open this. Try again in a moment.",
            "Jellyfin didn't answer at 192.168.0.20:8096. Check the server, then try again.",
            "Its plugin is turned off or no longer installed. Turn the plugin on, then try again.",
        )) {
            assertEquals(written, friendlyReason(CoreException.Plugin(written), written, "The Bear"))
        }
        // What is not a sentence is worded here.
        val late = CoreException.Plugin("tv.lounge.kodi did not answer in time")
        assertEquals("The Bear's source didn't answer. Check that it is running, then try again.", friendlyReason(late, late.detail, "The Bear"))
        val gone = CoreException.Plugin("plugin thread is gone")
        assertEquals("Static Bloom's plugin stopped partway through. Try again in a moment.", friendlyReason(gone, gone.detail, "Static Bloom"))
        // Text not known here never reaches the screen.
        val odd = RuntimeException("java.lang.IllegalStateException: Binder transaction failed at 0x7f")
        val said = friendlyReason(odd, odd.message!!, "The Bear")
        assertEquals("Couldn't open The Bear. Try again in a moment.", said)
        assertFalse("Binder" in said)
    }

    @Test
    fun theProgrammeOnAirSaysSoInWords() {
        val start = 1_700_000_000_000L
        val on = GuideProgramme(id = "p", title = "News", subtitle = null, startMs = start, endMs = start + 1_800_000, isNow = true)
        assertTrue(programmeTimes(on, is24Hour = true).startsWith("On now, until "))
        val later = on.copy(isNow = false)
        assertTrue(" to " in programmeTimes(later, is24Hour = true))
    }

    @Test
    fun episodeCodesAreReadAsWords() {
        assertEquals("Season 2, Episode 4, Netflix", spokenEpisodes("S2 E4, Netflix"))
        assertEquals("Honeydew", spokenEpisodes("Honeydew"))
        val tile = testTile("wn:1", "The Bear", subtitle = "S2 E4, Netflix").copy(timeLeft = "23 minutes left")
        assertEquals("The Bear, Continue watching, Season 2, Episode 4, Netflix, 23 minutes left", tileDescription(tile, "Continue watching"))
    }

    @Test
    fun onlyTilesWithSomethingToPinOrHideHaveAMenu() {
        assertTrue(hasMenu(testTile("app:tv.a")))
        assertFalse(hasMenu(testTile("control:allapps")))
        assertFalse(hasMenu(testTile("device:tv")))
        assertFalse(hasMenu(testTile("search:plex:1")))
    }
}
