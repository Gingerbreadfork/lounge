package tv.lounge.feature.home

import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isFocused
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.performSemanticsAction
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import tv.lounge.design.LoungeTheme

/** The Clear button beside the search field. */
@RunWith(RobolectricTestRunner::class)
class SearchClearTest {
    @get:Rule val rule = createComposeRule()

    private fun show(query: String?) {
        rule.setContent {
            LoungeTheme {
                SearchScreen(
                    snapshot = testSnapshot(allApps = listOf(testTile("app:tv.netflix", title = "Netflix"))),
                    visibleState = MutableTransitionState(true),
                    onTileFocused = { _, _ -> },
                    onTileClick = {},
                    onTileLongPress = { _, _, _, _ -> },
                    onClose = {},
                    pendingQuery = query,
                )
            }
        }
        rule.waitForIdle()
    }

    @Test
    fun anEmptyFieldHasNothingToClear() {
        show(null)
        rule.onNode(hasText("Clear")).assertDoesNotExist()
    }

    @Test
    fun clearEmptiesTheFieldAndGoesBackToIt() {
        show("netflix")
        rule.onNode(hasText("Netflix")).assertExists()
        // Clear holds focus when it is pressed, and is gone the moment the field is empty.
        rule.onNode(hasText("Clear")).performSemanticsAction(SemanticsActions.RequestFocus)
        rule.waitForIdle()
        rule.onNode(isFocused()).assert(hasText("Clear"))
        rule.onNode(hasText("Clear")).performSemanticsAction(SemanticsActions.OnClick)
        rule.waitForIdle()
        rule.onNode(hasText("Clear")).assertDoesNotExist()
        rule.onNode(hasText("Search your TV")).assertExists()
        rule.onNode(isFocused()).assert(hasContentDescription("Search your TV"))
    }
}
