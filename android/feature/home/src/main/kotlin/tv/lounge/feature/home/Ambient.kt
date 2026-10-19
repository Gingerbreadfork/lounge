package tv.lounge.feature.home

import android.content.Intent
import androidx.compose.animation.Crossfade
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.foundation.focusable
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.tv.material3.Text
import coil3.compose.AsyncImage
import java.util.Date
import kotlin.random.Random
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import tv.lounge.core.CoreClient
import tv.lounge.core.HomeSnapshot
import tv.lounge.core.TileKind
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme
import tv.lounge.design.LoungeVisuals
import tv.lounge.design.lightsFilter
import tv.lounge.platform.Prefs
import tv.lounge.power.PowerSetup
import tv.lounge.power.PowerStatus
import tv.lounge.power.PrivilegedException

private const val SLIDE_MS = 45_000L
private const val DRIFT_MS = 60_000L
private const val LATE_AFTER_MS = 30 * 60_000L
private const val LATE_DIM_MS = 60_000L
private const val LATE_DIM_STEPS = 20

/** One picture for the screensaver and the two lines under the clock that say what it is. */
data class AmbientSlide(val art: String, val title: String, val category: String)

/** The Stage's artwork first, then landscape tiles, without repeats: what the home screen already knows. */
fun ambientSlides(snapshot: HomeSnapshot): List<AmbientSlide> {
    val out = LinkedHashMap<String, AmbientSlide>()
    for (w in snapshot.wallpapers) out.putIfAbsent(w.artworkPath, AmbientSlide(w.artworkPath, w.title, w.source))
    if (out.size >= 12) return out.values.toList()
    for (s in snapshot.stage.spotlights) {
        val art = s.artworkPath ?: continue
        out.putIfAbsent(art, AmbientSlide(art, s.title, s.category))
    }
    for (row in snapshot.rows) {
        if (row.kind != TileKind.LANDSCAPE && row.kind != TileKind.POSTER) continue
        for (t in row.tiles) {
            val art = t.artworkPath ?: continue
            if (out.size >= 24) break
            out.putIfAbsent(art, AmbientSlide(art, t.title, row.title))
        }
    }
    return out.values.toList()
}

/**
 * The screensaver: full-bleed artwork dimmed with the house lights, a slow stepped push-in, the
 * clock bottom-left with two lines naming the picture. Everything drifts two pixels a minute, and
 * after half an hour the lights go down one more step (DESIGN.md §5).
 */
@Composable
fun AmbientScreen(core: CoreClient) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    val visuals = LoungeTheme.visuals
    val context = LocalContext.current
    val snapshot by core.snapshots.collectAsStateWithLifecycle()
    val slides = remember(snapshot) { ambientSlides(snapshot) }
    var index by remember { mutableIntStateOf(0) }
    var drift by remember { mutableStateOf(IntOffset.Zero) }
    var brightness by remember { mutableFloatStateOf(0.6f) }
    var clock by remember { mutableStateOf(android.text.format.DateFormat.getTimeFormat(context).format(Date())) }
    val startedAt = remember { System.currentTimeMillis() }
    val filter = remember(brightness) { lightsFilter(brightness, 0.95f) }

    LaunchedEffect(slides.size) {
        if (slides.size < 2) return@LaunchedEffect
        while (true) {
            delay(SLIDE_MS)
            index = (index + 1) % slides.size
        }
    }
    LaunchedEffect(Unit) {
        while (true) {
            delay(DRIFT_MS)
            drift = IntOffset(Random.nextInt(-2, 3), Random.nextInt(-2, 3))
        }
    }
    LaunchedEffect(Unit) {
        delay(LATE_AFTER_MS - (System.currentTimeMillis() - startedAt).coerceAtLeast(0))
        repeat(LATE_DIM_STEPS) { i ->
            brightness = 0.6f - 0.12f * (i + 1) / LATE_DIM_STEPS
            delay(LATE_DIM_MS / LATE_DIM_STEPS)
        }
    }
    LaunchedEffect(Unit) {
        while (true) {
            clock = android.text.format.DateFormat.getTimeFormat(context).format(Date())
            delay(1_000L * (61 - (System.currentTimeMillis() / 1_000 % 60)))
        }
    }

    val slide = slides.getOrNull(index % slides.size.coerceAtLeast(1))
    Box(Modifier.fillMaxSize().background(Color.Black)) {
        Box(Modifier.fillMaxSize().offset { drift }) {
            Crossfade(targetState = slide, animationSpec = tween(if (motion.reducedMotion) 0 else 2_400, easing = FastOutSlowInEasing), label = "ambient") { s ->
                if (s != null) {
                    // 6 % over the slide, in steps too small to see; nothing re-renders between steps.
                    val scale = remember(s.art) { mutableFloatStateOf(1f) }
                    val stepMs = if (visuals === LoungeVisuals.TierA) 250L else 500L
                    val origin = remember(s.art) { if (slides.indexOf(s) % 2 == 0) TransformOrigin(0.35f, 0.4f) else TransformOrigin(0.65f, 0.55f) }
                    LaunchedEffect(s.art) {
                        if (motion.reducedMotion) return@LaunchedEffect
                        val started = System.currentTimeMillis()
                        while (true) {
                            delay(stepMs)
                            val t = ((System.currentTimeMillis() - started).toFloat() / SLIDE_MS).coerceIn(0f, 1f)
                            scale.floatValue = 1f + 0.06f * t
                            if (t >= 1f) break
                        }
                    }
                    AsyncImage(
                        model = s.art,
                        contentDescription = null,
                        contentScale = ContentScale.Crop,
                        colorFilter = filter,
                        modifier = Modifier.fillMaxSize().graphicsLayer {
                            scaleX = scale.floatValue
                            scaleY = scale.floatValue
                            transformOrigin = origin
                        },
                    )
                } else {
                    Box(
                        Modifier.fillMaxSize().background(
                            Brush.radialGradient(listOf(colors.houselight.copy(alpha = 0.28f), colors.velvet), radius = 1400f),
                        ),
                    )
                }
            }
            Box(
                Modifier
                    .align(Alignment.BottomStart)
                    .fillMaxWidth()
                    .height(300.dp)
                    .background(Brush.verticalGradient(listOf(Color.Transparent, Color.Black.copy(alpha = 0.7f)))),
            )
            Column(Modifier.align(Alignment.BottomStart).padding(start = 96.dp, bottom = 72.dp)) {
                Text(clock, style = LoungeTheme.typography.clock, color = colors.limelight)
                if (slide != null) {
                    Spacer(Modifier.height(10.dp))
                    Text(slide.title, style = LoungeTheme.typography.meta, color = colors.limelight, maxLines = 1)
                    Text(slide.category, style = LoungeTheme.typography.caption, color = colors.dust, maxLines = 1)
                }
            }
        }
    }
}

/** The screensaver shown in-app for a look; the first key press closes it. */
@Composable
fun AmbientPreview(core: CoreClient, onClose: () -> Unit) {
    val focus = remember { FocusRequester() }
    Box(
        Modifier
            .fillMaxSize()
            .onPreviewKeyEvent { e -> if (e.type == KeyEventType.KeyUp) { onClose(); true } else true }
            .focusRequester(focus)
            .focusable(),
    ) {
        AmbientScreen(core)
    }
    LaunchedEffect(Unit) { focus.requestWhenReady() }
}

/** Settings → Screensaver: make Lounge the system screensaver (through Power Setup) or go back. */
@Composable
fun AmbientSheet(power: PowerSetup, prefs: Prefs, onPowerSetup: () -> Unit, onPreview: () -> Unit, firstFocus: FocusRequester) {
    val colors = LoungeTheme.colors
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val status by power.status.collectAsStateWithLifecycle()
    var current by remember { mutableStateOf(power.currentScreensaver) }
    var busy by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    val ready = status is PowerStatus.Ready
    val ours = current?.contains(context.packageName) == true

    LaunchedEffect(Unit) { power.refresh() }

    fun run(id: String, block: suspend () -> Unit) {
        if (busy != null) return
        busy = id
        error = null
        scope.launch {
            try {
                block()
            } catch (e: PrivilegedException) {
                error = e.message
            }
            current = power.currentScreensaver
            busy = null
        }
    }

    SheetHeading(
        "Screensaver",
        if (ours) "Lounge's artwork, a clock and what each picture is; it drifts and dims to be kind to OLED panels."
        else "Show the home screen's artwork with a clock when the TV idles.",
    )
    Column(verticalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(8.dp)) {
        SheetRow(
            icon = LoungeIcons.Frame,
            title = "Screensaver",
            value = when {
                ours -> "Lounge"
                current.isNullOrBlank() -> "Off"
                else -> power.labelFor(current!!)
            },
            accent = if (ours) colors.signal else colors.dust,
            onClick = {},
            focusRequester = firstFocus,
        )
        if (!ours) {
            SheetRow(
                icon = LoungeIcons.Power,
                title = "Use Lounge as the screensaver",
                value = when {
                    busy == "use" -> "Switching…"
                    ready -> "Through Power Setup"
                    status is PowerStatus.Unavailable -> "Open settings"
                    else -> "Power Setup"
                },
                accent = colors.houselight,
                onClick = {
                    when {
                        ready -> run("use") {
                            prefs.setPreviousScreensaver(current)
                            power.run(power.useAmbient)
                        }
                        status is PowerStatus.Unavailable -> runCatching {
                            context.startActivity(Intent(android.provider.Settings.ACTION_DREAM_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                        }.onFailure { error = "This device has no screensaver settings screen; use Power Setup." }
                        else -> onPowerSetup()
                    }
                },
            )
        } else {
            SheetRow(
                icon = LoungeIcons.Refresh,
                title = "Back to the previous screensaver",
                value = if (busy == "restore") "Switching…" else if (ready) "" else "Power Setup",
                onClick = {
                    if (ready) run("restore") {
                        val previous = prefs.previousScreensaver.first()
                        power.run(power.restoreScreensaver(previous))
                    } else onPowerSetup()
                },
            )
        }
        SheetRow(icon = LoungeIcons.Play, title = "Preview it now", value = "Any key brings you back", onClick = onPreview)
        if (error != null) Text(error!!, style = LoungeTheme.typography.caption, color = colors.ember)
    }
    SheetFootnote(
        "Pictures come from what is already on your home screen; nothing is downloaded for this.",
        "The system decides when the screensaver starts: Settings → System → Screensaver on most devices.",
    )
}
