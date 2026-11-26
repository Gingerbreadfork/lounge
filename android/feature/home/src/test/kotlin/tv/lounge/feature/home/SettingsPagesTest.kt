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
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import tv.lounge.design.LoungeTheme
import tv.lounge.platform.Prefs
import tv.lounge.power.PowerStatus

/** Settings as a list of pages: what each holds, and where focus lands on the way back. */
@RunWith(RobolectricTestRunner::class)
class SettingsPagesTest {
    @get:Rule val rule = createComposeRule()

    private val context = ApplicationProvider.getApplicationContext<android.content.Context>()
    private val prefs = Prefs(context)
    private val opened = mutableListOf<SettingsSection>()

    private fun show(section: SettingsSection?, cameFrom: Overlay? = null) {
        val first = FocusRequester()
        rule.setContent {
            LoungeTheme {
                Column {
                    QuickSettingsSheet(
                        prefs = prefs,
                        section = section,
                        onSection = { opened += it },
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
        shows("Rows", "App groups", "Your apps", "Apps only", "Phone apps in All apps", "Channels from apps")
        rule.onNode(hasText("Theme")).assertDoesNotExist()
    }

    @Test
    fun theLookPageHoldsTheThemeAndTheScreensaver() {
        show(SettingsSection.Look)
        shows("Theme", "Background", "Screensaver", "Lite mode", "Reduce motion")
    }

    @Test
    fun theRemotePageHoldsTheSounds() {
        show(SettingsSection.Remote)
        shows("Sounds")
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
    fun backFromAPageLandsOnItsRowInTheList() {
        show(null, cameFrom = Overlay.SettingsPage(SettingsSection.ThisTv))
        rule.onNode(isFocused()).assert(hasAnyDescendant(hasText("This TV")).or(hasText("This TV")))
    }

    @Test
    fun backFromASheetLandsOnTheRowThatOpenedIt() {
        show(SettingsSection.Look, cameFrom = Overlay.Ambient)
        rule.onNode(isFocused()).assert(hasAnyDescendant(hasText("Screensaver")).or(hasText("Screensaver")))
    }

}
