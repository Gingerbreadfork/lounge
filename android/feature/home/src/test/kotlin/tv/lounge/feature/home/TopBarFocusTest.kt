package tv.lounge.feature.home

import androidx.compose.foundation.focusable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsFocused
import androidx.compose.ui.test.hasAnyDescendant
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isFocused
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.pressKey
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.flow.MutableStateFlow
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import tv.lounge.design.LoungeTheme
import tv.lounge.platform.AppNotification

/** The top bar keeps focus at its ends (DESIGN.md §5.3) and hands Down to what is below it. */
@OptIn(ExperimentalTestApi::class)
@RunWith(RobolectricTestRunner::class)
class TopBarFocusTest {
    @get:Rule val rule = createComposeRule()

    private val focus = TopBarFocus()
    private val below = FocusRequester()
    private var searches = 0
    private val notices = MutableStateFlow(NotificationsShown())
    private var opened = 0

    private fun notice(key: String) = AppNotification(key, "tv.shop", "Shop", "Title $key", "", postedAt = 0, dismissible = true, opens = true)

    /**
     * The top bar with focusable boxes where the playbill's button and the strip's last tile sit:
     * below it and off to either side, where a d-pad search from an end would otherwise land.
     */
    private fun showTopBar() {
        rule.setContent {
            LoungeTheme {
                Box(Modifier.fillMaxSize()) {
                    Box(Modifier.offset(x = 96.dp, y = 300.dp).size(200.dp, 56.dp).testTag("left below").focusRequester(below).focusable())
                    Box(Modifier.offset(x = 1700.dp, y = 300.dp).size(400.dp, 130.dp).testTag("right below").focusable())
                    TopBar(
                        showTvControls = true,
                        showGuide = true,
                        nowPlaying = null,
                        notifications = notices,
                        focus = focus,
                        downFocus = { below },
                        onFocused = {},
                        onSearch = { searches++ },
                        onTvControls = {},
                        onGuide = {},
                        onNowPlaying = {},
                        onNotifications = { opened++ },
                        onSettings = {},
                    )
                }
            }
        }
    }

    private fun press(key: Key) {
        rule.onNode(isFocused()).performKeyInput { pressKey(key) }
        rule.waitForIdle()
    }

    private fun assertFocusedButton(label: String) = rule.onNode(isFocused()).assert(hasAnyDescendant(hasText(label)))

    @Test
    fun leftFromTheFirstButtonStaysInTheTopBar() {
        showTopBar()
        rule.runOnIdle { focus.search.requestFocus() }
        press(Key.DirectionLeft)
        assertFocusedButton("Search")
    }

    @Test
    fun rightFromTheLastButtonStaysInTheTopBar() {
        showTopBar()
        rule.runOnIdle { focus.settings.requestFocus() }
        press(Key.DirectionRight)
        assertFocusedButton("Settings")
    }

    @Test
    fun leftAndRightMoveAlongTheBar() {
        showTopBar()
        rule.runOnIdle { focus.search.requestFocus() }
        press(Key.DirectionRight)
        assertFocusedButton("TV guide")
        press(Key.DirectionRight)
        press(Key.DirectionRight)
        assertFocusedButton("Settings")
        press(Key.DirectionLeft)
        assertFocusedButton("TV controls")
    }

    @Test
    fun theNotificationsChipSitsBeforeSettingsWhileThereIsSomethingToRead() {
        showTopBar()
        rule.runOnIdle { focus.settings.requestFocus() }
        press(Key.DirectionLeft)
        assertFocusedButton("TV controls")

        notices.value = NotificationsShown(listOf(notice("a"), notice("b")), fresh = 1)
        rule.waitForIdle()
        press(Key.DirectionRight)
        rule.onNode(isFocused()).assert(hasContentDescription("2 notifications, 1 new"))
        press(Key.DirectionCenter)
        rule.runOnIdle { assertEquals(1, opened) }
        // Settings is still the end of the bar.
        press(Key.DirectionRight)
        assertFocusedButton("Settings")
        press(Key.DirectionRight)
        assertFocusedButton("Settings")

        // Once they have been seen the chip says how many and no more, and it leaves with the last.
        notices.value = NotificationsShown(listOf(notice("a")), fresh = 0)
        rule.waitForIdle()
        rule.onNode(hasContentDescription("1 notification")).assertExists()
        notices.value = NotificationsShown()
        rule.waitForIdle()
        rule.onNode(hasContentDescription("1 notification")).assertDoesNotExist()
        press(Key.DirectionLeft)
        assertFocusedButton("TV controls")
    }

    @Test
    fun aChipThatLeavesWhileItIsTheBarsRememberedButtonHandsItsPlaceToSettings() {
        showTopBar()
        notices.value = NotificationsShown(listOf(notice("a")), fresh = 1)
        rule.waitForIdle()
        rule.runOnIdle { focus.notifications.requestFocus() }
        rule.onNode(isFocused()).assert(hasContentDescription("1 notification, 1 new"))
        // Focus goes elsewhere, as into the panel.
        press(Key.DirectionDown)
        rule.onNodeWithTag("left below").assertIsFocused()
        // The last notification is dismissed there, and focus is sent to Settings in the same
        // moment, before the bar has let go of the chip.
        rule.runOnIdle {
            notices.value = NotificationsShown()
            focus.settings.requestFocus()
        }
        rule.waitForIdle()
        assertFocusedButton("Settings")
    }

    @Test
    fun downLeavesForWhatIsBelow() {
        showTopBar()
        rule.runOnIdle { focus.settings.requestFocus() }
        press(Key.DirectionDown)
        rule.onNodeWithTag("left below").assertIsFocused()
    }

    @Test
    fun accessibilityServicesCanPressAButton() {
        showTopBar()
        rule.onNode(hasAnyDescendant(hasText("Search")) and SemanticsMatcher.keyIsDefined(SemanticsActions.OnClick)).performSemanticsAction(SemanticsActions.OnClick)
        rule.runOnIdle { assertEquals(1, searches) }
    }
}
