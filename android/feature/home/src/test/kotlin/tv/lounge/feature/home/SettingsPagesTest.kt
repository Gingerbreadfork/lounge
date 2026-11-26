package tv.lounge.feature.home

import androidx.compose.foundation.layout.Column
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.hasAnyDescendant
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isFocused
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.performSemanticsAction
import androidx.test.core.app.ApplicationProvider
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import tv.lounge.core.RowSize
import tv.lounge.design.LoungeTheme
import tv.lounge.platform.Prefs
import tv.lounge.power.PowerStatus

/** Settings as a list of pages: what each holds, where focus lands on the way back, and what the newer switches do. */
@RunWith(RobolectricTestRunner::class)
class SettingsPagesTest {
    @get:Rule val rule = createComposeRule()

    private val context = ApplicationProvider.getApplicationContext<android.content.Context>()
    private val prefs = Prefs(context)
    private val opened = mutableListOf<SettingsSection>()

    private fun show(section: SettingsSection?, cameFrom: Overlay? = null, ui: Prefs.UiPrefs = Prefs.UiPrefs()) {
        val first = FocusRequester()
        rule.setContent {
            LoungeTheme {
                Column {
                    QuickSettingsSheet(
                        prefs = prefs,
                        section = section,
                        onSection = { opened += it },
                        ui = ui,
                        theme = Prefs.Theme.Dusk,
                        sounds = true,
                        lite = null,
                        reducedMotion = null,
                        showMobileApps = false,
                        isDefaultHome = true,
                        enabledChannels = 0,
                        enabledPlugins = 2,
                        diagnostics = { error("not asked for") },
                        onSetTheme = {},
                        onSetSounds = {},
                        onSetLite = {},
                        onSetReducedMotion = {},
                        onSetShowMobileApps = {},
                        onRequestPermission = {},
                        say = {},
                        onChannels = {},
                        onRows = {},
                        rowsSummary = "All shown",
                        onGroups = {},
                        groupsSummary = "None",
                        recentApps = 5,
                        onSetRecentApps = {},
                        onPlugins = {},
                        onCompanion = {},
                        companionOn = false,
                        onRecheckHome = {},
                        onPowerSetup = {},
                        powerStatus = PowerStatus.NotPaired,
                        onAmbient = {},
                        screensaver = { TvScreensaver.Lounge },
                        onSetupAgain = {},
                        onAbout = {},
                        onDonate = {},
                        onUpdates = {},
                        updateStatus = UpdateWord(R.string.updates_word_up_to_date),
                        firstFocus = first,
                        cameFrom = cameFrom,
                    )
                }
            }
        }
        rule.runOnIdle { runCatching { first.requestFocus() } }
        rule.waitForIdle()
    }

    /** Presses OK on the row with this title. */
    private fun press(title: String) =
        rule.onNode(hasClickAction() and hasAnyDescendant(hasText(title))).performSemanticsAction(SemanticsActions.OnClick)

    private fun shows(vararg titles: String) = titles.forEach { rule.onNode(hasText(it)).assertExists() }

    @Test
    fun settingsOpensOnAShortListOfPages() {
        show(null)
        shows("Home screen", "Look", "Remote and sounds", "Plugins", "Notifications", "Phone companion", "Access", "This TV", "About and updates")
        // Each page says what is in it, and none of a page's rows is on the list.
        shows("Theme, background, sizes, the clock and the screensaver")
        rule.onNode(hasText("Theme")).assertDoesNotExist()
        rule.onNode(isFocused()).assert(hasAnyDescendant(hasText("Home screen")).or(hasText("Home screen")))
        press("Look")
        assertEquals(listOf(SettingsSection.Look), opened)
    }

    @Test
    fun eachPageHoldsItsOwnRows() {
        show(SettingsSection.Home)
        shows("Rows", "App groups", "Your apps", "The Stage", "Apps only", "Phone apps in All apps", "Channels from apps", "When the TV starts")
        rule.onNode(hasText("Theme")).assertDoesNotExist()
    }

    @Test
    fun theLookPageHoldsTheSizesTheClockAndTheScreensaver() {
        show(SettingsSection.Look)
        shows("Theme", "Background", "Text size", "Tile size", "Clock", "Screensaver", "Lite mode", "Reduce motion")
    }

    @Test
    fun theRemotePageHoldsTheHoldTimeItsHintAndTheSounds() {
        show(SettingsSection.Remote)
        shows("Hold time for OK", "Hold OK hint", "Sounds")
    }

    @Test
    fun theAboutPageHoldsLoungeItself() {
        show(SettingsSection.Lounge)
        shows("Updates", "About Lounge", "Support Lounge", "Run the setup steps again", "Diagnostics")
    }

    @Test
    fun theAccessPageHoldsWhatLoungeMayDo() {
        show(SettingsSection.Access)
        shows("Default home screen", "Power Setup")
        rule.onNode(hasText("Language")).assertDoesNotExist()
    }

    @Test
    fun theTvPageOpensOnLanguage() {
        show(SettingsSection.ThisTv)
        shows("Language")
        rule.onNode(isFocused()).assert(hasAnyDescendant(hasText("Language")).or(hasText("Language")))
    }

    @Test
    fun whatTheTvNoLongerHasCannotBeStartedWith() {
        val snapshot = testSnapshot(allApps = listOf(testTile("app:tv.kodi", title = "Kodi")))
        assertTrue(canStartWith(snapshot, "open:app:tv.kodi"))
        assertFalse(canStartWith(snapshot, "open:app:tv.gone"))
    }

    @Test
    fun backFromAPageLandsOnItsRowInTheList() {
        show(null, cameFrom = Overlay.SettingsPage(SettingsSection.ThisTv))
        rule.onNode(isFocused()).assert(hasAnyDescendant(hasText("This TV")).or(hasText("This TV")))
    }

    @Test
    fun backFromASheetLandsOnTheRowThatOpenedIt() {
        show(SettingsSection.Look, cameFrom = Overlay.Ambient)
        rule.onNode(isFocused()).assert(hasAnyDescendant(hasText("Screensaver")).or(hasText("Screensaver")))
    }

    @Test
    fun aSwitchIsSavedWhenItsRowIsPressed() {
        show(SettingsSection.Look, ui = Prefs.UiPrefs(showClock = true))
        try {
            press("Clock")
            rule.waitUntil(5_000) { runBlocking { !prefs.all.first().showClock } }
        } finally {
            // The store is shared by every test here, so it is left as a new TV has it.
            runBlocking { prefs.setShowClock(true) }
        }
    }

    @Test
    fun theHomePageNamesWhatOpensAtStart() {
        show(SettingsSection.Home, ui = Prefs.UiPrefs(startAction = "open:app:tv.kodi", startTitle = "Kodi", showStage = false))
        shows("Kodi")
    }

    @Test
    fun withoutTheStageNothingNewIsPinnedToIt() {
        fun menu(stageShown: Boolean) = menuItemsFor(
            tile = testTile("app:tv.netflix", title = "Netflix"),
            rowId = "apps",
            rowTitle = "Your apps",
            isPinned = false,
            canMoveLeft = false,
            canMoveRight = false,
            isStagePinned = false,
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
            stageShown = stageShown,
        ).map { context.resources.getString(it.label) }
        assertTrue("Pin to the Stage" in menu(stageShown = true))
        assertFalse("Pin to the Stage" in menu(stageShown = false))
    }

    @Test
    fun largeTilesAreASizeUpWhereThereIsOne() {
        assertEquals(listOf(RowSize.M, RowSize.L, RowSize.L), listOf(RowSize.S, RowSize.M, RowSize.L).map { it.larger() })
    }

    @Test
    fun theInputsToStartWithLeaveOutTheRowsOwnControl() {
        val row = testRow("tvcontrols", testTile("input:hdmi1", title = "HDMI 1"), testTile("control:tv", title = "TV controls", action = "control:tv:open"), title = "Inputs")
        assertEquals(listOf("HDMI 1"), inputsOf(testSnapshot(rows = listOf(row))).map { it.title })
        assertEquals(emptyList<String>(), inputsOf(testSnapshot()).map { it.title })
    }
}
