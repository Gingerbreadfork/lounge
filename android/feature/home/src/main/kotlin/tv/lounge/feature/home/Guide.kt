package tv.lounge.feature.home

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.focusGroup
import androidx.compose.foundation.focusable
import androidx.compose.foundation.gestures.LocalBringIntoViewSpec
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.key
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.tv.material3.Text
import coil3.compose.AsyncImage
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import tv.lounge.core.Guide
import tv.lounge.core.GuideChannel
import tv.lounge.core.GuideProgramme
import tv.lounge.design.Dimens
import tv.lounge.design.LoungeTheme
import tv.lounge.design.Words
import tv.lounge.design.argb
import tv.lounge.design.focusRim
import tv.lounge.design.is24HourClock
import tv.lounge.design.lightGlow

private const val HALF_HOUR_MS = 30L * 60_000L
private const val WINDOW_MS = 5L * HALF_HOUR_MS
private val ChannelColumn = 240.dp
private val RowHeight = 72.dp

/**
 * The TV guide (DESIGN.md §5.4): a bottom sheet with channels down the side and two and a half
 * hours across, laid out to the minute. OK on a programme tunes its channel. Refreshes every minute
 * while open. Its heading takes focus at once, so while the guide is read the d-pad stays in the
 * sheet rather than acting on the home behind it.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun GuideSheet(loadGuide: (Long) -> Guide, visibleState: MutableTransitionState<Boolean>, onTune: (String) -> Unit, onDismiss: () -> Unit) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    val firstFocus = remember { FocusRequester() }
    val headingFocus = remember { FocusRequester() }
    var guide by remember { mutableStateOf<Guide?>(null) }
    val leaving = !visibleState.targetState
    // The window stays put while the guide is open, so programmes do not slide under the
    // focus; it moves on only once the present nears its right-hand edge.
    var from by remember { mutableLongStateOf(0L) }
    WhileStarted {
        while (true) {
            val now = System.currentTimeMillis()
            if (from == 0L || now > from + WINDOW_MS - HALF_HOUR_MS) from = (now - HALF_HOUR_MS / 2) / HALF_HOUR_MS * HALF_HOUR_MS
            guide = withContext(Dispatchers.IO) { loadGuide(from) }
            delay(60_000L - now % 60_000L)
        }
    }
    val hasChannels = guide?.channels?.isNotEmpty() == true
    // The heading lets go of focus only once a programme has it, so focus is never dropped between them.
    var landed by remember { mutableStateOf(false) }

    Box(Modifier.fillMaxSize()) {
        AnimatedVisibility(visibleState, enter = fadeIn(motion.timed(200)), exit = fadeOut(motion.timed(180))) {
            Box(Modifier.fillMaxSize().background(colors.scrim))
        }
        AnimatedVisibility(
            visibleState,
            enter = slideInVertically(motion.proscenium()) { it } + fadeIn(motion.proscenium()),
            exit = slideOutVertically(motion.settle()) { it } + fadeOut(motion.settle()),
            modifier = Modifier.align(Alignment.BottomCenter),
        ) {
            Column(
                Modifier
                    .fillMaxWidth()
                    .height(680.dp)
                    .background(colors.curtain)
                    .semantics { paneTitle = "TV guide" }
                    .onPreviewKeyEvent { e -> if (e.isBack() && e.type == KeyEventType.KeyUp) { onDismiss(); true } else false }
                    .focusProperties { canFocus = !leaving }
                    .trapFocus(leaving)
                    .focusGroup(),
            ) {
                Box(Modifier.fillMaxWidth().height(1.dp).background(colors.curtainEdge))
                val g = guide
                GuideHeader(g, headingFocus, holdsFocus = !hasChannels || !landed)
                if (g != null && hasChannels) {
                    TimeRuler(g)
                    GuideGrid(g, firstFocus, onTune)
                }
            }
        }
    }
    LaunchedEffect(Unit) { headingFocus.requestWhenReady() }
    LaunchedEffect(hasChannels) { if (hasChannels) landed = firstFocus.requestWhenReady() }
}

/**
 * "TV guide" with how many channels and the time, or what it is waiting for. Until there is a
 * programme to land on, the heading itself holds focus, so Back still closes the sheet.
 */
@Composable
private fun GuideHeader(g: Guide?, focus: FocusRequester, holdsFocus: Boolean) {
    val colors = LoungeTheme.colors
    val is24Hour = is24HourClock()
    val status = when {
        g == null -> "Reading the guide"
        g.channels.isEmpty() -> "No channels yet"
        else -> "${Words.counted(g.channels.size, "channel", "channels")}, ${Words.clock(g.nowMs, is24Hour)}"
    }
    Column(
        Modifier
            .fillMaxWidth()
            .focusRequester(focus)
            .then(if (holdsFocus) Modifier.focusable() else Modifier)
            .padding(start = Dimens.safeX, end = Dimens.safeX, top = 24.dp, bottom = 12.dp),
    ) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.SpaceBetween) {
            Text("TV guide", style = LoungeTheme.typography.headline, color = colors.limelight, modifier = Modifier.semantics { heading() })
            Text(status, style = LoungeTheme.typography.meta, color = colors.dust, modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite })
        }
        if (g != null && g.channels.isEmpty()) {
            Spacer(Modifier.height(16.dp))
            Text("Channels show here once a TV app or a plugin lists what is on.", style = LoungeTheme.typography.body, color = colors.dust)
        }
    }
}

@Composable
private fun TimeRuler(g: Guide) {
    val colors = LoungeTheme.colors
    val is24Hour = is24HourClock()
    val gridWidth = 1920.dp - Dimens.safeX * 2 - ChannelColumn
    // As tall as a caption line, so large text is not cut.
    val height = maxOf(28.dp, with(LocalDensity.current) { LoungeTheme.typography.caption.lineHeight.toDp() } + 2.dp)
    Row(Modifier.fillMaxWidth().padding(horizontal = Dimens.safeX).height(height)) {
        Spacer(Modifier.width(ChannelColumn))
        Box(Modifier.width(gridWidth).fillMaxHeight()) {
            var t = g.fromMs
            while (t < g.fromMs + WINDOW_MS) {
                val x = gridWidth * ((t - g.fromMs).toFloat() / WINDOW_MS)
                Text(Words.clock(t, is24Hour), style = LoungeTheme.typography.caption, color = colors.dust, modifier = Modifier.offset(x = x))
                t += HALF_HOUR_MS
            }
        }
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun GuideGrid(g: Guide, firstFocus: FocusRequester, onTune: (String) -> Unit) {
    val colors = LoungeTheme.colors
    val gridWidth = 1920.dp - Dimens.safeX * 2 - ChannelColumn
    val nowX = gridWidth * ((g.nowMs - g.fromMs).toFloat() / WINDOW_MS).coerceIn(0f, 1f)
    val rowHeaderPx = with(LocalDensity.current) { 0.dp.toPx() }
    // Rows grow with large text rather than cutting the title or its times (DESIGN.md §4.1).
    val type = LoungeTheme.typography
    val rowHeight = maxOf(RowHeight, with(LocalDensity.current) { type.meta.lineHeight.toDp() + type.caption.lineHeight.toDp() } + 16.dp)
    Box(Modifier.fillMaxSize().padding(horizontal = Dimens.safeX)) {
        CompositionLocalProvider(LocalBringIntoViewSpec provides pivotSpec(rowHeaderPx)) {
            LazyColumn(Modifier.fillMaxSize(), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                items(g.channels, key = { it.id }) { ch ->
                    ChannelRow(ch, g, gridWidth, rowHeight, if (ch.id == g.channels.firstOrNull()?.id) firstFocus else null, onTune)
                }
            }
        }
        // The present moment, as a thin line of house light down the grid.
        Box(
            Modifier
                .offset(x = ChannelColumn + nowX)
                .width(2.dp)
                .fillMaxHeight()
                .background(colors.houselight.copy(alpha = 0.7f)),
        )
    }
}

@Composable
private fun ChannelRow(ch: GuideChannel, g: Guide, gridWidth: Dp, rowHeight: Dp, firstFocus: FocusRequester?, onTune: (String) -> Unit) {
    val colors = LoungeTheme.colors
    Row(Modifier.fillMaxWidth().height(rowHeight)) {
        Row(Modifier.width(ChannelColumn).fillMaxHeight().padding(end = 16.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Box(Modifier.size(40.dp).clip(RoundedCornerShape(8.dp)).background(LoungeTheme.colors.light(ch.lightArgb).copy(alpha = 0.3f))) {
                if (ch.logoPath != null) AsyncImage(model = ch.logoPath, contentDescription = null, contentScale = ContentScale.Fit, modifier = Modifier.fillMaxSize())
            }
            Column {
                ch.number?.let { Text(it, style = LoungeTheme.typography.caption, color = colors.dust) }
                Text(ch.name, style = LoungeTheme.typography.meta, color = colors.limelight, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
        }
        Box(Modifier.width(gridWidth).fillMaxHeight()) {
            val visible = ch.programmes.filter { it.endMs > g.fromMs && it.startMs < g.fromMs + WINDOW_MS }
            if (visible.isEmpty()) {
                ProgrammeCell(
                    GuideProgramme(id = "${ch.id}:none", title = "No programme information", subtitle = null, startMs = g.fromMs, endMs = g.fromMs + WINDOW_MS, isNow = false),
                    ch, g, gridWidth, firstFocus,
                ) { onTune(ch.action) }
            } else {
                // Keyed by programme, so a refresh keeps each cell, and the focus in it, in place.
                visible.forEachIndexed { i, p ->
                    key(p.id) { ProgrammeCell(p, ch, g, gridWidth, if (i == 0) firstFocus else null) { onTune(ch.action) } }
                }
            }
        }
    }
}

/**
 * One programme, laid out to the minute. The one on air says so in words and keeps its
 * `houselight` underline under focus, so the present is never told by colour alone.
 */
@Composable
private fun ProgrammeCell(p: GuideProgramme, ch: GuideChannel, g: Guide, gridWidth: Dp, focusRequester: FocusRequester?, onClick: () -> Unit) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    val visuals = LoungeTheme.visuals
    val is24Hour = is24HourClock()
    val windowEnd = g.fromMs + WINDOW_MS
    val start = p.startMs.coerceAtLeast(g.fromMs)
    val end = p.endMs.coerceAtMost(windowEnd)
    val x = gridWidth * ((start - g.fromMs).toFloat() / WINDOW_MS)
    val w = (gridWidth * ((end - start).toFloat() / WINDOW_MS) - 4.dp).coerceAtLeast(12.dp)
    var focused by remember { mutableStateOf(false) }
    val glow = animateFloatAsState(if (focused) visuals.glowAlpha * 0.8f else 0f, if (focused) motion.snap() else motion.settle(), label = "cellGlow")
    val rim = animateFloatAsState(if (focused) 1f else 0f, if (focused) motion.snap() else motion.settle(), label = "cellRim")
    val times = programmeTimes(p, is24Hour)
    Column(
        Modifier
            .offset(x = x)
            .width(w)
            .fillMaxHeight()
            .then(if (focusRequester != null) Modifier.focusRequester(focusRequester) else Modifier)
            .lightGlow(colors.houselight, { glow.value }, radius = 24.dp, cornerRadius = 10.dp)
            .clip(RoundedCornerShape(10.dp))
            .background(if (focused) colors.curtainEdge else if (p.isNow) colors.velvet.copy(alpha = 0.6f) else colors.velvet.copy(alpha = 0.35f))
            .drawBehind {
                if (p.isNow) drawLine(colors.houselight, Offset(0f, size.height - 1f), Offset(size.width, size.height - 1f), strokeWidth = 2f)
            }
            .focusRim(colors, { rim.value }, 10.dp)
            .onFocusChanged { focused = it.isFocused }
            .tilePress(onClick, onClick, clickLabel = "Watch")
            .focusable()
            .semantics { role = Role.Button; contentDescription = "${ch.name}, ${p.title}, $times" }
            .padding(horizontal = 14.dp, vertical = 6.dp),
        verticalArrangement = Arrangement.Center,
    ) {
        Text(p.title, style = LoungeTheme.typography.meta, color = colors.limelight, maxLines = 1, overflow = TextOverflow.Ellipsis)
        Text(times, style = LoungeTheme.typography.caption, color = colors.dust, maxLines = 1)
    }
}

/** "On now, until 21:30" for what is on air, else "21:00 to 21:30". */
internal fun programmeTimes(p: GuideProgramme, is24Hour: Boolean): String =
    if (p.isNow) "On now, until ${Words.clock(p.endMs, is24Hour)}"
    else "${Words.clock(p.startMs, is24Hour)} to ${Words.clock(p.endMs, is24Hour)}"
