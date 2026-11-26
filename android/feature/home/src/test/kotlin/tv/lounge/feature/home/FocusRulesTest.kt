package tv.lounge.feature.home

import tv.lounge.power.CecDevice
import androidx.compose.ui.input.key.Key
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.launch
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import tv.lounge.core.CoreException
import tv.lounge.core.GuideProgramme

/** The focus decisions the home makes, and the words it says, as plain rules. */
@RunWith(RobolectricTestRunner::class)
class FocusRulesTest {
    private val resources get() = RuntimeEnvironment.getApplication().resources
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
    fun thePhoneListsOnlyTheTvControlsThisDeviceCanRun() {
        val set = CecDevice(logicalAddress = 0, type = 0, name = "TV", powerStatus = 1, physicalAddress = 0)
        val lounge = CecDevice(logicalAddress = 4, type = 4, name = "Streamer", powerStatus = 0, physicalAddress = 0x1000)
        val console = CecDevice(logicalAddress = 8, type = 4, name = "Console", powerStatus = 1, physicalAddress = 0x2000)
        val cable = listOf(set, lounge, console)

        // Without Power Setup there is nothing to press, and the phone is told where to set it up.
        val none = phoneTvControls(resources, ready = false, hasCec = true, isTelevision = false, devices = cable)
        assertEquals("Power and input need Power Setup. On the TV, open Settings, then Power Setup." to emptyList<String>(), none.note to none.controls.map { it.id })

        // A box plugged into a TV: the sheet's power rows, then the inputs under their own label.
        val box = phoneTvControls(resources, ready = true, hasCec = true, isTelevision = false, devices = cable)
        assertEquals(null, box.note)
        assertEquals(listOf("cec_otp", "cec_tv_power", "sleep", "sleep_timer", "cec_select_4", "cec_select_8"), box.controls.map { it.id })
        assertEquals(listOf("TV is off", "On or off", "Sleep, TV follows", "Off", "On", "Standby"), box.controls.map { it.value })
        assertEquals(listOf(null, null, null, null, "Switch the TV to", "Switch the TV to"), box.controls.map { it.group })
        assertEquals(listOf("Streamer", "Console"), box.controls.drop(4).map { it.title })

        // With nothing else on the cable there is nothing to switch to.
        val alone = phoneTvControls(resources, ready = true, hasCec = true, isTelevision = false, devices = listOf(lounge))
        assertEquals(listOf("cec_otp", "cec_tv_power", "sleep", "sleep_timer"), alone.controls.map { it.id })
        assertEquals(null, alone.controls.first().value)

        // A television cannot wake or switch itself over the cable, and a box without control
        // on its cable cannot reach the TV: each can only sleep.
        val television = phoneTvControls(resources, ready = true, hasCec = true, isTelevision = true, devices = cable)
        assertEquals(listOf("sleep" to "Turn the TV off", "sleep_timer" to "Sleep timer"), television.controls.map { it.id to it.title })
        val deaf = phoneTvControls(resources, ready = true, hasCec = false, isTelevision = false, devices = cable)
        assertEquals(listOf("sleep" to "Go to sleep", "sleep_timer" to "Sleep timer"), deaf.controls.map { it.id to it.title })
    }

    @Test
    fun thePhonesSleepTimerSaysWhatIsLeftToTheMinuteAbove() {
        assertEquals(null, sleepMinutesLeft(null, now = 1_000))
        assertEquals(30, sleepMinutesLeft(1_000 + 30 * 60_000L, now = 1_000))
        assertEquals(30, sleepMinutesLeft(1_000 + 29 * 60_000L + 1, now = 1_000))
        assertEquals(1, sleepMinutesLeft(1_000, now = 5_000))
        val running = phoneTvControls(resources, ready = true, hasCec = false, isTelevision = true, devices = emptyList(), sleepLeft = 28)
        assertEquals("sleep" to "28 min left", running.controls.last().glyph to running.controls.last().value)
        // A timer still running once Power Setup has gone is listed under the note, to be taken back.
        val away = phoneTvControls(resources, ready = false, hasCec = false, isTelevision = true, devices = emptyList(), sleepLeft = 28)
        assertEquals(true to listOf("sleep_timer"), (away.note != null) to away.controls.map { it.id })
    }

    @Test
    fun theLastSetupStepNamesTheTopBarThisTvHas() {
        assertEquals("Up from the Stage reaches Search and Settings.", resources.getString(topBarWords(showGuide = false, showTvControls = false)))
        assertEquals("Up from the Stage reaches Search, TV controls and Settings.", resources.getString(topBarWords(showGuide = false, showTvControls = true)))
        assertEquals("Up from the Stage reaches Search, the TV guide and Settings.", resources.getString(topBarWords(showGuide = true, showTvControls = false)))
        assertEquals("Up from the Stage reaches Search, the TV guide, TV controls and Settings.", resources.getString(topBarWords(showGuide = true, showTvControls = true)))
    }

    @Test
    fun backFromEachRegion() {
        assertEquals(HomeBack.ToStage, homeBack(FocusRegion.Rows, playbillFocused = false))
        assertEquals(HomeBack.ToStage, homeBack(FocusRegion.TopBar, playbillFocused = false))
        assertEquals(HomeBack.ToStrip, homeBack(FocusRegion.Stage, playbillFocused = true))
        assertEquals(HomeBack.Stay, homeBack(FocusRegion.Stage, playbillFocused = false))
    }

    @Test
    fun theNotificationsPanelLeavesWithItsLastNotification() {
        val page = Overlay.NotificationDetail("b")
        assertEquals(PanelChange.Stay, panelAfter(Overlay.Notifications, listOf("a")))
        assertEquals(PanelChange.Stay, panelAfter(page, listOf("a", "b")))
        // The one on show has gone and others remain: back to the list.
        assertEquals(PanelChange.ToList, panelAfter(page, listOf("a")))
        assertEquals(PanelChange.Close, panelAfter(page, emptyList()))
        assertEquals(PanelChange.Close, panelAfter(Overlay.Notifications, emptyList()))
        // Nothing else is moved by a notification coming or going.
        assertEquals(PanelChange.Stay, panelAfter(Overlay.None, emptyList()))
        assertEquals(PanelChange.Stay, panelAfter(Overlay.NotificationSettings, emptyList()))
        assertEquals(PanelChange.Stay, panelAfter(Overlay.QuickSettings, emptyList()))
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

        val rows = search(snapshot, "st", testSearchTitles)
        assertEquals(listOf("allapps", "continue"), rows.map { it.id })
        assertEquals("Apps", rows[0].title)
        assertEquals(listOf("Stan", "Google Play Store"), rows[0].tiles.map { it.title })

        // Episode codes never match, and neither does a single letter on the second line.
        assertTrue(search(snapshot, "e16", testSearchTitles).isEmpty())
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
        assertEquals(listOf("continue"), search(snapshot, "sev", testSearchTitles).map { it.id })
        assertEquals(listOf("app:x"), search(snapshot, "xb", testSearchTitles).single().tiles.map { it.id })
        // Being hidden is a state the tile wears and says, not words to be found by.
        assertTrue(search(snapshot, "hi", testSearchTitles).isEmpty())
        assertTrue(search(snapshot, "ro", testSearchTitles).isEmpty())
        assertEquals("Xbox, All apps, hidden from every row", tileDescription(resources, hidden, "All apps"))
    }

    @Test
    fun aFailureSaidInAnotherLanguageIsStillKnown() {
        val context = org.robolectric.RuntimeEnvironment.getApplication()
        val german = context.createConfigurationContext(android.content.res.Configuration().apply { setLocale(java.util.Locale.GERMAN) }).resources
        // The platform words its failures in the language in use, and the home knows them by those words.
        val said = german.getString(tv.lounge.platform.R.string.android_platform_no_video_player)
        assertTrue(said != resources.getString(tv.lounge.platform.R.string.android_platform_no_video_player))
        assertEquals(german.getString(R.string.home_screen_error_no_video_player, "Ein Film"), friendlyReason(german, CoreException.Platform(said), said, "Ein Film"))
        val playing = german.getString(tv.lounge.platform.R.string.android_platform_not_playing)
        assertEquals(german.getString(R.string.home_screen_error_not_playing, "Radio"), friendlyReason(german, CoreException.Platform(playing), playing, "Radio"))
        // Episode codes are read as words whichever letters the language writes them with.
        assertEquals(german.getString(R.string.tiles_spoken_episode, "2", "4") + ", Honeydew", spokenEpisodes(german, "S2 F4, Honeydew"))
        assertEquals(resources.getString(R.string.tiles_spoken_episode, "2", "4"), spokenEpisodes(resources, "T2 E4"))
        assertEquals("Mix: A1 B2", spokenEpisodes(resources, "Mix: A1 B2"))
    }

    @Test
    fun failuresAreSentencesWithTheirFix() {
        // What the TV says when nothing can play a stream names the kind of player to install.
        val film = "No video player can open this stream. Install VLC or another player."
        assertEquals("Nothing on this TV can play A film. Install VLC or another video player, then try again.", friendlyReason(resources, CoreException.Platform(film), film, "A film"))
        val station = "No player can open this stream. Install VLC or another player."
        assertEquals("Nothing on this TV can play A station. Install VLC or another player, then try again.", friendlyReason(resources, CoreException.Platform(station), station, "A station"))
        val launch = friendlyReason(resources, CoreException.Platform("No launch intent for com.netflix.ninja"), "No launch intent for com.netflix.ninja", "Netflix")
        assertEquals("Nothing on this TV can open Netflix. Is its app still installed?", launch)
        val unknown = CoreException.UnknownAction("open:wn:42")
        assertEquals("Alien: Earth isn't on this TV any more. Its app may have removed it.", friendlyReason(resources, unknown, "no such item or action: open:wn:42", "Alien: Earth"))
        // A plugin's sentence and the host's show as written, whatever words they hold.
        for (written in listOf(
            "Kodi has no episodes of this show. Update Kodi's library, then try again.",
            "Kodi stopped before it could open this. Try again in a moment.",
            "Jellyfin didn't answer at 192.168.0.20:8096. Check the server, then try again.",
            "Its plugin is turned off or no longer installed. Turn the plugin on, then try again.",
        )) {
            assertEquals(written, friendlyReason(resources, CoreException.Plugin(written), written, "The Bear"))
        }
        // What is not a sentence is worded here.
        val late = CoreException.Plugin("tv.lounge.kodi did not answer in time")
        assertEquals("The Bear's source didn't answer. Check that it is running, then try again.", friendlyReason(resources, late, late.detail, "The Bear"))
        val gone = CoreException.Plugin("plugin thread is gone")
        assertEquals("Static Bloom's plugin stopped partway through. Try again in a moment.", friendlyReason(resources, gone, gone.detail, "Static Bloom"))
        // Text not known here never reaches the screen.
        val odd = RuntimeException("java.lang.IllegalStateException: Binder transaction failed at 0x7f")
        val said = friendlyReason(resources, odd, odd.message!!, "The Bear")
        assertEquals("Couldn't open The Bear. Try again in a moment.", said)
        assertFalse("Binder" in said)
    }

    @Test
    fun aPairingFailureIsTheCoresSentenceWhenItWroteOne() {
        for (written in listOf("The TV isn't on a network. Connect it to your Wi-Fi, then try again.", "The phone companion is off. Turn it on, then try again.")) {
            assertTrue(readsAsSentence(written))
        }
        for (raw in listOf("the TV's random number source failed, so no code was made", "io: No space left on device (os error 28)", "")) {
            assertFalse(readsAsSentence(raw))
        }
    }

    @Test
    fun theCodeShownIsTheOneTheCoreOffersAtThisAddress() {
        val code = tv.lounge.core.PairingInfo(code = "123456", url = "http://192.168.0.20:8787/#token=abc", qrPngPath = "/cache/pair.png", expiresMs = 1L)
        val status = tv.lounge.core.CompanionStatus(running = true, url = "http://192.168.0.20:8787", port = 8787u, paired = 0u, pairing = code)
        assertEquals(code, status.livePairing())
        // None to show, off the network, or made for an address the TV has since left.
        assertNull(status.copy(pairing = null).livePairing())
        assertNull(status.copy(url = null).livePairing())
        assertNull(status.copy(url = "http://192.168.0.31:8787").livePairing())
    }

    @Test
    fun textFromThePhoneNeedsAFieldOpenToTakeIt() = kotlinx.coroutines.runBlocking {
        val typed = kotlinx.coroutines.flow.MutableSharedFlow<String>(extraBufferCapacity = 8)
        val phone = PhoneText(typed)
        assertFalse(phone.fieldOpen)
        val got = ArrayList<String>()
        val field = launch(kotlinx.coroutines.Dispatchers.Unconfined) { phone.into { got += it } }
        assertTrue(phone.fieldOpen)
        typed.emit("bear")
        kotlinx.coroutines.yield()
        assertEquals(listOf("bear"), got)
        field.cancelAndJoin()
        assertFalse(phone.fieldOpen)
    }

    @Test
    fun theProgrammeOnAirSaysSoInWords() {
        val start = 1_700_000_000_000L
        val on = GuideProgramme(id = "p", title = "News", subtitle = null, startMs = start, endMs = start + 1_800_000, isNow = true)
        assertTrue(programmeTimes(resources, on, is24Hour = true).startsWith("On now, until "))
        val later = on.copy(isNow = false)
        assertTrue(" to " in programmeTimes(resources, later, is24Hour = true))
    }

    @Test
    fun episodeCodesAreReadAsWords() {
        assertEquals("Season 2, Episode 4, Netflix", spokenEpisodes(resources, "S2 E4, Netflix"))
        assertEquals("Honeydew", spokenEpisodes(resources, "Honeydew"))
        val tile = testTile("wn:1", "The Bear", subtitle = "S2 E4, Netflix").copy(timeLeft = "23 minutes left")
        assertEquals("The Bear, Continue watching, Season 2, Episode 4, Netflix, 23 minutes left", tileDescription(resources, tile, "Continue watching"))
    }

    @Test
    fun aStripTileIsReadAsThePlaybillSaysItAndSaysWhenItIsPinned() {
        val sp = tv.lounge.core.StageSpotlight(
            id = "wn:1", kind = tv.lounge.core.SpotlightKind.CONTENT, category = "Continue watching", title = "The Bear", lines = listOf("S2 E4"),
            progress = null, artworkPath = null, backdropPath = null, iconPath = null, lightArgb = 0u,
            primary = tv.lounge.core.StageAction(label = "Continue", action = "open:wn:1"), secondary = null, explanation = null,
            framed = false, artAspect = 0f, glowArgb = emptyList(), focalY = 0.5f, source = "Netflix", sourceIconPath = null,
        )
        assertEquals("The Bear, Continue watching, Season 2, Episode 4, on Netflix", spotlightDescription(resources, sp))
        assertEquals("The Bear, Continue watching, Season 2, Episode 4, on Netflix, pinned to the Stage", spotlightDescription(resources, sp, pinned = true))
    }

    @Test
    fun onlyTilesWithSomethingToPinOrHideHaveAMenu() {
        assertTrue(hasMenu(testTile("app:tv.a")))
        assertFalse(hasMenu(testTile("control:allapps")))
        assertFalse(hasMenu(testTile("device:tv")))
        assertFalse(hasMenu(testTile("search:plex:1")))
    }
}
