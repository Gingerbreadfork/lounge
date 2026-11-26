package tv.lounge.feature.home

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.ActivityInfo
import android.content.pm.ApplicationInfo
import android.os.Bundle
import android.speech.RecognizerIntent
import android.speech.SpeechRecognizer
import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.hasAnyDescendant
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isFocused
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.pressKey
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.shadows.ShadowSpeechRecognizer
import tv.lounge.design.LoungeTheme

/** Voice search: the listening panel, which of the TV's screens listens in its place, and where a spoken query lands. */
@OptIn(ExperimentalTestApi::class)
@RunWith(RobolectricTestRunner::class)
class VoiceTest {
    @get:Rule val rule = createComposeRule()

    private val context: Context = RuntimeEnvironment.getApplication()

    private fun tvHas(action: String, owner: String, exported: Boolean = true, system: Boolean = true) {
        val screen = ActivityInfo().apply {
            packageName = owner
            name = "$owner.${action.substringAfterLast('.')}"
            this.exported = exported
            applicationInfo = ApplicationInfo().apply { packageName = owner; flags = if (system) ApplicationInfo.FLAG_SYSTEM else 0 }
        }
        shadowOf(context.packageManager).addOrUpdateActivity(screen)
        shadowOf(context.packageManager).addIntentFilterForActivity(
            ComponentName(screen.packageName, screen.name),
            IntentFilter(action).apply { addCategory(Intent.CATEGORY_DEFAULT) },
        )
    }

    @Test
    fun aTvThatCannotListenOffersNothing() {
        assertNull(voiceInputIntent(context, null))
        tvHas(RecognizerIntent.ACTION_RECOGNIZE_SPEECH, "tv.closed", exported = false)
        assertNull(voiceInputIntent(context, null))
    }

    @Test
    fun theTvsAssistantListensWhenSeveralCould() {
        tvHas(RecognizerIntent.ACTION_RECOGNIZE_SPEECH, "tv.speech")
        tvHas(RecognizerIntent.ACTION_RECOGNIZE_SPEECH, "tv.assistant")
        tvHas(Intent.ACTION_ASSIST, "tv.assistant")
        val ask = voiceInputIntent(context, "nl")!!
        assertEquals("tv.assistant", ask.component?.packageName)
        assertEquals("nl", ask.getStringExtra(RecognizerIntent.EXTRA_LANGUAGE))
        assertNull(voiceInputIntent(context, null)!!.getStringExtra(RecognizerIntent.EXTRA_LANGUAGE))
    }

    @Test
    fun withNoAssistantAScreenTheTvCameWithListens() {
        tvHas(RecognizerIntent.ACTION_RECOGNIZE_SPEECH, "app.added", system = false)
        tvHas(RecognizerIntent.ACTION_RECOGNIZE_SPEECH, "tv.speech")
        assertEquals("tv.speech", voiceInputIntent(context, null)?.component?.packageName)
    }

    @Test
    fun whatWasHeardIsTheFirstResultOrNothing() {
        fun result(vararg words: String) = Intent().putStringArrayListExtra(RecognizerIntent.EXTRA_RESULTS, ArrayList(words.toList()))
        assertEquals("the bear", heard(result(" the bear ", "the beer")))
        assertNull(heard(result("  ")))
        assertNull(heard(result()))
        assertNull(heard(null))
    }

    private fun showSearch(spoken: Boolean, covered: Boolean = false) {
        rule.setContent {
            LoungeTheme {
                SearchScreen(
                    snapshot = testSnapshot(allApps = listOf(testTile("app:tv.netflix", title = "Netflix"), testTile("app:tv.plex", title = "Plex"))),
                    visibleState = MutableTransitionState(true),
                    onTileFocused = { _, _ -> },
                    onTileClick = {},
                    onTileLongPress = { _, _, _, _ -> },
                    onClose = {},
                    pendingQuery = "netflix",
                    pendingSpoken = spoken,
                    covered = covered,
                    onVoice = {},
                )
            }
        }
        rule.waitForIdle()
    }

    @Test
    fun aSpokenQueryLandsOnWhatItFound() {
        showSearch(spoken = true)
        rule.onNode(isFocused()).assert(hasAnyDescendant(hasText("Netflix")) or hasText("Netflix") or hasContentDescription("Netflix", substring = true))
    }

    @Test
    fun resultsDoNotTakeFocusFromAPanelOverSearch() {
        showSearch(spoken = true, covered = true)
        rule.onNode(isFocused()).assert(hasText("Speak"))
    }

    @Test
    fun aTypedQueryStaysInTheField() {
        showSearch(spoken = false)
        rule.onNode(isFocused()).assert(hasContentDescription("Search your TV"))
    }

    private fun showPanel(onHeard: (String) -> Unit = {}, onTvScreen: (() -> Unit)? = null) {
        rule.setContent {
            LoungeTheme { VoicePanel(language = null, visibleState = MutableTransitionState(true), onHeard = onHeard, onTvScreen = onTvScreen, onDismiss = {}) }
        }
        rule.waitForIdle()
    }

    private fun press(key: Key) {
        rule.onNode(isFocused()).performKeyInput { pressKey(key) }
        rule.waitForIdle()
    }

    @Test
    fun withoutTheMicrophoneThePanelAsksFirstAndOffersTheTvsOwnScreen() {
        var handedOver = 0
        showPanel(onTvScreen = { handedOver++ })
        rule.onNode(hasText("Let Lounge listen?")).assertExists()
        rule.onNode(isFocused()).assert(hasText("Allow the microphone"))
        press(Key.DirectionDown)
        rule.onNode(isFocused()).assert(hasText("Use the TV's own screen"))
        press(Key.DirectionCenter)
        assertEquals(1, handedOver)
    }

    @Test
    fun whatIsSaidIsHandedOnAndSilenceOffersAnotherTry() {
        shadowOf(RuntimeEnvironment.getApplication()).grantPermissions(MICROPHONE)
        var said: String? = null
        showPanel(onHeard = { said = it })
        val speech = shadowOf(ShadowSpeechRecognizer.getLatestSpeechRecognizer())
        fun words(vararg heard: String) = Bundle().apply { putStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION, ArrayList(heard.toList())) }
        rule.runOnIdle { speech.triggerOnReadyForSpeech(Bundle()) }
        rule.onNode(hasText("Listening…")).assertExists()
        // The words show as they come.
        rule.runOnIdle { speech.triggerOnPartialResults(words("the be")) }
        rule.onNode(hasText("the be")).assertExists()
        rule.runOnIdle { speech.triggerOnError(SpeechRecognizer.ERROR_NO_MATCH) }
        rule.onNode(hasText("Didn't catch that")).assertExists()
        rule.onNode(isFocused()).assert(hasText("Try again"))
        press(Key.DirectionCenter)
        val again = shadowOf(ShadowSpeechRecognizer.getLatestSpeechRecognizer())
        rule.runOnIdle { again.triggerOnResults(words(" the bear ")) }
        assertEquals("the bear", said)
    }
}
