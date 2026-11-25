package tv.lounge.feature.home

import androidx.compose.foundation.layout.Column
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
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.pressKey
import androidx.test.core.app.ApplicationProvider
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import tv.lounge.core.GroupInfo
import tv.lounge.core.Tile
import tv.lounge.design.LoungeTheme

/** Groups of apps: what a tile's menu offers for them, and the sheet that lists them. */
@OptIn(ExperimentalTestApi::class)
@RunWith(RobolectricTestRunner::class)
class GroupsTest {
    @get:Rule val rule = createComposeRule()

    private val resources = ApplicationProvider.getApplicationContext<android.content.Context>().resources
    private val netflix = testTile("app:tv.netflix", title = "Netflix")

    private fun menu(tile: Tile, rowId: String, groupName: String? = null, left: Boolean = false, right: Boolean = false): List<String> = menuItemsFor(
        tile = tile,
        rowId = rowId,
        rowTitle = groupName ?: "Your apps",
        isPinned = false,
        canMoveLeft = left,
        canMoveRight = right,
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
        groupName = groupName,
    ).map { if (it.named != null) resources.getString(it.label, it.named) else resources.getString(it.label) }

    @Test
    fun anAppCanBeAddedToAGroupFromAnyRow() {
        assertTrue("Add to a group" in menu(netflix, "apps"))
        assertTrue("Add to a group" in menu(netflix, "allapps"))
        assertFalse("Edit this group" in menu(netflix, "apps"))
        assertFalse("Add to a group" in menu(testTile("plugin:h:film"), "plugin:h:new"))
        assertFalse("Add to a group" in menu(netflix.copy(hidden = true), "allapps"))
    }

    @Test
    fun anAppInAGroupsRowCanBeMovedTakenOutAndTheGroupEdited() {
        val items = menu(netflix, "group:g1", groupName = "Kids", left = true, right = true)
        assertEquals(listOf("Move left", "Move right", "Remove from Kids", "Add to a group"), items.take(4))
        assertTrue(items.indexOf("Edit this group") == items.indexOf("Hide this row") + 1)
        assertEquals("Remove from Kids", menu(netflix, "group:g1", groupName = "Kids").first())
        assertEquals("g1", groupOfRow("group:g1"))
        assertEquals(null, groupOfRow("apps"))
    }

    private fun press(key: Key) {
        rule.onNode(isFocused()).performKeyInput { pressKey(key) }
        rule.waitForIdle()
    }

    @Test
    fun okPutsTheAppInAGroupAndTakesItOut() {
        var groups by mutableStateOf(listOf(GroupInfo("g1", "Kids", emptyList()), GroupInfo("g2", "Sport", listOf("tv.netflix", "tv.other"))))
        val first = FocusRequester()
        rule.setContent {
            LoungeTheme {
                Column {
                    GroupsSheet(
                        app = netflix,
                        groups = groups,
                        onToggle = { group, add -> groups = groups.map { if (it.id == group.id) it.copy(apps = if (add) it.apps + "tv.netflix" else it.apps - "tv.netflix") else it } },
                        onOpen = {},
                        onCreate = {},
                        firstFocus = first,
                    )
                }
            }
        }
        rule.waitForIdle()
        rule.onNode(isFocused()).assert(hasAnyDescendant(hasText("Kids")) and hasAnyDescendant(hasText("Not added")) and hasAnyDescendant(hasText("No apps yet")))
        press(Key.DirectionCenter)
        rule.onNode(isFocused()).assert(hasAnyDescendant(hasText("Added")) and hasAnyDescendant(hasText("1 app")))
        press(Key.DirectionDown)
        rule.onNode(isFocused()).assert(hasAnyDescendant(hasText("Sport")) and hasAnyDescendant(hasText("2 apps")))
        press(Key.DirectionCenter)
        assertEquals(listOf(listOf("tv.netflix"), listOf("tv.other")), groups.map { it.apps })
    }

    @Test
    fun focusOpensOnTheGroupJustLeftOrTheFirstWhenThatOneHasGone() {
        var groups by mutableStateOf(listOf(GroupInfo("g1", "Kids", emptyList()), GroupInfo("g2", "Sport", emptyList()), GroupInfo("g3", "Music", emptyList())))
        var returnTo by mutableStateOf<String?>("g2")
        val first = FocusRequester()
        rule.setContent {
            LoungeTheme {
                Column { GroupsSheet(app = null, groups = groups, onToggle = { _, _ -> }, onOpen = {}, onCreate = {}, firstFocus = first, returnTo = returnTo) }
            }
        }
        rule.waitForIdle()
        rule.onNode(isFocused()).assert(hasAnyDescendant(hasText("Sport")))
        // The focused group is deleted, as from the phone, and focus goes to the first one left.
        returnTo = "g9"
        groups = groups.filter { it.id != "g2" }
        rule.waitForIdle()
        rule.onNode(isFocused()).assert(hasAnyDescendant(hasText("Kids")))
        // A group counts only the apps its row would show.
        groups = listOf(GroupInfo("g1", "Kids", listOf("tv.a", "tv.gone")))
        rule.waitForIdle()
        rule.onNode(hasText("2 apps")).assertExists()
    }

    @Test
    fun aGroupCountsTheAppsItsRowWouldShow() {
        val first = FocusRequester()
        rule.setContent {
            LoungeTheme {
                Column {
                    GroupsSheet(app = null, groups = listOf(GroupInfo("g1", "Kids", listOf("tv.a", "tv.gone"))), onToggle = { _, _ -> }, onOpen = {}, onCreate = {}, firstFocus = first, onTv = setOf("tv.a"))
                }
            }
        }
        rule.waitForIdle()
        rule.onNode(hasText("1 app")).assertExists()
    }

    @Test
    fun withoutAnAppOkOpensTheGroup() {
        var opened: String? = null
        val first = FocusRequester()
        rule.setContent {
            LoungeTheme {
                Column {
                    GroupsSheet(app = null, groups = listOf(GroupInfo("g1", "Kids", listOf("tv.a"))), onToggle = { _, _ -> }, onOpen = { opened = it.id }, onCreate = {}, firstFocus = first)
                }
            }
        }
        rule.waitForIdle()
        press(Key.DirectionCenter)
        assertEquals("g1", opened)
    }
}
