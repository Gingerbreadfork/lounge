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
import androidx.tv.material3.Icon
import androidx.tv.material3.Text
import kotlinx.coroutines.delay
import tv.lounge.core.NowPlaying
import tv.lounge.core.PlaybackStatus
import tv.lounge.design.Dimens
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme
import tv.lounge.design.Swap
import tv.lounge.design.argb
import tv.lounge.design.lightGlow
import kotlin.math.sin
import java.time.LocalTime
import java.time.format.DateTimeFormatter
import java.util.Locale

/** Top bar (DESIGN.md §5.1): the mark on the left; Search, TV controls, Settings and the clock on the right. */
@Composable
fun TopBar(
    showTvControls: Boolean,
    showGuide: Boolean,
    nowPlaying: NowPlaying?,
    idle: Boolean,
    firstFocus: FocusRequester,
    downFocus: () -> FocusRequester,
    onFocused: () -> Unit,
    onSearch: () -> Unit,
    onTvControls: () -> Unit,
    onGuide: () -> Unit,
    onNowPlaying: () -> Unit,
    onSettings: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier
            .fillMaxWidth()
            .padding(start = Dimens.safeX, end = Dimens.safeX, top = Dimens.safeY)
            .height(Dimens.topBarHeight)
            .onFocusChanged { if (it.hasFocus) onFocused() }
            .focusRestorer()
            .focusGroup()
            .focusProperties { down = downFocus() },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Mark()
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(28.dp)) {
            TopBarButton(LoungeIcons.Search, "Search", onSearch, Modifier.focusRequester(firstFocus))
            if (showGuide) TopBarButton(LoungeIcons.Tv, "Guide", onGuide)
            if (showTvControls) TopBarButton(LoungeIcons.Tv, "TV controls", onTvControls)
            if (nowPlaying != null) NowPlayingChip(nowPlaying, idle, onNowPlaying)
            TopBarButton(LoungeIcons.Settings, "Settings", onSettings)
            Spacer(Modifier.width(12.dp))
            Clock()
        }
    }
}

/** The mark: the sofa, 42 dp wide, on the velvet. */
@Composable
private fun Mark() {
    Image(painterResource(tv.lounge.design.R.drawable.mark_sofa), contentDescription = null, modifier = Modifier.width(42.dp))
}

/**
 * A top-bar button: glyph and label. Focus is a `curtainEdge` pill behind it (the label does
 * not move), the glyph in houselight, the full glow and a 1.04 scale, all read while drawing.
 */
@Composable
private fun TopBarButton(icon: ImageVector, label: String, onClick: () -> Unit, modifier: Modifier = Modifier) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    val visuals = LoungeTheme.visuals
    var focused by remember { mutableStateOf(false) }
    val scale = animateFloatAsState(if (focused) 1.04f else 1f, if (focused) motion.snap() else motion.settle(), label = "topBtnScale")
    val lift = animateFloatAsState(if (focused) 1f else 0f, if (focused) motion.snap() else motion.settle(), label = "topBtnLift")
    val glow = animateFloatAsState(if (focused) visuals.glowAlpha else 0f, if (focused) motion.snap() else motion.settle(), label = "topBtnGlow")
    Row(
        modifier
            .graphicsLayer { scaleX = scale.value; scaleY = scale.value }
            .lightGlow(colors.houselight, { glow.value }, cornerRadius = 24.dp)
            .drawBehind {
                val a = lift.value
                if (a > 0.001f) drawRoundRect(colors.curtainEdge.copy(alpha = a), cornerRadius = CornerRadius(size.height / 2f))
            }
            .clip(RoundedCornerShape(24.dp))
            .onFocusChanged { focused = it.isFocused }
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
private fun NowPlayingChip(np: NowPlaying, idle: Boolean, onClick: () -> Unit) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    val visuals = LoungeTheme.visuals
    var focused by remember { mutableStateOf(false) }
    val light = LoungeTheme.colors.light(np.lightArgb)
    val playing = np.status == PlaybackStatus.PLAYING
    val scale = animateFloatAsState(if (focused) 1.04f else 1f, if (focused) motion.snap() else motion.settle(), label = "chipScale")
    val lift = animateFloatAsState(if (focused) 1f else 0f, if (focused) motion.snap() else motion.settle(), label = "chipLift")
    val glow = animateFloatAsState(if (focused) visuals.glowAlpha else 0f, if (focused) motion.snap() else motion.settle(), label = "chipGlow")
    val phase = remember { mutableFloatStateOf(0f) }
    val animate = playing && !idle && !motion.reducedMotion
    LaunchedEffect(animate) {
        while (animate) {
            delay(125)
            phase.floatValue = (phase.floatValue + 0.14f) % 1f
        }
    }
    Row(
        Modifier
            .graphicsLayer { scaleX = scale.value; scaleY = scale.value }
            .lightGlow(light, { glow.value }, cornerRadius = 24.dp)
            .drawBehind {
                val a = lift.value
                if (a > 0.001f) drawRoundRect(colors.curtainEdge.copy(alpha = a), cornerRadius = CornerRadius(size.height / 2f))
            }
            .clip(RoundedCornerShape(24.dp))
            .onFocusChanged { focused = it.isFocused }
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

/** The clock. Only the digits that change swap, with a 6 px rise, once a minute. */
@Composable
private fun Clock() {
    val context = androidx.compose.ui.platform.LocalContext.current
    val is24Hour = android.text.format.DateFormat.is24HourFormat(context)
    val formatter = remember(is24Hour) { DateTimeFormatter.ofPattern(if (is24Hour) "HH:mm" else "h:mm", Locale.getDefault()) }
    var now by remember { mutableStateOf(LocalTime.now()) }
    LaunchedEffect(Unit) {
        while (true) {
            now = LocalTime.now()
            delay(60_000L - (System.currentTimeMillis() % 60_000L) + 20)
        }
    }
    val text = now.format(formatter)
    Row {
        text.forEachIndexed { i, ch ->
            Swap(key = "$i:$ch", rise = 6.dp, durationMs = 180) {
                Text(ch.toString(), style = LoungeTheme.typography.clock, color = LoungeTheme.colors.limelight)
            }
        }
    }
}
