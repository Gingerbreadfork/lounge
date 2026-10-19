package tv.lounge.design

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.Animatable
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
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.Stable
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
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
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
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

/** A [Swap]'s two slots. Composition reads the plain fields; the draw and focus lambdas read the states. */
private class SwapBook {
    val keys = arrayOfNulls<Any>(2)
    val contents = arrayOfNulls<@Composable () -> Unit>(2)
    var active = 0
    var leaving = -1
    val activeState = mutableIntStateOf(0)
    val leavingState = mutableIntStateOf(-1)
    val cleared = mutableIntStateOf(0)
}

/**
 * Content swap (DESIGN.md §6.3): the outgoing content fades over the first half while the incoming
 * one rises [rise] px, slides [shiftX] px and fades in after [delayMs]. Two persistent slots
 * alternate, so only the incoming content is composed and measured, and one progress value is read
 * only while drawing.
 */
@Composable
fun Swap(
    key: Any,
    delayMs: Int = 0,
    rise: Dp = 10.dp,
    shiftX: Dp = 0.dp,
    durationMs: Int? = null,
    /**
     * Keep the hidden slot composed so the next swap updates its nodes instead of creating them.
     */
    keepSlots: Boolean = false,
    content: @Composable () -> Unit,
) {
    val motion = LoungeTheme.motion
    val book = remember { SwapBook() }
    val progress = remember { Animatable(1f) }
    book.cleared.intValue
    when {
        book.keys[book.active] == null -> book.keys[book.active] = key
        book.keys[book.active] != key -> {
            val next = 1 - book.active
            book.keys[next] = key
            book.leaving = book.active
            book.active = next
            book.leavingState.intValue = book.leaving
            book.activeState.intValue = next
        }
    }
    book.contents[book.active] = content
    LaunchedEffect(key) {
        if (book.leaving < 0) return@LaunchedEffect
        progress.snapTo(0f)
        if (motion.reducedMotion) progress.snapTo(1f) else progress.animateTo(1f, tween(durationMs ?: motion.curtainMs, delayMillis = delayMs, easing = motion.curtainEasing))
        val gone = book.leaving
        book.leaving = -1
        book.leavingState.intValue = -1
        if (!keepSlots && gone >= 0 && gone != book.active) {
            book.contents[gone] = null
            book.keys[gone] = null
            book.cleared.intValue++
        }
    }
    Box {
        for (i in 0..1) {
            val slotContent = book.contents[i] ?: continue
            androidx.compose.runtime.key(i) {
                val live = remember { derivedStateOf { book.activeState.intValue == i } }
                Box(
                    Modifier
                        .focusProperties { canFocus = book.activeState.intValue == i }
                        .graphicsLayer {
                            val p = progress.value
                            val leaving = book.leavingState.intValue
                            when {
                                leaving == i -> {
                                    alpha = (1f - p * 2f).coerceIn(0f, 1f)
                                    translationX = -p * shiftX.toPx()
                                    translationY = 0f
                                }
                                book.activeState.intValue == i -> {
                                    alpha = if (leaving >= 0) p else 1f
                                    translationY = (1f - p) * rise.toPx()
                                    translationX = (1f - p) * shiftX.toPx()
                                }
                                else -> alpha = 0f
                            }
                        },
                ) { CompositionLocalProvider(LocalSwapSlotActive provides live) { slotContent() } }
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
                    .padding(horizontal = 24.dp, vertical = 18.dp),
            ) {
                Text(shown, style = LoungeTheme.typography.title, color = colors.limelight, maxLines = 2, overflow = TextOverflow.Ellipsis)
            }
        }
    }
}
