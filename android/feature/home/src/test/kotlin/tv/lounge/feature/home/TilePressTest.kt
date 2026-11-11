package tv.lounge.feature.home

import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.isFocused
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.pressKey
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import tv.lounge.core.RowSize
import tv.lounge.core.Tile
import tv.lounge.core.TileKind
import tv.lounge.design.Lights
import tv.lounge.design.LoungeTheme

/** What a press and a hold do on a tile, and what a screen reader is offered. */
@OptIn(ExperimentalTestApi::class)
@RunWith(RobolectricTestRunner::class)
class TilePressTest {
    @get:Rule val rule = createComposeRule()

    private var presses = 0
    private var menus = 0

    private fun show(tile: Tile) {
        val focus = FocusRequester()
        rule.setContent {
            LoungeTheme {
                TileView(
                    tile = tile,
                    kind = TileKind.COMPACT,
                    size = tileSize(TileKind.COMPACT, RowSize.M),
                    lights = remember { mutableStateOf(Lights.Rested) },
                    rowName = "Your apps",
                    onFocused = { _, _ -> },
                    onClick = { presses++ },
                    onLongPress = { menus++ },
                    focusRequester = focus,
                )
            }
        }
        rule.runOnIdle { focus.requestFocus() }
        rule.waitForIdle()
    }

    /** The Menu key is the hold without the wait. */
    private fun hold() {
        rule.onNode(isFocused()).performKeyInput { pressKey(Key.Menu) }
        rule.waitForIdle()
    }

    @Test
    fun aHoldOpensTheMenuOfATileThatHasOne() {
        show(testTile("app:tv.a", "Alpha"))
        hold()
        rule.runOnIdle { assertEquals(0 to 1, presses to menus) }
    }

    @Test
    fun aHoldOnATileWithNoMenuIsAPress() {
        show(testTile("control:allapps", "All apps", action = "control:allapps"))
        hold()
        rule.runOnIdle { assertEquals(1 to 0, presses to menus) }
    }

    @Test
    fun aTileThatOpensNothingOffersNoPressToAScreenReader() {
        show(testTile("plugin:weather:now", "18 degrees now", action = ""))
        rule.onNode(isFocused())
            .assert(SemanticsMatcher.keyNotDefined(SemanticsActions.OnClick))
            .assert(SemanticsMatcher.keyIsDefined(SemanticsActions.OnLongClick))
    }

    @Test
    fun aTileThatOpensSomethingOffersThePress() {
        show(testTile("app:tv.a", "Alpha"))
        rule.onNode(isFocused()).assert(SemanticsMatcher.keyIsDefined(SemanticsActions.OnClick))
    }
}
