package tv.lounge.feature.home

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.focusGroup
import androidx.compose.foundation.focusable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.tv.material3.Icon
import androidx.tv.material3.Text
import coil3.compose.AsyncImage
import kotlinx.coroutines.delay
import tv.lounge.core.NowPlaying
import tv.lounge.core.PlaybackStatus
import tv.lounge.design.ButtonKind
import tv.lounge.design.Dimens
import tv.lounge.design.LoungeButton
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme
import tv.lounge.design.ProgressBar
import tv.lounge.design.argb
import tv.lounge.design.focusRing
import tv.lounge.design.lightGlow

/**
 * The Now Playing card (DESIGN.md §5.4): bottom-right, 640 × 200, artwork left, title and
 * artist, transport buttons, elapsed progress. Back closes it; focus returns to what opened it.
 */
@Composable
fun NowPlayingCard(
    np: NowPlaying,
    visibleState: MutableTransitionState<Boolean>,
    onCommand: (String) -> Unit,
    onOpenApp: () -> Unit,
    onDismiss: () -> Unit,
) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    val light = LoungeTheme.colors.light(np.lightArgb)
    val firstFocus = remember { FocusRequester() }
    val leaving = !visibleState.targetState
    val playing = np.status == PlaybackStatus.PLAYING

    // Position advances locally while playing; the core only hears about state changes. Only the
    // bar's drawing reads the clock, so the card does not recompose as it ticks.
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    LaunchedEffect(playing, np.updatedMs) {
        while (playing) {
            now = System.currentTimeMillis()
            delay(1000)
        }
    }
    val position = np.positionMs
    val duration = np.durationMs?.takeIf { it > 0 }
    val progress: (() -> Float)? = if (position != null && duration != null) {
        { (if (playing) position + (now - np.updatedMs).coerceAtLeast(0) else position).toFloat() / duration }
    } else {
        null
    }

    Box(Modifier.fillMaxSize()) {
        AnimatedVisibility(visibleState, enter = fadeIn(tween(200)), exit = fadeOut(tween(180))) {
            Box(Modifier.fillMaxSize().background(colors.scrim))
        }
        AnimatedVisibility(
            visibleState,
            enter = slideInVertically(motion.proscenium()) { it / 2 } + fadeIn(motion.proscenium()),
            exit = slideOutVertically(motion.settle()) { it / 2 } + fadeOut(motion.settle()),
            modifier = Modifier.align(Alignment.BottomEnd).padding(end = Dimens.safeX, bottom = Dimens.safeY),
        ) {
            Row(
                Modifier
                    .size(640.dp, 200.dp)
                    .lightGlow(light, 0.22f, radius = 60.dp, cornerRadius = 20.dp)
                    .clip(RoundedCornerShape(20.dp))
                    .background(colors.curtain)
                    .onPreviewKeyEvent { e -> if (e.isBack() && e.type == KeyEventType.KeyUp) { onDismiss(); true } else false }
                    .focusProperties { canFocus = !leaving }
                    .trapFocus(leaving)
                    .focusGroup(),
            ) {
                Box(Modifier.size(200.dp).background(light.copy(alpha = 0.25f))) {
                    if (np.artPath != null) {
                        AsyncImage(model = np.artPath, contentDescription = null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
                    } else {
                        Icon(LoungeIcons.Equaliser, contentDescription = null, tint = light, modifier = Modifier.align(Alignment.Center).size(64.dp))
                    }
                }
                Column(Modifier.weight(1f).fillMaxSize().padding(start = 24.dp, end = 24.dp, top = 20.dp, bottom = 18.dp)) {
                    Text(np.appLabel, style = LoungeTheme.typography.caption, color = colors.dust, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Spacer(Modifier.height(4.dp))
                    Text(np.title, style = LoungeTheme.typography.title, color = colors.limelight, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    np.subtitle?.let {
                        Spacer(Modifier.height(2.dp))
                        Text(it, style = LoungeTheme.typography.meta, color = colors.dust, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                    Spacer(Modifier.weight(1f))
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        TransportButton(LoungeIcons.SkipPrevious, "Previous", np.canSkipPrevious, light) { onCommand("previous") }
                        TransportButton(
                            if (playing) LoungeIcons.Pause else LoungeIcons.Play,
                            if (playing) "Pause" else "Play",
                            if (playing) np.canPause else np.canPlay,
                            light,
                            primary = true,
                            focusRequester = firstFocus,
                        ) { onCommand("play-pause") }
                        TransportButton(LoungeIcons.SkipNext, "Next", np.canSkipNext, light) { onCommand("next") }
                        Spacer(Modifier.weight(1f))
                        LoungeButton("Open", onClick = onOpenApp, kind = ButtonKind.Secondary)
                    }
                    if (progress != null) {
                        Spacer(Modifier.height(14.dp))
                        ProgressBar(progress, Modifier.fillMaxWidth(), thickness = 4.dp)
                    }
                }
            }
        }
    }
    LaunchedEffect(Unit) { firstFocus.requestWhenReady() }
}

/** A 56 px round transport button; the primary one is filled with the artwork's light. */
@Composable
private fun TransportButton(
    icon: ImageVector,
    label: String,
    enabled: Boolean,
    light: androidx.compose.ui.graphics.Color,
    primary: Boolean = false,
    focusRequester: FocusRequester? = null,
    onClick: () -> Unit,
) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    val visuals = LoungeTheme.visuals
    var focused by remember { mutableStateOf(false) }
    val scale = animateFloatAsState(if (focused) 1.08f else 1f, if (focused) motion.snap() else motion.settle(), label = "transportScale")
    val glow = animateFloatAsState(if (focused) visuals.glowAlpha else 0f, if (focused) motion.snap() else motion.settle(), label = "transportGlow")
    val fill = if (primary) light else if (focused) colors.curtainEdge else colors.velvet
    val tint = if (primary) tv.lounge.design.textOn(light, colors) else if (focused) colors.limelight else colors.dust
    Box(
        Modifier
            .then(if (focusRequester != null) Modifier.focusRequester(focusRequester) else Modifier)
            .graphicsLayer { scaleX = scale.value; scaleY = scale.value }
            .lightGlow(light, { glow.value }, radius = 32.dp, cornerRadius = 28.dp)
            .clip(CircleShape)
            .background(fill)
            .focusRing(colors, { focused }, 28.dp)
            .alpha(if (enabled) 1f else 0.4f)
            .onFocusChanged { focused = it.isFocused }
            .then(if (enabled) Modifier.tilePress(onClick, onClick) else Modifier)
            .focusable(enabled)
            .semantics { role = Role.Button; contentDescription = label }
            .size(56.dp),
        contentAlignment = Alignment.Center,
    ) {
        Icon(icon, contentDescription = null, tint = tint, modifier = Modifier.size(28.dp))
    }
}
