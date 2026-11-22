package tv.lounge.feature.home

import android.content.res.Resources
import androidx.annotation.PluralsRes
import androidx.annotation.StringRes
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.focusGroup
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.requiredWidth
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.imageResource
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp
import androidx.tv.material3.Text
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import tv.lounge.core.QrCode
import tv.lounge.design.Dimens
import tv.lounge.design.LoungeTheme
import tv.lounge.design.SpillMask
import tv.lounge.design.Swap
import tv.lounge.design.drawSpill
import tv.lounge.design.lightGlow
import kotlin.math.roundToInt

/**
 * One thing a code on these pages can hold: the row's word for it, what the code itself says,
 * what is written under it, and a line of care where a mistake would cost something. The word
 * is a product's own [name], or the text [named] where it has none. The [note] may have a place
 * for the name.
 */
internal data class CodeChoice(val name: String?, val code: String, val shown: String, @StringRes val note: Int? = null, @StringRes val named: Int? = null)

/** The row's word for the choice. */
@Composable
private fun CodeChoice.label(): String = named?.let { stringResource(it) } ?: name.orEmpty()

/** Where Lounge lives, the two pages of it people ask for most, and where its maker posts. */
internal val AboutLinks = listOf(
    CodeChoice(null, "https://github.com/Gingerbreadfork/lounge", "github.com/Gingerbreadfork/lounge", named = R.string.about_link_source),
    CodeChoice(null, "https://github.com/Gingerbreadfork/lounge/issues", "github.com/Gingerbreadfork/lounge/issues", named = R.string.about_link_report),
    CodeChoice(null, "https://github.com/Gingerbreadfork/lounge/releases", "github.com/Gingerbreadfork/lounge/releases", named = R.string.about_link_whats_new),
    CodeChoice(null, "https://x.com/gingerbreadfork", "x.com/gingerbreadfork", named = R.string.about_link_maker_on_x),
)

/** Something to count on the About page: how many, and the plural that says what they are called. */
internal data class AboutFact(val count: Int, @PluralsRes val label: Int)

/** The choice [by] places on from the one at [index] of [count], round the end and back. */
internal fun choiceAfter(index: Int, by: Int, count: Int): Int = ((index + by) % count + count) % count

/** How far something has arrived, from 0 to 1, [beat] beats after the page's lamp starts to come up. */
@Composable
internal fun arriving(up: Boolean, beat: Int): State<Float> {
    val motion = LoungeTheme.motion
    return animateFloatAsState(if (up) 1f else 0f, motion.timed(motion.curtainMs, 90 * beat), label = "arriving")
}

/** Fades in and rises 10 px as [level] goes from 0 to 1. */
@Composable
internal fun Modifier.arrives(level: State<Float>): Modifier {
    val rise = with(LocalDensity.current) { 10.dp.toPx() }
    return graphicsLayer {
        alpha = level.value
        translationY = (1f - level.value) * rise
    }
}

/** A code's light and dark, the same in every theme so any phone reads it. */
private val QrLight = Color(0xFFF6EFE7)
private val QrDark = Color(0xFF1A1016)

/**
 * The About page: the mark under its lamp with what Lounge is and its version, a code that leads
 * to the repository, and who made it (DESIGN.md §5.5). Left and Right change where the code
 * leads, and Back returns to Settings.
 */
@Composable
internal fun AboutScreen(
    /** Works out the code for an address, or nothing when it cannot. Called off the main thread. */
    qr: (String) -> QrCode?,
    version: String,
    /** What this build holds, counted now so the page is never out of date. Called off the main thread. */
    facts: () -> List<AboutFact>,
    visibleState: MutableTransitionState<Boolean>,
    /** Whether the page is being come back to from the page for giving, whose row then takes focus. */
    fromDonate: Boolean,
    onDonate: () -> Unit,
    onClose: () -> Unit,
) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    val type = LoungeTheme.typography
    val glowAlpha = LoungeTheme.visuals.glowAlpha
    val codeFocus = remember { FocusRequester() }
    val giveFocus = remember { FocusRequester() }
    val counted by produceState(emptyList<AboutFact>()) { value = withContext(Dispatchers.IO) { facts() }.filter { it.count > 0 } }

    // The lamp comes up first and the rest follows it in, a beat apart.
    var up by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) { up = true }
    val lamp = animateFloatAsState(if (up) 1f else 0f, motion.lamp(), label = "aboutLamp")

    val resources = LocalResources.current
    CodePage(stringResource(R.string.about_title), visibleState, onClose) {
        Column(Modifier.width(940.dp).fillMaxHeight(), verticalArrangement = Arrangement.SpaceBetween) {
            Column {
                Spacer(Modifier.height(150.dp))
                Lockup(lamp)
                Spacer(Modifier.height(56.dp))
                Column(Modifier.width(760.dp).arrives(arriving(up, 1)), verticalArrangement = Arrangement.spacedBy(14.dp)) {
                    Text(stringResource(R.string.about_what), style = type.headline, color = colors.limelight)
                    Text(
                        stringResource(R.string.about_terms),
                        style = type.body,
                        color = colors.dust,
                    )
                    Text(aboutVersion(resources, version), style = type.meta, color = colors.dust)
                }
                Spacer(Modifier.height(56.dp))
                Row(Modifier.arrives(arriving(up, 2)), horizontalArrangement = Arrangement.spacedBy(64.dp)) {
                    counted.forEach { fact ->
                        Column(Modifier.semantics(mergeDescendants = true) {}, verticalArrangement = Arrangement.spacedBy(2.dp)) {
                            Text(fact.count.toString(), style = type.displayL, color = colors.limelight)
                            Text(pluralStringResource(fact.label, fact.count), style = type.caption, color = colors.dust)
                        }
                    }
                }
            }
            MakerLockup(stringResource(R.string.about_made_by), Modifier.arrives(arriving(up, 4)))
        }
        Box(Modifier.fillMaxSize()) {
            Text(
                stringResource(R.string.about_set_in),
                style = type.caption,
                color = colors.dust,
                modifier = Modifier.align(Alignment.BottomEnd).padding(bottom = 6.dp).arrives(arriving(up, 4)),
            )
            CodeColumn(
                qr = qr,
                choices = AboutLinks,
                above = stringResource(R.string.about_scan),
                rowTitle = stringResource(R.string.about_leads_to),
                describe = { stringResource(R.string.about_code_description, it.shown) },
                glow = { glowAlpha * 0.6f * lamp.value },
                focus = codeFocus,
                modifier = Modifier.align(Alignment.Center).arrives(arriving(up, 3)),
            ) {
                Spacer(Modifier.height(14.dp))
                SheetRow(
                    icon = SettingsGlyphs.Heart,
                    title = stringResource(R.string.about_support),
                    value = stringResource(R.string.about_support_value),
                    focusRequester = giveFocus,
                    modifier = Modifier.requiredWidth(CodeRowWidth),
                    onClick = onDonate,
                )
            }
        }
    }
    LaunchedEffect(Unit) { (if (fromDonate) giveFocus else codeFocus).requestWhenReady() }
}

/** A full screen on `velvet` that Back closes, with its two columns inside the safe area. */
@Composable
internal fun CodePage(title: String, visibleState: MutableTransitionState<Boolean>, onClose: () -> Unit, content: @Composable RowScope.() -> Unit) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    val leaving = !visibleState.targetState
    AnimatedVisibility(visibleState, enter = fadeIn(motion.curtain()), exit = fadeOut(motion.settle())) {
        Box(
            Modifier
                .fillMaxSize()
                .background(colors.velvet)
                .semantics { paneTitle = title }
                .onPreviewKeyEvent { e ->
                    // Both halves of Back are taken: left alone, the press would clear focus.
                    if (!e.isBack()) return@onPreviewKeyEvent false
                    if (e.type == KeyEventType.KeyUp) onClose()
                    true
                }
                .focusProperties { canFocus = !leaving }
                .trapFocus(leaving)
                .focusGroup(),
        ) {
            Row(Modifier.fillMaxSize().padding(horizontal = Dimens.safeX, vertical = Dimens.safeY), content = content)
        }
    }
}

/** The top bar's lockup at three times its size, the sofa in a lamp's light that follows [lamp]. */
@Composable
internal fun Lockup(lamp: State<Float>) {
    val colors = LoungeTheme.colors
    Row(
        Modifier.drawBehind {
            val sofa = Offset(84.dp.toPx(), size.height * 0.5f)
            drawSpill(SpillMask.soft, sofa, 640.dp.toPx(), 460.dp.toPx(), colors.houselight, 0.42f * colors.spillScale * lamp.value)
        },
    ) {
        Image(
            painterResource(tv.lounge.design.R.drawable.mark_sofa_large),
            contentDescription = null,
            modifier = Modifier.width(168.dp).alignBy { (it.measuredHeight * SEAT).roundToInt() },
        )
        Spacer(Modifier.width(42.dp))
        Text(
            "Lounge",
            style = LoungeTheme.typography.displayXl.copy(fontSize = 90.sp, lineHeight = 90.sp, letterSpacing = (-0.01).em),
            color = colors.limelight,
            modifier = Modifier.alignByBaseline().semantics { heading() },
        )
    }
}

/** The maker's mark with the maker's name beside it, under a line that says [how] they come into it. */
@Composable
internal fun MakerLockup(how: String, modifier: Modifier = Modifier) {
    val colors = LoungeTheme.colors
    Row(modifier.semantics(mergeDescendants = true) {}, verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(20.dp)) {
        // Drawn at its own size, a pixel for a pixel, so nothing softens it.
        Image(
            ImageBitmap.imageResource(tv.lounge.design.R.drawable.maker_gingerbreadfork),
            contentDescription = null,
            colorFilter = ColorFilter.tint(colors.limelight),
            filterQuality = FilterQuality.None,
            modifier = Modifier.size(64.dp),
        )
        Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(how, style = LoungeTheme.typography.caption, color = colors.dust)
            Text("Gingerbreadfork", style = LoungeTheme.typography.headline, color = colors.limelight)
        }
    }
}

/** How wide the row under a code is: wider than the code, so the longest name sits on one line. */
internal val CodeRowWidth = 480.dp

/**
 * A code with the row that chooses what it holds: a line [above], the code on its own light,
 * the row, what the code says written out, and a line of care under that where there is one.
 * Left, Right and OK on the row change the choice, and the code swaps with it.
 */
@Composable
internal fun CodeColumn(
    qr: (String) -> QrCode?,
    choices: List<CodeChoice>,
    above: String,
    rowTitle: String,
    /** What a screen reader says for the code. */
    describe: @Composable (CodeChoice) -> String,
    glow: () -> Float,
    focus: FocusRequester,
    modifier: Modifier = Modifier,
    below: @Composable ColumnScope.() -> Unit = {},
) {
    val colors = LoungeTheme.colors
    val type = LoungeTheme.typography
    var chosen by remember { mutableIntStateOf(0) }
    val choice = choices[chosen]
    val description = describe(choice)
    // Worked out off the main thread once, since each is a few thousand modules.
    val codes by produceState(emptyMap<String, QrCode>()) {
        value = withContext(Dispatchers.IO) { choices.mapNotNull { c -> qr(c.code)?.let { c.code to it } }.toMap() }
    }
    Column(modifier.width(400.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        Text(above, style = type.meta, color = colors.dust)
        Spacer(Modifier.height(20.dp))
        Box(
            Modifier
                .size(400.dp)
                .lightGlow(colors.houselight, glow, cornerRadius = 28.dp)
                .clip(RoundedCornerShape(28.dp))
                .background(QrLight)
                .semantics { contentDescription = description },
            contentAlignment = Alignment.Center,
        ) {
            Swap(choice.code, rise = 0.dp, durationMs = 180) { code ->
                codes[code]?.let { QrPicture(it, 344.dp) }
            }
        }
        Spacer(Modifier.height(20.dp))
        SheetRow(
            icon = null,
            title = rowTitle,
            value = choice.label(),
            focusRequester = focus,
            modifier = Modifier.requiredWidth(CodeRowWidth),
            onCycle = { by -> chosen = choiceAfter(chosen, by, choices.size) },
            onClick = { chosen = choiceAfter(chosen, 1, choices.size) },
        )
        Spacer(Modifier.height(10.dp))
        Swap(choice, rise = 0.dp, durationMs = 180) { shown ->
            Column(Modifier.requiredWidth(CodeRowWidth), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(shown.shown, style = type.meta, color = colors.limelight, textAlign = TextAlign.Center, maxLines = 2)
                shown.note?.let { Text(stringResource(it, shown.name.orEmpty()), style = type.caption, color = colors.houselight, textAlign = TextAlign.Center) }
            }
        }
        below()
    }
}

/** "Version 0.9.0", or a line that owns up to not knowing. */
internal fun aboutVersion(resources: Resources, version: String): String =
    if (version.isBlank()) resources.getString(R.string.about_version_unknown) else resources.getString(R.string.about_version, version)

/** The side of one module in whole pixels, so no seam shows between neighbours, and where the code starts so it sits centred. */
internal fun qrLayout(sidePx: Float, modules: Int): Pair<Float, Float> {
    val module = (sidePx / modules).toInt().coerceAtLeast(1).toFloat()
    return module to ((sidePx - module * modules) / 2f).toInt().toFloat()
}

/** Whether the module at [x], [y] belongs to one of the three eyes, which are drawn whole. */
internal fun inQrEye(x: Int, y: Int, modules: Int): Boolean = (x < 7 && y < 7) || (x >= modules - 7 && y < 7) || (x < 7 && y >= modules - 7)

/** A QR code drawn from its modules: square cells, and the three eyes with their corners eased. */
@Composable
private fun QrPicture(code: QrCode, side: Dp) {
    val n = code.size.toInt()
    Canvas(Modifier.size(side)) {
        val (module, start) = qrLayout(size.minDimension, n)
        for (y in 0 until n) {
            for (x in 0 until n) {
                if (code.dark[y * n + x] && !inQrEye(x, y, n)) {
                    drawRect(QrDark, Offset(start + x * module, start + y * module), Size(module, module))
                }
            }
        }
        for ((ex, ey) in listOf(0 to 0, n - 7 to 0, 0 to n - 7)) drawEye(Offset(start + ex * module, start + ey * module), module)
    }
}

/**
 * One eye: a ring seven modules across, one thick, round a block of three. Its corners are
 * eased by half a module and no more, since readers find a code by its eyes and a rounder one
 * is missed by the plainer of them.
 */
private fun DrawScope.drawEye(at: Offset, module: Float) {
    drawRoundRect(QrDark, at, Size(module * 7f, module * 7f), CornerRadius(module * 0.5f))
    drawRect(QrLight, at + Offset(module, module), Size(module * 5f, module * 5f))
    drawRoundRect(QrDark, at + Offset(module * 2f, module * 2f), Size(module * 3f, module * 3f), CornerRadius(module * 0.25f))
}
