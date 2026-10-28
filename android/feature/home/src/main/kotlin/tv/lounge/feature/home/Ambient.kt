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
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.produceState
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
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.isUnspecified
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.drawscope.translate
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
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import tv.lounge.core.CoreClient
import tv.lounge.core.HomeSnapshot
import tv.lounge.core.TileKind
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme
import tv.lounge.design.lightsFilter
import tv.lounge.design.sounds
import tv.lounge.platform.PanelSize
import tv.lounge.platform.Prefs
import tv.lounge.power.PowerSetup
import tv.lounge.power.PowerStatus
import tv.lounge.power.PrivilegedException

internal const val SLIDE_MS = 45_000L
private const val DRIFT_MS = 60_000L
internal const val LATE_AFTER_MS = 30 * 60_000L
internal const val LATE_DIM_MS = 60_000L
internal const val LATE_DIM_STEPS = 20

/**
 * One picture for the screensaver and the two lines under the clock that say what it is. A
 * [framed] picture is shown whole in its own shape with the room around it, as a painting would be.
 */
data class AmbientSlide(val art: String, val title: String, val category: String, val framed: Boolean = false)

/** Plugin pictures first, then the Stage's artwork and landscape tiles, without repeats: what the home screen already knows. */
fun ambientSlides(snapshot: HomeSnapshot): List<AmbientSlide> {
    val out = LinkedHashMap<String, AmbientSlide>()
    for (w in snapshot.wallpapers) out.putIfAbsent(w.artworkPath, AmbientSlide(w.artworkPath, w.title, w.subtitle ?: w.source, w.framed))
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
    val drift = rememberAmbientDrift()
    var brightness by remember { mutableFloatStateOf(0.6f) }
    val clock = rememberAmbientClock()
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
        delay(LATE_AFTER_MS - (System.currentTimeMillis() - startedAt).coerceAtLeast(0))
        repeat(LATE_DIM_STEPS) { i ->
            brightness = 0.6f - 0.12f * (i + 1) / LATE_DIM_STEPS
            delay(LATE_DIM_MS / LATE_DIM_STEPS)
        }
    }

    val slide = slides.getOrNull(index % slides.size.coerceAtLeast(1))
    // A panel with more pixels than the TV draws its apps at gets the pictures on a surface of
    // its own size; the captions stay in Compose over it.
    val panel = remember { PanelSize.of(context) }
    val window = remember { PanelSize.window(context) }
    val onPanel = panel.height > window.height && panel.width > 0
    if (onPanel) {
        Box(Modifier.fillMaxSize()) {
            PanelPictures(
                slide = slide,
                slideIndex = index,
                brightness = brightness,
                drift = drift,
                panel = panel,
                window = window,
                stepMs = if (visuals.tierA) 250L else 500L,
                reducedMotion = motion.reducedMotion,
                modifier = Modifier.fillMaxSize(),
            )
            AmbientCaption(clock, slide?.title, slide?.category, drift)
        }
        return
    }
    Box(Modifier.fillMaxSize().background(Color.Black)) {
        Box(Modifier.fillMaxSize().offset { drift }) {
            Crossfade(targetState = slide, animationSpec = tween(if (motion.reducedMotion) 0 else 2_400, easing = FastOutSlowInEasing), label = "ambient") { s ->
                if (s != null) {
                    // 6 % over the slide, in steps too small to see; nothing re-renders between steps.
                    val scale = remember(s.art) { mutableFloatStateOf(1f) }
                    val stepMs = if (visuals.tierA) 250L else 500L
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
                    if (s.framed) {
                        // A painting hangs whole on the dark wall, with air around it and a shadow.
                        FramedPicture(
                            s.art,
                            filter,
                            Modifier
                                .fillMaxSize()
                                .padding(horizontal = 160.dp, vertical = 100.dp)
                                .graphicsLayer {
                                    scaleX = scale.floatValue
                                    scaleY = scale.floatValue
                                    transformOrigin = TransformOrigin.Center
                                },
                        )
                    } else {
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
                    }
                } else {
                    Box(
                        Modifier.fillMaxSize().background(
                            Brush.radialGradient(listOf(colors.houselight.copy(alpha = 0.28f), colors.velvet), radius = 1400f),
                        ),
                    )
                }
            }
            AmbientCaption(clock, slide?.title, slide?.category, IntOffset.Zero)
        }
    }
}

/**
 * A picture fitted whole inside its box and centred, with a soft shadow just outside its own
 * edges, so a portrait painting hangs as a painting and not as a cropped still.
 */
@Composable
private fun FramedPicture(path: String, filter: androidx.compose.ui.graphics.ColorFilter?, modifier: Modifier) {
    val context = LocalContext.current
    // The file is already prepared at the screen's size, so it is decoded as it is.
    val request = remember(path) { coil3.request.ImageRequest.Builder(context).data(path).size(coil3.size.Size.ORIGINAL).build() }
    val painter = coil3.compose.rememberAsyncImagePainter(model = request, contentScale = ContentScale.Fit)
    Box(
        modifier.drawBehind {
            val intrinsic = painter.intrinsicSize
            if (intrinsic.isUnspecified || intrinsic.width <= 0f || intrinsic.height <= 0f) {
                with(painter) { draw(this@drawBehind.size, colorFilter = filter) }
                return@drawBehind
            }
            val fit = minOf(size.width / intrinsic.width, size.height / intrinsic.height)
            val drawn = androidx.compose.ui.geometry.Size(intrinsic.width * fit, intrinsic.height * fit)
            val dx = (size.width - drawn.width) / 2f
            val dy = (size.height - drawn.height) / 2f
            // The shadow: a few rectangles stepping out and fading, cheaper than a blur and as soft from the sofa.
            val step = 5.dp.toPx()
            for (i in 1..8) {
                val a = 0.16f * (1f - (i - 1) / 8f)
                val grow = step * i
                drawRect(Color.Black.copy(alpha = a), topLeft = Offset(dx - grow, dy - grow + step * 2), size = androidx.compose.ui.geometry.Size(drawn.width + grow * 2, drawn.height + grow * 2))
            }
            translate(dx, dy) { with(painter) { draw(drawn, colorFilter = filter) } }
        },
    )
}

/** The time, updated on the minute. */
@Composable
internal fun rememberAmbientClock(): String {
    val context = LocalContext.current
    var clock by remember { mutableStateOf(android.text.format.DateFormat.getTimeFormat(context).format(Date())) }
    LaunchedEffect(Unit) {
        while (true) {
            clock = android.text.format.DateFormat.getTimeFormat(context).format(Date())
            delay(1_000L * (61 - (System.currentTimeMillis() / 1_000 % 60)))
        }
    }
    return clock
}

/** Two pixels of drift a minute, so nothing sits still on an OLED panel. */
@Composable
internal fun rememberAmbientDrift(): IntOffset {
    var drift by remember { mutableStateOf(IntOffset.Zero) }
    LaunchedEffect(Unit) {
        while (true) {
            delay(DRIFT_MS)
            drift = IntOffset(Random.nextInt(-2, 3), Random.nextInt(-2, 3))
        }
    }
    return drift
}

/** The clock bottom-left with two lines saying what is on screen, over a scrim that keeps it readable. */
@Composable
internal fun AmbientCaption(clock: String, title: String?, line: String?, drift: IntOffset) {
    val colors = LoungeTheme.colors
    Box(Modifier.fillMaxSize()) {
        Box(
            Modifier
                .align(Alignment.BottomStart)
                .fillMaxWidth()
                .height(300.dp)
                .background(Brush.verticalGradient(listOf(Color.Transparent, Color.Black.copy(alpha = 0.7f)))),
        )
        Column(Modifier.align(Alignment.BottomStart).offset { drift }.padding(start = 96.dp, bottom = 72.dp)) {
            Text(clock, style = LoungeTheme.typography.clock, color = colors.limelight)
            if (title != null) {
                Spacer(Modifier.height(10.dp))
                Text(title, style = LoungeTheme.typography.meta, color = colors.limelight, maxLines = 1)
                if (line != null) Text(line, style = LoungeTheme.typography.caption, color = colors.dust, maxLines = 1)
            }
        }
    }
}

/** The screensaver chosen in Settings: Lounge's pictures, or an abstract scene drawn on the GPU. */
@Composable
fun Screensaver(core: CoreClient, prefs: Prefs, preview: Boolean = false) {
    val context = LocalContext.current
    val chosen by prefs.ambientStyle.collectAsState(initial = null)
    val id = chosen
    if (id == null) {
        Box(Modifier.fillMaxSize().background(Color.Black))
        return
    }
    // Read off the main thread; the screen stays black until it is known, so pictures never flash
    // before a scene.
    val found by produceState<Pair<String, ShaderStyle?>?>(null, id) {
        value = id to if (id == ShaderStyles.PICTURES) null else withContext(Dispatchers.IO) { ShaderStyles.find(context, core, id) }
    }
    val loaded = found
    if (loaded == null || loaded.first != id) {
        Box(Modifier.fillMaxSize().background(Color.Black))
        return
    }
    val style = loaded.second
    var unusable by remember(id) { mutableStateOf(false) }
    if (style == null || unusable) {
        AmbientScreen(core)
    } else {
        ShaderScreensaver(
            style,
            onUnusable = { why ->
                android.util.Log.w("Lounge", "screensaver: ${style.id} falls back to pictures: $why")
                unusable = true
            },
            preview = preview,
        )
    }
}

/** The screensaver shown in-app for a look; the first key press closes it, except volume and mute. */
@Composable
fun AmbientPreview(core: CoreClient, prefs: Prefs, onClose: () -> Unit) {
    val focus = remember { FocusRequester() }
    Box(
        Modifier
            .fillMaxSize()
            .onPreviewKeyEvent { e ->
                when {
                    e.key == Key.VolumeUp || e.key == Key.VolumeDown || e.key == Key.VolumeMute -> false
                    e.type == KeyEventType.KeyUp -> { onClose(); true }
                    else -> true
                }
            }
            .focusRequester(focus)
            .focusable(),
    ) {
        Screensaver(core, prefs, preview = true)
    }
    LaunchedEffect(Unit) { focus.requestWhenReady() }
}

/** Settings → Screensaver: make Lounge the system screensaver (through Power Setup) or go back. */
@Composable
fun AmbientSheet(power: PowerSetup, prefs: Prefs, core: CoreClient, onPowerSetup: () -> Unit, onPreview: () -> Unit, firstFocus: FocusRequester) {
    val colors = LoungeTheme.colors
    val sounds = LoungeTheme.sounds
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val styles by produceState(emptyList<ShaderStyle>()) { value = withContext(Dispatchers.IO) { ShaderStyles.all(context, core) } }
    val chosen by prefs.ambientStyle.collectAsState(initial = ShaderStyles.PICTURES)
    val choices = listOf(Triple(ShaderStyles.PICTURES, "Pictures", "Artwork from your home screen")) +
        styles.map { Triple(it.id, it.name, it.description ?: it.origin) }
    val at = choices.indexOfFirst { it.first == chosen }.coerceAtLeast(0)
    fun step(by: Int) {
        sounds?.tick()
        val next = choices[(at + by).mod(choices.size)].first
        scope.launch { prefs.setAmbientStyle(next) }
    }
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
        if (ours) "Your artwork or a slow abstract scene, with a clock; it drifts and dims to be kind to OLED panels."
        else "Show your artwork or a slow abstract scene with a clock when the TV idles.",
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
        SheetRow(
            icon = LoungeIcons.Theme,
            title = "Style",
            value = choices[at].second,
            subtitle = choices[at].third,
            onClick = { step(1) },
            onCycle = { step(it) },
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
                            val r = power.run(power.useAmbient)
                            prefs.setPreviousScreensaverEnabled(PowerSetup.previousScreensaverEnabled(r.output))
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
                        power.run(power.restoreScreensaver(previous, prefs.previousScreensaverEnabled.first()))
                    } else onPowerSetup()
                },
            )
        }
        SheetRow(icon = LoungeIcons.Play, title = "Preview it now", value = "Any key brings you back", onClick = onPreview)
        if (error != null) Text(error!!, style = LoungeTheme.typography.caption, color = colors.ember)
    }
    SheetFootnote(
        "Pictures come from what is already on your home screen and abstract scenes are drawn on the TV; nothing is downloaded for this.",
        "The system decides when the screensaver starts: Settings → System → Screensaver on most devices.",
    )
}
