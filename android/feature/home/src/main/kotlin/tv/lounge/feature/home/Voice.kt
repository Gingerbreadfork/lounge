package tv.lounge.feature.home

import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.os.Bundle
import android.speech.RecognitionListener
import android.speech.RecognizerIntent
import android.speech.SpeechRecognizer
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.foundation.background
import androidx.compose.foundation.focusGroup
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.tv.material3.Icon
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import androidx.tv.material3.Text
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.delay
import tv.lounge.design.ButtonKind
import tv.lounge.design.LoungeButton
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme
import tv.lounge.design.R as DesignR
import tv.lounge.design.lightGlow

/**
 * The TV's own screen that listens and hands back the words, or null on a TV with none Lounge may
 * open. It needs nothing allowed to Lounge. [language] is the one to listen for, where it is not
 * the TV's own.
 */
internal fun voiceInputIntent(context: Context, language: String?): Intent? {
    val ask = Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH)
        .putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM)
        .putExtra(RecognizerIntent.EXTRA_MAX_RESULTS, 1)
    if (language != null) ask.putExtra(RecognizerIntent.EXTRA_LANGUAGE, language)
    val packages = context.packageManager
    val listeners = runCatching { packages.queryIntentActivities(ask, PackageManager.MATCH_DEFAULT_ONLY) }.getOrDefault(emptyList())
        .map { it.activityInfo }
        .filter { it.exported && (it.permission == null || context.checkSelfPermission(it.permission) == PackageManager.PERMISSION_GRANTED) }
    if (listeners.isEmpty()) return null
    // Named outright, so Android does not ask which of several to use. The TV's assistant comes
    // first, since its screen is made for the microphone in a remote.
    val assistant = runCatching { packages.resolveActivity(Intent(Intent.ACTION_ASSIST), PackageManager.MATCH_DEFAULT_ONLY)?.activityInfo?.packageName }.getOrNull()
    val chosen = listeners.firstOrNull { it.packageName == assistant }
        ?: listeners.firstOrNull { it.applicationInfo.flags and ApplicationInfo.FLAG_SYSTEM != 0 }
        ?: listeners.first()
    return ask.setClassName(chosen.packageName, chosen.name)
}

/** What the listening screen heard, or null when it heard nothing. */
internal fun heard(result: Intent?): String? =
    result?.getStringArrayListExtra(RecognizerIntent.EXTRA_RESULTS)?.firstOrNull()?.trim()?.takeIf { it.isNotEmpty() }

/**
 * Starts the TV listening and hands [onHeard] the words. Null on a TV that cannot listen.
 * [onFailed] is called when the listening screen would not open.
 */
@Composable
internal fun rememberVoiceSearch(language: String?, onHeard: (String) -> Unit, onFailed: () -> Unit): (() -> Unit)? {
    val context = LocalContext.current
    val ask = remember(language) { voiceInputIntent(context, language) }
    val taken = rememberUpdatedState(onHeard)
    val failed = rememberUpdatedState(onFailed)
    val listening = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
        heard(result.data)?.let { taken.value(it) }
    }
    return remember(ask) {
        ask?.let { intent -> { runCatching { listening.launch(intent) }.onFailure { failed.value() }; Unit } }
    }
}

/** What Lounge must be allowed before it can listen in its own panel. */
internal const val MICROPHONE = android.Manifest.permission.RECORD_AUDIO

/** Whether the TV has a speech service Lounge can listen through in its own panel. */
internal fun listensHere(context: Context): Boolean = runCatching { SpeechRecognizer.isRecognitionAvailable(context) }.getOrDefault(false)

/** What the listening panel is doing. */
private enum class Hearing { Asking, Starting, Listening, NotHeard, Failed }

private fun firstOf(results: Bundle?): String? =
    results?.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)?.firstOrNull()?.trim()?.takeIf { it.isNotEmpty() }

/**
 * The listening panel: a card in the middle of the screen that hears a query through the TV's
 * speech service and shows the words as they come. Lounge needs the microphone for it and asks
 * here first. [onTvScreen] hands over to the TV's own listening screen, where it has one.
 */
@Composable
fun VoicePanel(
    language: String?,
    visibleState: MutableTransitionState<Boolean>,
    onHeard: (String) -> Unit,
    onTvScreen: (() -> Unit)?,
    onDismiss: () -> Unit,
) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    val context = LocalContext.current
    val leaving = !visibleState.targetState
    val firstFocus = remember { FocusRequester() }
    var allowed by remember { mutableStateOf(context.checkSelfPermission(MICROPHONE) == PackageManager.PERMISSION_GRANTED) }
    var phase by remember { mutableStateOf(if (allowed) Hearing.Starting else Hearing.Asking) }
    var words by remember { mutableStateOf("") }
    // How loud the room is, from 0 to 1, read while drawing.
    val level = remember { mutableFloatStateOf(0f) }
    var attempt by remember { mutableIntStateOf(0) }
    val heard = rememberUpdatedState(onHeard)
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        allowed = granted
        when {
            granted -> phase = Hearing.Starting
            onTvScreen != null -> onTvScreen()
            // Nothing else can listen on this TV, so the question stays.
            else -> phase = Hearing.Asking
        }
    }

    // A panel shown again starts over, with the microphone as it is allowed now.
    LaunchedEffect(leaving) {
        if (!leaving) {
            allowed = context.checkSelfPermission(MICROPHONE) == PackageManager.PERMISSION_GRANTED
            words = ""
            phase = if (allowed) Hearing.Starting else Hearing.Asking
        }
    }
    // Something else in front ends the listening. Android's own dialog for the microphone is let be.
    LifecycleEventEffect(Lifecycle.Event.ON_PAUSE) { if (phase != Hearing.Asking) onDismiss() }

    // Lounge's language is asked for until the speech service says it has none for it.
    var inLanguage by remember(language) { mutableStateOf(language) }
    LaunchedEffect(allowed, attempt, leaving, inLanguage) {
        if (!allowed || leaving) return@LaunchedEffect
        // A service still letting go of the last try answers the next one as busy.
        if (attempt > 0) delay(250)
        val recognizer = runCatching { SpeechRecognizer.createSpeechRecognizer(context) }.getOrNull()
        if (recognizer == null) {
            phase = Hearing.Failed
            return@LaunchedEffect
        }
        recognizer.setRecognitionListener(object : RecognitionListener {
            override fun onReadyForSpeech(params: Bundle?) { phase = Hearing.Listening }
            override fun onBeginningOfSpeech() {}
            override fun onRmsChanged(rmsdB: Float) { level.floatValue = ((rmsdB + 2f) / 12f).coerceIn(0f, 1f) }
            override fun onBufferReceived(buffer: ByteArray?) {}
            override fun onEndOfSpeech() { level.floatValue = 0f }
            override fun onPartialResults(partialResults: Bundle?) { firstOf(partialResults)?.let { words = it } }
            override fun onEvent(eventType: Int, params: Bundle?) {}
            override fun onResults(results: Bundle?) {
                val said = firstOf(results)
                if (said != null) heard.value(said) else phase = Hearing.NotHeard
            }
            override fun onError(error: Int) {
                android.util.Log.w("Lounge", "voice: the speech service answered with error $error")
                level.floatValue = 0f
                val noSuchLanguage = error == SpeechRecognizer.ERROR_LANGUAGE_NOT_SUPPORTED || error == SpeechRecognizer.ERROR_LANGUAGE_UNAVAILABLE
                when {
                    noSuchLanguage && inLanguage != null -> inLanguage = null
                    error == SpeechRecognizer.ERROR_NO_MATCH || error == SpeechRecognizer.ERROR_SPEECH_TIMEOUT -> phase = Hearing.NotHeard
                    else -> phase = Hearing.Failed
                }
            }
        })
        val listen = Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH)
            .putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM)
            .putExtra(RecognizerIntent.EXTRA_PARTIAL_RESULTS, true)
            .putExtra(RecognizerIntent.EXTRA_CALLING_PACKAGE, context.packageName)
        inLanguage?.let { listen.putExtra(RecognizerIntent.EXTRA_LANGUAGE, it) }
        try {
            recognizer.startListening(listen)
            awaitCancellation()
        } catch (e: SecurityException) {
            phase = Hearing.Failed
        } finally {
            runCatching {
                recognizer.cancel()
                recognizer.destroy()
            }
        }
    }

    val pane = stringResource(DesignR.string.common_search_by_voice)
    val title = stringResource(
        when (phase) {
            Hearing.Asking -> R.string.voice_ask_title
            Hearing.Starting -> R.string.voice_starting
            Hearing.Listening -> R.string.voice_listening
            Hearing.NotHeard -> R.string.voice_not_heard
            Hearing.Failed -> R.string.voice_failed
        },
    )
    val body = when (phase) {
        Hearing.Asking -> stringResource(R.string.voice_ask_body)
        Hearing.Starting, Hearing.Listening -> words.ifEmpty { stringResource(R.string.voice_hint) }
        Hearing.NotHeard -> stringResource(R.string.voice_not_heard_hint)
        Hearing.Failed -> stringResource(R.string.voice_failed_hint)
    }
    val lit = phase == Hearing.Listening
    Box(Modifier.fillMaxSize()) {
        AnimatedVisibility(visibleState, enter = fadeIn(motion.timed(200)), exit = fadeOut(motion.timed(180))) {
            Box(Modifier.fillMaxSize().background(colors.scrim))
        }
        AnimatedVisibility(
            visibleState,
            enter = fadeIn(motion.snap()) + scaleIn(motion.snap(), initialScale = 0.96f),
            exit = fadeOut(motion.timed(120)),
            modifier = Modifier.align(Alignment.Center),
        ) {
            Column(
                Modifier
                    .width(720.dp)
                    .lightGlow(colors.houselight, 0.22f, radius = 60.dp, cornerRadius = 24.dp)
                    .clip(RoundedCornerShape(24.dp))
                    .background(colors.curtain)
                    .semantics { paneTitle = pane }
                    .onPreviewKeyEvent { e -> if (e.isBack() && e.type == KeyEventType.KeyUp) { onDismiss(); true } else false }
                    .focusProperties { canFocus = !leaving }
                    .trapFocus(leaving)
                    .focusGroup()
                    .padding(horizontal = 48.dp, vertical = 40.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                // A lamp behind the microphone that swells with the voice.
                Box(
                    Modifier.size(112.dp).drawBehind {
                        val loud = if (lit) level.floatValue else 0f
                        drawCircle(colors.houselight.copy(alpha = if (lit) 0.10f + 0.14f * loud else 0.06f), radius = size.minDimension / 2f * (0.78f + 0.22f * loud))
                        drawCircle(colors.houselight.copy(alpha = if (lit) 0.22f else 0.10f), radius = size.minDimension / 2f * 0.58f)
                    },
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(LoungeIcons.Mic, contentDescription = null, tint = if (lit) colors.houselight else colors.dust, modifier = Modifier.size(48.dp))
                }
                Spacer(Modifier.height(24.dp))
                Text(title, style = LoungeTheme.typography.headline, color = colors.limelight, textAlign = TextAlign.Center, modifier = Modifier.semantics { heading() })
                Spacer(Modifier.height(10.dp))
                Text(
                    body,
                    style = if (words.isNotEmpty() && (phase == Hearing.Listening || phase == Hearing.Starting)) LoungeTheme.typography.title else LoungeTheme.typography.body,
                    color = if (words.isNotEmpty() && (phase == Hearing.Listening || phase == Hearing.Starting)) colors.limelight else colors.dust,
                    textAlign = TextAlign.Center,
                    maxLines = 4,
                    overflow = TextOverflow.Ellipsis,
                )
                Spacer(Modifier.height(32.dp))
                // The two choices before listening are long in some languages, so they stack.
                if (phase == Hearing.Asking) {
                    Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(12.dp)) {
                        LoungeButton(stringResource(R.string.voice_ask_allow), onClick = { ask.launch(MICROPHONE) }, focusRequester = firstFocus)
                        if (onTvScreen != null) LoungeButton(stringResource(R.string.voice_ask_tv), onClick = onTvScreen, kind = ButtonKind.Secondary)
                    }
                }
                Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                    when (phase) {
                        Hearing.Asking -> Unit
                        Hearing.Starting, Hearing.Listening ->
                            LoungeButton(stringResource(DesignR.string.common_cancel), onClick = onDismiss, kind = ButtonKind.Secondary, focusRequester = firstFocus)
                        Hearing.NotHeard, Hearing.Failed -> {
                            LoungeButton(
                                stringResource(DesignR.string.common_try_again),
                                onClick = {
                                    words = ""
                                    phase = Hearing.Starting
                                    attempt++
                                },
                                focusRequester = firstFocus,
                            )
                            // Where the panel cannot listen, the TV's own screen may.
                            if (phase == Hearing.Failed && onTvScreen != null) {
                                LoungeButton(stringResource(R.string.voice_ask_tv), onClick = onTvScreen, kind = ButtonKind.Secondary)
                            } else {
                                LoungeButton(stringResource(DesignR.string.common_cancel), onClick = onDismiss, kind = ButtonKind.Secondary)
                            }
                        }
                    }
                }
            }
        }
    }
    // The buttons change with what the panel is doing, and focus follows to the first of them.
    LaunchedEffect(phase == Hearing.Asking, phase == Hearing.NotHeard || phase == Hearing.Failed) { firstFocus.requestWhenReady() }
}
