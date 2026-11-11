package tv.lounge.feature.home

import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.foundation.focusable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsFocused
import androidx.compose.ui.test.hasAnyDescendant
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isFocused
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.pressKey
import androidx.compose.ui.unit.dp
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import tv.lounge.core.Guide
import tv.lounge.core.GuideChannel
import tv.lounge.core.GuideProgramme
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme
import tv.lounge.platform.Prefs

/**
 * Overlays keep the d-pad inside them, close with Back and hand focus back to what opened them,
 * and a tile that leaves hands focus to its neighbour (DESIGN.md §6.4).
 */
@OptIn(ExperimentalTestApi::class)
@RunWith(RobolectricTestRunner::class)
class OverlayFocusTest {
    @get:Rule val rule = createComposeRule()

    private fun press(key: Key) {
        rule.onNode(isFocused()).performKeyInput { pressKey(key) }
        rule.waitForIdle()
    }

    @Test
    fun theTileMenuKeepsFocusAndGivesItBackToItsTile() {
        val tile = FocusRequester()
        var open by mutableStateOf(true)
        rule.setContent {
            LoungeTheme {
                Box(Modifier.fillMaxSize()) {
                    Box(Modifier.offset(96.dp, 300.dp).size(384.dp, 216.dp).testTag("tile").focusRequester(tile).focusable())
                    Box(Modifier.offset(96.dp, 80.dp).size(200.dp, 56.dp).testTag("above").focusable())
                    if (open) {
                        TileMenuPopover(
                            tile = testTile("app:tv.a", "Alpha"),
                            anchor = Rect(96f, 300f, 480f, 516f),
                            items = listOf(
                                MenuItem("Pin to the Stage", LoungeIcons.Pin) {},
                                MenuItem("Hide", LoungeIcons.Hide) {},
                            ),
                            visibleState = MutableTransitionState(true),
                            onDismiss = {
                                open = false
                                tile.requestFocus()
                            },
                        )
                    }
                }
            }
        }
        rule.waitForIdle()
        rule.onNode(isFocused()).assert(hasAnyDescendant(hasText("Pin to the Stage")))
        // Up from the first item and Left out of the menu both stay in it.
        press(Key.DirectionUp)
        press(Key.DirectionLeft)
        rule.onNode(isFocused()).assert(hasAnyDescendant(hasText("Pin to the Stage")))
        press(Key.DirectionDown)
        rule.onNode(isFocused()).assert(hasAnyDescendant(hasText("Hide")))
        press(Key.Back)
        rule.onNodeWithTag("tile").assertIsFocused()
    }

    @Test
    fun allAppsHandsFocusToTheNeighbourOfAnAppThatLeaves() {
        var apps by mutableStateOf(listOf(testTile("app:a", "Alpha"), testTile("app:b", "Beta"), testTile("app:c", "Gamma")))
        var closed = 0
        rule.setContent {
            LoungeTheme {
                AllAppsScreen(
                    apps = apps,
                    sort = Prefs.SORT_AZ,
                    onSetSort = {},
                    visibleState = MutableTransitionState(true),
                    covered = false,
                    heldTileId = { null },
                    onTileClick = {},
                    onTileLongPress = { _, _, _ -> },
                    onClose = { closed++ },
                )
            }
        }
        rule.waitForIdle()
        rule.onNode(isFocused()).assert(hasContentDescription("Alpha", substring = true))
        press(Key.DirectionRight)
        rule.onNode(isFocused()).assert(hasContentDescription("Beta", substring = true))
        apps = apps.filter { it.id != "app:b" }
        rule.waitForIdle()
        rule.onNode(isFocused()).assert(hasContentDescription("Gamma", substring = true))
        press(Key.Back)
        rule.runOnIdle { assertEquals(1, closed) }
    }

    @Test
    fun theGuideTakesFocusWhileItLoadsAndKeepsIt() {
        val ready = CountDownLatch(1)
        var dismissed = 0
        val start = System.currentTimeMillis()
        val guide = Guide(
            fromMs = start,
            toMs = start + 3 * 3_600_000,
            nowMs = start,
            channels = listOf(
                GuideChannel(
                    id = "chan:2",
                    name = "ABC",
                    number = "2",
                    logoPath = null,
                    lightArgb = 0xFFF2B263u,
                    action = "open:chan:2",
                    programmes = listOf(GuideProgramme(id = "p1", title = "News", subtitle = null, startMs = start, endMs = start + 1_800_000, isNow = true)),
                ),
            ),
        )
        rule.setContent {
            LoungeTheme {
                Box(Modifier.fillMaxSize()) {
                    Box(Modifier.offset(96.dp, 80.dp).size(200.dp, 56.dp).testTag("home").focusable())
                    GuideSheet(
                        loadGuide = { ready.await(10, TimeUnit.SECONDS); guide },
                        visibleState = MutableTransitionState(true),
                        onTune = {},
                        onDismiss = { dismissed++ },
                    )
                }
            }
        }
        rule.waitForIdle()
        // While the guide is read, its heading holds focus and the d-pad stays in the sheet.
        rule.onNode(isFocused()).assert(hasAnyDescendant(hasText("Reading the guide")))
        press(Key.DirectionUp)
        rule.onNode(isFocused()).assert(hasAnyDescendant(hasText("TV guide")))
        ready.countDown()
        rule.waitUntil(5_000) { runCatching { rule.onNode(isFocused()).assert(hasContentDescription("News", substring = true)) }.isSuccess }
        rule.onNode(isFocused()).assert(hasContentDescription("ABC, News, On now", substring = true))
        press(Key.Back)
        rule.runOnIdle { assertEquals(1, dismissed) }
    }
}
