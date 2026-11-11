package tv.lounge.feature.home

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.focusGroup
import androidx.compose.foundation.focusable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.Image
import androidx.compose.ui.res.painterResource
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.focusRestorer
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlin.math.roundToInt
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp
import androidx.tv.material3.Icon
import androidx.tv.material3.Text
import kotlinx.coroutines.delay
import tv.lounge.core.NowPlaying
import tv.lounge.core.PlaybackStatus
import tv.lounge.design.Dimens
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.platform.LocalDensity
import kotlinx.coroutines.launch
import tv.lounge.design.Swap
import tv.lounge.design.Words
import tv.lounge.design.argb
import tv.lounge.design.focusRim
import tv.lounge.design.is24HourClock
import tv.lounge.design.lightGlow
import tv.lounge.design.lightState
import tv.lounge.design.sounds
import kotlin.math.sin
import java.time.LocalTime

/**
 * Top bar (DESIGN.md §5.1): the mark on the left; Search, TV controls, Settings and the clock on
 * the right. Left off Search and Right off Settings nudge the button instead of leaving the bar.
 */
@Composable
fun TopBar(
    showTvControls: Boolean,
    showGuide: Boolean,
    nowPlaying: NowPlaying?,
    focus: TopBarFocus,
    downFocus: () -> FocusRequester,
    onFocused: () -> Unit,
    onSearch: () -> Unit,
    onTvControls: () -> Unit,
    onGuide: () -> Unit,
    onNowPlaying: () -> Unit,
    onSettings: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val motion = LoungeTheme.motion
    val scope = rememberCoroutineScope()
    val nudgePx = with(LocalDensity.current) { 6.dp.toPx() }
    Row(
        modifier
            .fillMaxWidth()
            .padding(start = Dimens.safeX, end = Dimens.safeX, top = Dimens.safeY)
            .height(Dimens.topBarHeight)
            .onFocusChanged { if (it.hasFocus) onFocused() }
            .onPreviewKeyEvent { e ->
                if (e.type != KeyEventType.KeyDown) return@onPreviewKeyEvent false
                val edge = topBarEdge(e.key, focus.last, focus.search, focus.settings)
                if (edge != 0 && e.nativeKeyEvent.repeatCount == 0 && !motion.reducedMotion) {
                    scope.launch {
                        focus.nudge.animateTo(edge * nudgePx, motion.snap())
                        focus.nudge.animateTo(0f, motion.settle())
                    }
                }
                edge != 0
            }
            .focusRestorer()
            .focusGroup()
            .focusProperties { down = downFocus() },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Mark()
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(28.dp)) {
            TopBarButton(LoungeIcons.Search, "Search", onSearch, focus, focus.search)
            if (showGuide) TopBarButton(LoungeIcons.Guide, "TV guide", onGuide, focus, focus.guide)
            if (showTvControls) TopBarButton(LoungeIcons.Tv, "TV controls", onTvControls, focus, focus.tvControls)
            if (nowPlaying != null) NowPlayingChip(nowPlaying, onNowPlaying, focus)
            TopBarButton(LoungeIcons.Settings, "Settings", onSettings, focus, focus.settings)
            Spacer(Modifier.width(12.dp))
            Clock()
        }
    }
}

/**
 * The mark (DESIGN.md §5.x): the sofa 56 dp wide with "Lounge" beside it, set as on the banner,
 * the word's baseline on the sofa's seat so the legs hang below the line as the g's tail does.
 */
@Composable
private fun Mark() {
    Row {
        Image(
            painterResource(tv.lounge.design.R.drawable.mark_sofa),
            contentDescription = null,
            modifier = Modifier.width(56.dp).alignBy { (it.measuredHeight * SEAT).roundToInt() },
        )
        Spacer(Modifier.width(14.dp))
        Text(
            "Lounge",
            style = LoungeTheme.typography.displayXl.copy(fontSize = 30.sp, lineHeight = 30.sp, letterSpacing = (-0.01).em),
            color = LoungeTheme.colors.limelight,
            modifier = Modifier.alignByBaseline(),
        )
    }
}

/** Where the sofa's seat meets its legs, as a fraction of the picture's height. */
private const val SEAT = 0.85f

/**
 * A top-bar button: glyph and label. Focus is a `curtainEdge` pill behind it with the tiles' rim,
 * the glyph in houselight, the full glow and a 1.04 scale, all read while drawing; the label does
 * not move.
 */
@Composable
private fun TopBarButton(icon: ImageVector, label: String, onClick: () -> Unit, focus: TopBarFocus, requester: FocusRequester) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    val visuals = LoungeTheme.visuals
    val sounds = LoungeTheme.sounds
    var focused by remember { mutableStateOf(false) }
    val scale = animateFloatAsState(if (focused) 1.04f else 1f, if (focused) motion.snap() else motion.settle(), label = "topBtnScale")
    val lift = animateFloatAsState(if (focused) 1f else 0f, if (focused) motion.snap() else motion.settle(), label = "topBtnLift")
    val glow = animateFloatAsState(if (focused) visuals.glowAlpha else 0f, if (focused) motion.snap() else motion.settle(), label = "topBtnGlow")
    Row(
        Modifier
            .focusRequester(requester)
            .graphicsLayer {
                scaleX = scale.value
                scaleY = scale.value
                translationX = if (focus.last === requester) focus.nudge.value else 0f
            }
            .lightGlow(colors.houselight, { glow.value }, cornerRadius = 24.dp)
            .drawBehind {
                val a = lift.value
                if (a > 0.001f) drawRoundRect(colors.curtainEdge.copy(alpha = a), cornerRadius = CornerRadius(size.height / 2f))
            }
            .clip(RoundedCornerShape(24.dp))
            .focusRim(colors, { lift.value }, 24.dp)
            .onFocusChanged {
                if (it.isFocused && !focused) sounds?.tick()
                focused = it.isFocused
                if (it.isFocused) focus.last = requester
            }
            .tilePress(onClick, onClick)
            .focusable()
            .semantics { role = Role.Button }
            .height(48.dp)
            .padding(horizontal = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Icon(icon, contentDescription = null, tint = if (focused) colors.houselight else colors.dust, modifier = Modifier.size(28.dp))
        Text(label, style = LoungeTheme.typography.meta, color = if (focused) colors.limelight else colors.dust)
    }
}

/**
 * What is playing, as a top-bar entry: three bars that move while sound is on, the title in `meta`,
 * the artwork's light as its colour. The bars step eight times a second and hold still when the
 * room is idle.
 */
@Composable
private fun NowPlayingChip(np: NowPlaying, onClick: () -> Unit, focus: TopBarFocus) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    val visuals = LoungeTheme.visuals
    val sounds = LoungeTheme.sounds
    var focused by remember { mutableStateOf(false) }
    val light = LoungeTheme.colors.light(np.lightArgb)
    val playing = np.status == PlaybackStatus.PLAYING
    val scale = animateFloatAsState(if (focused) 1.04f else 1f, if (focused) motion.snap() else motion.settle(), label = "chipScale")
    val lift = animateFloatAsState(if (focused) 1f else 0f, if (focused) motion.snap() else motion.settle(), label = "chipLift")
    val glow = animateFloatAsState(if (focused) visuals.glowAlpha else 0f, if (focused) motion.snap() else motion.settle(), label = "chipGlow")
    val phase = remember { mutableFloatStateOf(0f) }
    // Idle is read here, so the room going quiet recomposes this chip and nothing around it.
    val animate = playing && !LoungeTheme.lightState.idle && !motion.reducedMotion
    LaunchedEffect(animate) {
        while (animate) {
            delay(125)
            phase.floatValue = (phase.floatValue + 0.14f) % 1f
        }
    }
    Row(
        Modifier
            .focusRequester(focus.nowPlaying)
            .graphicsLayer { scaleX = scale.value; scaleY = scale.value }
            .lightGlow(light, { glow.value }, cornerRadius = 24.dp)
            .drawBehind {
                val a = lift.value
                if (a > 0.001f) drawRoundRect(colors.curtainEdge.copy(alpha = a), cornerRadius = CornerRadius(size.height / 2f))
            }
            .clip(RoundedCornerShape(24.dp))
            .focusRim(colors, { lift.value }, 24.dp)
            .onFocusChanged {
                if (it.isFocused && !focused) sounds?.tick()
                focused = it.isFocused
                if (it.isFocused) focus.last = focus.nowPlaying
            }
            .tilePress(onClick, onClick)
            .focusable()
            .semantics { role = Role.Button; contentDescription = "Now playing, ${np.title}" }
            .height(48.dp)
            .padding(horizontal = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Canvas(Modifier.size(28.dp)) {
            val w = size.width / 7f
            val heights = if (animate) {
                val t = phase.floatValue * 2f * Math.PI.toFloat()
                floatArrayOf(0.45f + 0.35f * sin(t), 0.55f + 0.4f * sin(t + 2.1f), 0.5f + 0.35f * sin(t + 4.2f))
            } else {
                floatArrayOf(0.45f, 0.75f, 0.55f)
            }
            heights.forEachIndexed { i, h ->
                val x = w * (1 + i * 2.5f)
                val barH = size.height * h.coerceIn(0.15f, 1f)
                drawRoundRect(light, topLeft = Offset(x, size.height - barH), size = Size(w * 1.4f, barH), cornerRadius = CornerRadius(w * 0.5f))
            }
        }
        Text(
            np.title,
            style = LoungeTheme.typography.meta,
            color = if (focused) colors.limelight else colors.dust,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.widthIn(max = 280.dp),
        )
    }
}

/**
 * The clock, as the TV's 24-hour setting has it and without am or pm. Only the digits that change
 * swap, with a 6 px rise, once a minute.
 */
@Composable
private fun Clock() {
    val is24Hour = is24HourClock()
    var now by remember { mutableStateOf(LocalTime.now()) }
    LaunchedEffect(Unit) {
        while (true) {
            now = LocalTime.now()
            delay(60_000L - (System.currentTimeMillis() % 60_000L) + 20)
        }
    }
    val text = Words.clock(now, is24Hour, period = false)
    Row {
        // Places are keyed from the right, so an hour gaining a digit shifts nothing else.
        text.forEachIndexed { i, ch ->
            key(text.length - i) {
                Swap(ch, rise = 6.dp, durationMs = 180) { c ->
                    Text(c.toString(), style = LoungeTheme.typography.clock, color = LoungeTheme.colors.limelight)
                }
            }
        }
    }
}
