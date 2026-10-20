package tv.lounge.feature.home

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
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
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.tv.material3.Text
import coil3.compose.AsyncImage
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import tv.lounge.core.CoreClient
import tv.lounge.core.Guide
import tv.lounge.core.GuideChannel
import tv.lounge.core.GuideProgramme
import tv.lounge.design.Dimens
import tv.lounge.design.LoungeTheme
import tv.lounge.design.argb
import tv.lounge.design.lightGlow
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter

private const val HALF_HOUR_MS = 30L * 60_000L
private const val WINDOW_MS = 5L * HALF_HOUR_MS
private val ChannelColumn = 240.dp
private val RowHeight = 72.dp

/**
 * The Live Guide (DESIGN.md §5.4): a bottom sheet with channels down the side and two and a half
 * hours across, laid out to the minute. OK on a programme tunes its channel. Refreshes every minute
 * while open.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun GuideSheet(core: CoreClient, visibleState: MutableTransitionState<Boolean>, onTune: (String) -> Unit, onDismiss: () -> Unit) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    val firstFocus = remember { FocusRequester() }
    var guide by remember { mutableStateOf<Guide?>(null) }
    val leaving = !visibleState.targetState
    LaunchedEffect(Unit) {
        while (true) {
            val now = System.currentTimeMillis()
            val from = (now - HALF_HOUR_MS / 2) / HALF_HOUR_MS * HALF_HOUR_MS
            guide = withContext(Dispatchers.IO) { core.guide(from, 3u) }
            delay(60_000L - now % 60_000L)
        }
    }

    Box(Modifier.fillMaxSize()) {
        AnimatedVisibility(visibleState, enter = fadeIn(tween(200)), exit = fadeOut(tween(180))) {
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
                    .onPreviewKeyEvent { e -> if (e.isBack() && e.type == KeyEventType.KeyUp) { onDismiss(); true } else false }
                    .focusProperties { canFocus = !leaving }
                    .trapFocus(leaving)
                    .focusGroup(),
            ) {
                Box(Modifier.fillMaxWidth().height(1.dp).background(colors.curtainEdge))
                val g = guide
                if (g == null) {
                    Spacer(Modifier.weight(1f))
                } else {
                    GuideHeader(g)
                    TimeRuler(g)
                    GuideGrid(g, firstFocus, onTune)
                }
            }
        }
    }
    LaunchedEffect(guide != null) { if (guide != null) runCatching { firstFocus.requestFocus() } }
}

@Composable
private fun GuideHeader(g: Guide) {
    val colors = LoungeTheme.colors
    Row(
        Modifier.fillMaxWidth().padding(start = Dimens.safeX, end = Dimens.safeX, top = 24.dp, bottom = 12.dp),
        verticalAlignment = Alignment.Bottom,
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Text("TV guide", style = LoungeTheme.typography.headline, color = colors.limelight)
        Text("${g.channels.size} channels, ${clock(g.nowMs)}", style = LoungeTheme.typography.meta, color = colors.dust)
    }
}

@Composable
private fun TimeRuler(g: Guide) {
    val colors = LoungeTheme.colors
    val gridWidth = 1920.dp - Dimens.safeX * 2 - ChannelColumn
    Row(Modifier.fillMaxWidth().padding(horizontal = Dimens.safeX).height(28.dp)) {
        Spacer(Modifier.width(ChannelColumn))
        Box(Modifier.width(gridWidth).fillMaxHeight()) {
            var t = g.fromMs
            while (t < g.fromMs + WINDOW_MS) {
                val x = gridWidth * ((t - g.fromMs).toFloat() / WINDOW_MS)
                Text(clock(t), style = LoungeTheme.typography.caption, color = colors.dust, modifier = Modifier.offset(x = x))
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
    Box(Modifier.fillMaxSize().padding(horizontal = Dimens.safeX)) {
        CompositionLocalProvider(LocalBringIntoViewSpec provides pivotSpec(rowHeaderPx)) {
            LazyColumn(Modifier.fillMaxSize(), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                items(g.channels, key = { it.id }) { ch ->
                    ChannelRow(ch, g, gridWidth, if (ch.id == g.channels.firstOrNull()?.id) firstFocus else null, onTune)
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
private fun ChannelRow(ch: GuideChannel, g: Guide, gridWidth: androidx.compose.ui.unit.Dp, firstFocus: FocusRequester?, onTune: (String) -> Unit) {
    val colors = LoungeTheme.colors
    Row(Modifier.fillMaxWidth().height(RowHeight)) {
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
                    g, gridWidth, firstFocus,
                ) { onTune(ch.action) }
            } else {
                visible.forEachIndexed { i, p ->
                    ProgrammeCell(p, g, gridWidth, if (i == 0) firstFocus else null) { onTune(ch.action) }
                }
            }
        }
    }
}

@Composable
private fun ProgrammeCell(p: GuideProgramme, g: Guide, gridWidth: androidx.compose.ui.unit.Dp, focusRequester: FocusRequester?, onClick: () -> Unit) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    val visuals = LoungeTheme.visuals
    val windowEnd = g.fromMs + WINDOW_MS
    val start = p.startMs.coerceAtLeast(g.fromMs)
    val end = p.endMs.coerceAtMost(windowEnd)
    val x = gridWidth * ((start - g.fromMs).toFloat() / WINDOW_MS)
    val w = (gridWidth * ((end - start).toFloat() / WINDOW_MS) - 4.dp).coerceAtLeast(12.dp)
    var focused by remember { mutableStateOf(false) }
    val glow = animateFloatAsState(if (focused) visuals.glowAlpha * 0.8f else 0f, if (focused) motion.snap() else motion.settle(), label = "cellGlow")
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
                if (p.isNow && !focused) {
                    drawLine(colors.houselight, Offset(0f, size.height - 1f), Offset(size.width, size.height - 1f), strokeWidth = 2f)
                }
            }
            .onFocusChanged { focused = it.isFocused }
            .tilePress(onClick, onClick)
            .focusable()
            .semantics { role = Role.Button; contentDescription = "${p.title}, ${clock(p.startMs)} to ${clock(p.endMs)}" }
            .padding(horizontal = 14.dp, vertical = 6.dp),
        verticalArrangement = Arrangement.Center,
    ) {
        Text(p.title, style = LoungeTheme.typography.meta, color = colors.limelight, maxLines = 1, overflow = TextOverflow.Ellipsis)
        Text("${clock(p.startMs)} – ${clock(p.endMs)}", style = LoungeTheme.typography.caption, color = colors.dust, maxLines = 1)
    }
}

private val clockFormat: DateTimeFormatter = DateTimeFormatter.ofPattern("HH:mm")

private fun clock(ms: Long): String = Instant.ofEpochMilli(ms).atZone(ZoneId.systemDefault()).format(clockFormat)
