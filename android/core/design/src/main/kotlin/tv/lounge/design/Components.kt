package tv.lounge.design

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.Easing
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.animate
import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.focusable
import androidx.compose.foundation.interaction.MutableInteractionSource
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
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.Stable
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.zIndex
import androidx.compose.foundation.clickable
import androidx.tv.material3.Icon
import androidx.tv.material3.Text
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first

enum class ButtonKind { Primary, Secondary }

/**
 * Pill button (DESIGN.md §7.2): 56 dp tall, label in `title`, focus = scale 1.04 + glow in `light`.
 */
@Composable
fun LoungeButton(
    label: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    kind: ButtonKind = ButtonKind.Primary,
    light: Color = LoungeTheme.colors.houselight,
    icon: ImageVector? = null,
    enabled: Boolean = true,
    focusRequester: androidx.compose.ui.focus.FocusRequester? = null,
) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    val visuals = LoungeTheme.visuals
    var focused by remember { mutableStateOf(false) }
    val scale = animateFloatAsState(if (focused) 1.04f else 1f, if (focused) motion.snap() else motion.settle(), label = "buttonScale")
    val glow = animateFloatAsState(if (focused) visuals.glowAlpha else 0f, if (focused) motion.snap() else motion.settle(), label = "buttonGlow")
    val background = when (kind) {
        ButtonKind.Primary -> light
        ButtonKind.Secondary -> colors.curtain
    }
    val content = when (kind) {
        ButtonKind.Primary -> textOn(light, colors)
        ButtonKind.Secondary -> colors.limelight
    }
    val shape = RoundedCornerShape(28.dp)
    Box(
        modifier
            .then(if (focusRequester != null) Modifier.focusRequester(focusRequester) else Modifier)
            .graphicsLayer { scaleX = scale.value; scaleY = scale.value }
            .lightGlow(light, { glow.value }, cornerRadius = 28.dp)
            .clip(shape)
            .background(background)
            .focusRing(colors, { focused }, 28.dp)
            .then(if (kind == ButtonKind.Secondary) Modifier.border(1.dp, colors.curtainEdge, shape) else Modifier)
            .onFocusChanged { focused = it.isFocused }
            .clickable(enabled = enabled, interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = onClick)
            .semantics { role = Role.Button }
            .height(56.dp)
            .padding(horizontal = 32.dp),
        contentAlignment = Alignment.Center,
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            if (icon != null) Icon(icon, contentDescription = null, tint = content, modifier = Modifier.size(28.dp))
            Text(label, style = LoungeTheme.typography.title, color = content, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

/** Live / connected indicator dot (10 dp). */
@Composable
fun Dot(color: Color, modifier: Modifier = Modifier) {
    Box(modifier.size(10.dp).clip(CircleShape).background(color))
}

/**
 * The live dot: `ember`, pulsing once every 4 s (opacity 1 → 0.6 → 1 over 600 ms, DESIGN.md
 * §7.3); still while idle and in reduced motion. The pulse is read while drawing.
 */
@Composable
fun LiveDot(idle: Boolean, modifier: Modifier = Modifier) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    val pulse = remember { Animatable(1f) }
    LaunchedEffect(idle, motion.reducedMotion) {
        if (idle || motion.reducedMotion) {
            pulse.snapTo(1f)
            return@LaunchedEffect
        }
        while (true) {
            delay(4_000)
            pulse.animateTo(0.6f, tween(300))
            pulse.animateTo(1f, tween(300))
        }
    }
    Box(
        modifier.size(10.dp).drawBehind {
            drawCircle(colors.ember.copy(alpha = pulse.value))
        },
    )
}

/**
 * Progress bar drawn in one pass: `houselight` on `limelight` at 22 %. [brightness] follows the
 * tile's house light so a rested tile's bar is not its brightest part (DESIGN.md §7.1).
 */
@Composable
fun ProgressBar(
    progress: Float,
    modifier: Modifier = Modifier,
    thickness: Dp = 4.dp,
    brightness: () -> Float = { 1f },
) {
    val colors = LoungeTheme.colors
    val p = progress.coerceIn(0f, 1f)
    val track = colors.limelight.copy(alpha = 0.22f)
    Box(
        modifier.height(thickness).drawBehind {
            val r = CornerRadius(size.height / 2f)
            val b = brightness().coerceIn(0f, 1f)
            drawRoundRect(track.copy(alpha = track.alpha * b), cornerRadius = r)
            val w = size.width * p
            if (w > 0f) {
                val fill = Color(colors.houselight.red * b, colors.houselight.green * b, colors.houselight.blue * b)
                drawRoundRect(fill, size = Size(w, size.height), cornerRadius = r)
            }
        },
    )
}

/** A badge: optional dot plus `caption` text, e.g. "Live" or "New". */
@Composable
fun Badge(text: String, dot: Color? = null, textColor: Color = LoungeTheme.colors.limelight, modifier: Modifier = Modifier, idle: Boolean = false) {
    val colors = LoungeTheme.colors
    Row(modifier, verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        if (dot == colors.ember) LiveDot(idle) else if (dot != null) Dot(dot)
        Text(text, style = LoungeTheme.typography.caption, color = textColor)
    }
}

/** Hairline used only where a surface must be separated from the backdrop. */
@Composable
fun Hairline(modifier: Modifier = Modifier) {
    Box(modifier.height(1.dp).background(LoungeTheme.colors.curtainEdge))
}

@Composable
fun HSpacer(width: Dp) = Spacer(Modifier.width(width))

@Composable
fun VSpacer(height: Dp) = Spacer(Modifier.height(height))

/** A focusable region that reports focus and nothing else; used by overlays' first items. */
fun Modifier.focusReporter(onChange: (Boolean) -> Unit): Modifier = this.onFocusChanged { onChange(it.isFocused) }.focusable()

fun Modifier.describe(description: String): Modifier = semantics { contentDescription = description }

/** Whether the enclosing [Swap] slot is the live one; hidden copies read false. */
val LocalSwapSlotActive = staticCompositionLocalOf<State<Boolean>> { mutableStateOf(true) }

private const val SWAP_SLOTS = 3

/**
 * A [Swap]'s slots. Composition reads and writes the plain fields when the target changes; drawing
 * reads [clock] and [stamp], focus reads [activeState], and composition reads [cleared].
 */
private class SwapBook {
    val keys = arrayOfNulls<Any>(SWAP_SLOTS)
    val values = arrayOfNulls<Any>(SWAP_SLOTS)
    val filled = BooleanArray(SWAP_SLOTS)
    /** Drawing order: the most recent target is drawn on top. */
    val order = IntArray(SWAP_SLOTS)
    var counter = 0
    var active = 0
    val activeState = mutableIntStateOf(0)
    val cleared = mutableIntStateOf(0)
    val stamp = mutableIntStateOf(0)
    /** Milliseconds into the current swap; one write a frame moves every slot. */
    val clock = mutableFloatStateOf(0f)
    var easing: Easing = LinearEasing
    /** Each slot's look as alpha, x, y and scale, at the start and the end of its part of the swap. */
    val from = Array(SWAP_SLOTS) { floatArrayOf(0f, 0f, 0f, 1f) }
    val to = Array(SWAP_SLOTS) { floatArrayOf(0f, 0f, 0f, 1f) }
    val startMs = FloatArray(SWAP_SLOTS)
    val durationMs = FloatArray(SWAP_SLOTS)

    fun fraction(i: Int, clock: Float): Float =
        if (durationMs[i] <= 0f) 1f else easing.transform(((clock - startMs[i]) / durationMs[i]).coerceIn(0f, 1f))

    fun value(i: Int, k: Int, clock: Float): Float = from[i][k] + (to[i][k] - from[i][k]) * fraction(i, clock)

    fun look(i: Int): FloatArray {
        val c = Snapshot.withoutReadObservation { clock.floatValue }
        return FloatArray(4) { k -> value(i, k, c) }
    }

    /** Puts a slot at [look] and keeps it there. */
    fun hold(i: Int, look: FloatArray) {
        from[i] = look
        to[i] = look.copyOf()
        durationMs[i] = 0f
        stamp.intValue++
    }
}

private val SwapRest = floatArrayOf(1f, 0f, 0f, 1f)

/**
 * Content swap (DESIGN.md §6.3): the outgoing content fades over [outFraction] of the time and the
 * incoming one fades in after [delayMs] and [inDelayFraction], from [shiftX], [rise] and [fromScale].
 * Three slots each draw their own target, newest on top, so a change mid-swap carries on from
 * whatever is on screen instead of cutting it.
 */
@Composable
fun <T> Swap(
    target: T,
    modifier: Modifier = Modifier,
    delayMs: Int = 0,
    rise: Dp = 10.dp,
    shiftX: Dp = 0.dp,
    fromScale: Float = 1f,
    durationMs: Int? = null,
    outFraction: Float = 0.5f,
    inDelayFraction: Float = 0f,
    /** Keep hidden slots composed so the next swap updates their nodes instead of creating them. */
    keepSlots: Boolean = false,
    /** Draw each slot's alpha straight into its content, without an offscreen layer. */
    modulateAlpha: Boolean = false,
    keyOf: (T) -> Any? = { it },
    content: @Composable (T) -> Unit,
) {
    val motion = LoungeTheme.motion
    val density = LocalDensity.current
    val shiftPx = with(density) { shiftX.toPx() }
    val risePx = with(density) { rise.toPx() }
    val book = remember { SwapBook() }
    book.easing = motion.curtainEasing
    book.cleared.intValue
    val key = keyOf(target)
    when {
        !book.filled[book.active] -> {
            book.keys[book.active] = key
            book.filled[book.active] = true
            book.hold(book.active, SwapRest.copyOf())
        }
        book.keys[book.active] != key -> Snapshot.withoutReadObservation {
            // Back to something still on screen: it turns around. Otherwise the least visible slot,
            // an empty one first, takes the new target.
            val back = (0 until SWAP_SLOTS).firstOrNull { it != book.active && book.filled[it] && book.keys[it] == key }
            val chosen = back ?: (0 until SWAP_SLOTS).minBy { if (book.filled[it]) book.look(it)[0] else -1f }
            if (back == null) {
                book.keys[chosen] = key
                book.filled[chosen] = true
                book.hold(chosen, floatArrayOf(0f, shiftPx, risePx, fromScale))
            }
            book.order[chosen] = ++book.counter
            book.active = chosen
            book.activeState.intValue = chosen
        }
    }
    book.values[book.active] = target
    LaunchedEffect(key) {
        val incoming = book.active
        val total = (durationMs ?: motion.curtainMs).toFloat()
        var end = 0f
        for (i in 0 until SWAP_SLOTS) {
            if (!book.filled[i]) continue
            val now = book.look(i)
            val target = if (i == incoming) SwapRest.copyOf() else floatArrayOf(0f, -shiftPx, 0f, 1f)
            if (now.contentEquals(target) || (now[0] == 0f && target[0] == 0f)) {
                book.hold(i, target)
                continue
            }
            val wait = when {
                i != incoming -> 0f
                now[0] > 0f -> 0f
                else -> delayMs + total * inDelayFraction
            }
            book.from[i] = now
            book.to[i] = target
            book.startMs[i] = wait
            book.durationMs[i] = if (i == incoming) total * (1f - inDelayFraction) else total * outFraction
            end = maxOf(end, wait + book.durationMs[i])
        }
        // The looks changed under a clock that may not move (reduced motion lands on the same end).
        book.stamp.intValue++
        if (end > 0f) {
            if (motion.reducedMotion) {
                book.clock.floatValue = end
            } else {
                book.clock.floatValue = 0f
                animate(0f, end, animationSpec = tween(end.toInt(), easing = LinearEasing)) { v, _ -> book.clock.floatValue = v }
            }
        }
        if (!keepSlots) {
            var removed = false
            for (i in 0 until SWAP_SLOTS) {
                if (i == book.active || !book.filled[i]) continue
                book.filled[i] = false
                book.values[i] = null
                book.keys[i] = null
                removed = true
            }
            if (removed) book.cleared.intValue++
        }
    }
    Box(modifier) {
        for (i in 0 until SWAP_SLOTS) {
            if (!book.filled[i]) continue
            @Suppress("UNCHECKED_CAST")
            val value = book.values[i] as T
            androidx.compose.runtime.key(i, if (keepSlots) null else book.keys[i]) {
                val live = remember { derivedStateOf { book.activeState.intValue == i } }
                Box(
                    Modifier
                        .zIndex(book.order[i].toFloat())
                        .focusProperties { if (book.activeState.intValue != i) canFocus = false }
                        .graphicsLayer {
                            book.stamp.intValue
                            val c = book.clock.floatValue
                            alpha = book.value(i, 0, c)
                            translationX = book.value(i, 1, c)
                            translationY = book.value(i, 2, c)
                            val sc = book.value(i, 3, c)
                            scaleX = sc
                            scaleY = sc
                            if (modulateAlpha) compositingStrategy = CompositingStrategy.ModulateAlpha
                        },
                ) { CompositionLocalProvider(LocalSwapSlotActive provides live) { content(value) } }
            }
        }
    }
}

/**
 * Keeps an overlay composed through its exit. [item] is what should show, null for nothing;
 * [content] gets the item and a transition state for `AnimatedVisibility`. A new item swaps in
 * place; null plays the exit first.
 */
@Composable
fun <T : Any> Presence(item: T?, content: @Composable (item: T, visibleState: MutableTransitionState<Boolean>) -> Unit) {
    var shown by remember { mutableStateOf<T?>(null) }
    val visibleState = remember { MutableTransitionState(false) }
    SideEffect { if (item != null && shown != item) shown = item }
    visibleState.targetState = item != null
    val current = item ?: shown ?: return
    LaunchedEffect(item == null) {
        if (item == null) {
            snapshotFlow { visibleState.isIdle && !visibleState.currentState }.first { it }
            shown = null
        }
    }
    content(current, visibleState)
}

/** One toast at a time, shown for four seconds (DESIGN.md §7.4). */
@Stable
class ToastController {
    var current: String? by mutableStateOf(null)
        private set
    private var serial = 0

    fun show(text: String) {
        current = text
        serial++
    }

    internal val version: Int get() = serial

    internal fun clear(version: Int) {
        if (serial == version) current = null
    }
}

/**
 * The toast host: a `curtain` card bottom-left that rises 16 px over `proscenium` and fades out
 * over `settle`. Place it last in the screen's root box.
 */
@Composable
fun LoungeToastHost(toasts: ToastController, modifier: Modifier = Modifier) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    val text = toasts.current
    LaunchedEffect(text, toasts.version) {
        if (text != null) {
            val v = toasts.version
            delay(4_000)
            toasts.clear(v)
        }
    }
    val rise = with(androidx.compose.ui.platform.LocalDensity.current) { 16.dp.roundToPx() }
    Presence(text) { shown, visibleState ->
        AnimatedVisibility(
            visibleState = visibleState,
            enter = fadeIn(motion.proscenium()) + slideInVertically(motion.proscenium()) { rise },
            exit = fadeOut(motion.settle()),
            modifier = modifier.padding(start = Dimens.safeX, bottom = Dimens.safeY),
        ) {
            Box(
                Modifier
                    .widthIn(max = 900.dp)
                    .clip(RoundedCornerShape(16.dp))
                    .background(colors.curtain)
                    .border(1.dp, colors.curtainEdge, RoundedCornerShape(16.dp))
                    .padding(horizontal = 24.dp, vertical = 18.dp)
                    .semantics { liveRegion = LiveRegionMode.Polite },
            ) {
                Text(shown, style = LoungeTheme.typography.title, color = colors.limelight, maxLines = 2, overflow = TextOverflow.Ellipsis)
            }
        }
    }
}
