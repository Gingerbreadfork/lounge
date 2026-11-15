package tv.lounge.feature.home

import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.hasAnyDescendant
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isFocused
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.pressKey
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import tv.lounge.design.LoungeTheme
import tv.lounge.platform.AppNotification
import tv.lounge.platform.Prefs

/**
 * The notifications panel (DESIGN.md §5.4): what it shows, where focus lands on each of its
 * pages, and where focus goes when a notification leaves from under it.
 */
@OptIn(ExperimentalTestApi::class)
@RunWith(RobolectricTestRunner::class)
class NotificationsTest {
    @get:Rule val rule = createComposeRule()

    private fun notice(
        key: String,
        at: Long = 0,
        packageName: String = "tv.shop",
        app: String = "Shop",
        title: String = "Title $key",
        text: String = "",
        dismissible: Boolean = true,
        opens: Boolean = true,
        actions: List<AppNotification.Action> = emptyList(),
    ) = AppNotification(key, packageName, app, title, text, at, dismissible, opens, actions)

    private fun press(key: Key) {
        rule.onNode(isFocused()).performKeyInput { pressKey(key) }
        rule.waitForIdle()
    }

    private fun assertFocusedRow(title: String) = rule.onNode(isFocused()).assert(hasAnyDescendant(hasText(title)))

    private fun sheet(content: @Composable (FocusRequester) -> Unit) {
        rule.setContent {
            LoungeTheme {
                RightSheet(visibleState = MutableTransitionState(true), key = "sheet", depth = 0, onDismiss = {}) { first, _ -> content(first) }
            }
        }
        rule.waitForIdle()
    }

    @Test
    fun theSwitchAndTheHiddenAppsDecideWhatIsShownAndWhatIsNew() {
        val posted = listOf(notice("new", at = 30), notice("old", at = 10), notice("other", at = 20, packageName = "tv.noisy"))
        val all = shownNotifications(posted, Prefs.NotificationPrefs(seenAt = 15))
        assertEquals(listOf("new", "old", "other"), all.list.map { it.key })
        assertEquals(2, all.fresh)
        val quiet = shownNotifications(posted, Prefs.NotificationPrefs(hiddenApps = setOf("tv.noisy"), seenAt = 30))
        assertEquals(listOf("new", "old"), quiet.list.map { it.key })
        assertEquals(0, quiet.fresh)
        assertEquals(NotificationsShown(), shownNotifications(posted, Prefs.NotificationPrefs(shown = false)))
    }

    @Test
    fun theWordsAreSentencesAndAButtonIsNeverShouted() {
        assertEquals("1 notification", NotificationWords.count(1))
        assertEquals("3 notifications, 2 new", NotificationWords.spoken(3, 2))
        assertEquals("3 notifications", NotificationWords.spoken(3, 0))
        assertEquals("Free up space. Then try again.", NotificationWords.preview(" Free up space.\n\nThen   try again. "))
        val now = 10 * 60_000L
        assertEquals("Shop, 5 minutes ago", NotificationWords.byline(notice("a", at = 5 * 60_000L), now))
        // A notification with no title of its own is titled with its app, which is then not said twice.
        assertEquals("Just now", NotificationWords.byline(notice("a", at = now, title = "Shop"), now))
        assertEquals("Show in folder", NotificationWords.button("SHOW IN FOLDER"))
        assertEquals("Open in VLC", NotificationWords.button("Open in VLC"))
        assertEquals("OK", NotificationWords.button("OK"))
        assertEquals("Turn off TV", NotificationWords.button("TURN OFF TV"))
        assertEquals("Yes", NotificationWords.button("YES"))
        assertNull(NotificationWords.staying(0))
        assertEquals("2 stay until their apps take them back", NotificationWords.staying(2))
    }

    @Test
    fun theListOpensOnItsFirstRowAndOkReadsTheFocusedOne() {
        val read = ArrayList<Pair<Int, String>>()
        val shown = NotificationsShown(listOf(notice("a"), notice("b"), notice("c", dismissible = false)))
        sheet { first -> NotificationsSheet(shown, { null }, returnTo = null, lastIndex = 0, onRead = { at, n -> read += at to n.key }, onDismissAll = {}, firstFocus = first) }
        assertFocusedRow("Title a")
        press(Key.DirectionDown)
        assertFocusedRow("Title b")
        press(Key.DirectionCenter)
        rule.runOnIdle { assertEquals(listOf(1 to "b"), read) }
        // Dismiss all ends the list and says what it will leave.
        press(Key.DirectionDown)
        press(Key.DirectionDown)
        assertFocusedRow("Dismiss all")
        rule.onNodeWithText("1 stays until its app takes it back").assertExists()
        // Down from the last row stays inside the panel.
        press(Key.DirectionDown)
        assertFocusedRow("Dismiss all")
    }

    @Test
    fun oneNotificationOffersNoDismissAll() {
        sheet { first -> NotificationsSheet(NotificationsShown(listOf(notice("a"))), { null }, returnTo = null, lastIndex = 0, onRead = { _, _ -> }, onDismissAll = {}, firstFocus = first) }
        assertFocusedRow("Title a")
        rule.onNodeWithText("Dismiss all").assertDoesNotExist()
    }

    @Test
    fun backFromANotificationLandsOnItsRowOrOnWhatTookItsPlace() {
        val shown = NotificationsShown(listOf(notice("a"), notice("b"), notice("c")))
        sheet { first -> NotificationsSheet(shown, { null }, returnTo = "b", lastIndex = 2, onRead = { _, _ -> }, onDismissAll = {}, firstFocus = first) }
        assertFocusedRow("Title b")
    }

    @Test
    fun aNotificationDismissedFromItsPageLeavesFocusOnWhatTookItsPlace() {
        // The one at the second place was dismissed from its page; the list comes back without it.
        val shown = NotificationsShown(listOf(notice("a"), notice("c"), notice("d")))
        sheet { first -> NotificationsSheet(shown, { null }, returnTo = "b", lastIndex = 1, onRead = { _, _ -> }, onDismissAll = {}, firstFocus = first) }
        assertFocusedRow("Title c")
    }

    @Test
    fun aNotificationItsAppTakesBackHandsFocusToItsNeighbour() {
        var shown by mutableStateOf(NotificationsShown(listOf(notice("a"), notice("b"), notice("c"))))
        sheet { first -> NotificationsSheet(shown, { null }, returnTo = null, lastIndex = 0, onRead = { _, _ -> }, onDismissAll = {}, firstFocus = first) }
        press(Key.DirectionDown)
        assertFocusedRow("Title b")
        shown = NotificationsShown(listOf(notice("a"), notice("c")))
        rule.waitForIdle()
        assertFocusedRow("Title c")
        // The last one going hands focus up.
        shown = NotificationsShown(listOf(notice("a")))
        rule.waitForIdle()
        assertFocusedRow("Title a")
    }

    @Test
    fun dismissAllLeavesFocusOnWhatStays() {
        var shown by mutableStateOf(NotificationsShown(listOf(notice("a"), notice("b"), notice("stays", dismissible = false))))
        sheet { first ->
            NotificationsSheet(shown, { null }, returnTo = null, lastIndex = 0, onRead = { _, _ -> }, onDismissAll = { shown = NotificationsShown(shown.list.filter { !it.dismissible }) }, firstFocus = first)
        }
        repeat(3) { press(Key.DirectionDown) }
        assertFocusedRow("Dismiss all")
        press(Key.DirectionCenter)
        assertFocusedRow("Title stays")
        rule.onNodeWithText("Dismiss all").assertDoesNotExist()
    }

    private fun page(n: AppNotification, pressed: MutableList<String> = ArrayList()) = sheet { first ->
        NotificationDetailSheet(
            notification = n,
            onOpen = { pressed += "open" },
            onPress = { _, action -> pressed += "press ${action.index}" },
            onDismiss = { pressed += "dismiss" },
            onHideApp = { pressed += "hide" },
            firstFocus = first,
        )
    }

    @Test
    fun aPageShowsTheWholeTextAndLandsOnOpen() {
        val pressed = ArrayList<String>()
        val text = "Some system functions may not work.\nFree up space by removing apps or files you no longer need."
        page(notice("a", title = "Storage space running out", app = "Android System", text = text, actions = listOf(AppNotification.Action(0, "FREE UP SPACE"))), pressed)
        rule.onNodeWithText(text).assertExists()
        assertFocusedRow("Open")
        press(Key.DirectionCenter)
        press(Key.DirectionDown)
        assertFocusedRow("Free up space")
        press(Key.DirectionCenter)
        rule.onNodeWithText("Sent").assertExists()
        press(Key.DirectionDown)
        assertFocusedRow("Dismiss")
        press(Key.DirectionCenter)
        press(Key.DirectionDown)
        assertFocusedRow("Hide everything from Android System")
        press(Key.DirectionCenter)
        rule.runOnIdle { assertEquals(listOf("open", "press 0", "dismiss", "hide"), pressed) }
    }

    @Test
    fun aPageWithNothingToOpenLandsOnTheFirstThingThereIsToDo() {
        page(notice("a", opens = false, actions = listOf(AppNotification.Action(1, "Eject"))))
        assertFocusedRow("Eject")
    }

    @Test
    fun aPageToReadLandsOnDismiss() {
        page(notice("a", opens = false))
        assertFocusedRow("Dismiss")
        rule.onNodeWithText("Open").assertDoesNotExist()
    }

    @Test
    fun aPageThatStaysSaysSoAndLandsOnTheOneRowItHas() {
        page(notice("a", opens = false, dismissible = false))
        assertFocusedRow("Hide everything from Shop")
        rule.onNodeWithText("Dismiss").assertDoesNotExist()
        rule.onNodeWithText(NotificationWords.STAYS).assertExists()
    }

    @Test
    fun settingsTurnsThemOffAndShowsAHiddenAppAgain() {
        var on by mutableStateOf(true)
        var hidden by mutableStateOf(listOf(HiddenApp("tv.a", "Alpha"), HiddenApp("tv.b", "Beta")))
        sheet { first ->
            NotificationSettingsSheet(on = on, allowed = true, hidden = hidden, iconPath = { null }, onSet = { on = it }, onAllow = {}, onShow = { app -> hidden = hidden - app }, firstFocus = first)
        }
        assertFocusedRow("Notifications")
        rule.onNodeWithText("On").assertExists()
        press(Key.DirectionCenter)
        rule.onNodeWithText("Off").assertExists()
        // A row shown again leaves, and focus moves to its neighbour, then back up to the switch.
        press(Key.DirectionDown)
        assertFocusedRow("Alpha")
        press(Key.DirectionCenter)
        assertFocusedRow("Beta")
        press(Key.DirectionCenter)
        assertFocusedRow("Notifications")
        rule.onNodeWithText("Hidden apps").assertDoesNotExist()
    }

    @Test
    fun withoutAccessTheSwitchIsTheStepToTake() {
        var asked = 0
        sheet { first ->
            NotificationSettingsSheet(on = true, allowed = false, hidden = emptyList(), iconPath = { null }, onSet = {}, onAllow = { asked++ }, onShow = {}, firstFocus = first)
        }
        assertFocusedRow("Not allowed")
        press(Key.DirectionCenter)
        rule.runOnIdle { assertEquals(1, asked) }
    }
}
