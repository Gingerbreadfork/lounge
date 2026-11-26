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
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.runtime.State
import androidx.compose.runtime.derivedStateOf
import androidx.compose.ui.text.style.TextOverflow
import androidx.tv.material3.Icon
import tv.lounge.core.NowPlaying
import tv.lounge.core.PlaybackStatus
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
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.tv.material3.Text
import coil3.compose.AsyncImage
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
import tv.lounge.design.R as DesignR
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
/** The house lights late at night, against their level by day. */
private const val NIGHT_LIGHTS = 0.8f
/** How long the mix stays on one scene. */
internal const val MIX_SCENE_MS = 4 * 60_000L
/** Minutes the clock spends on one side of the screen before it crosses to the other. */
internal const val SIDE_MINUTES = 10
private const val LAST_PICTURE = "pictures:last"

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

/** The picture after the one at [after], round the list; the first when that one is not in it. */
internal fun nextSlide(slides: List<AmbientSlide>, after: String?): AmbientSlide? =
    if (slides.isEmpty()) null else slides[(slides.indexOfFirst { it.art == after } + 1) % slides.size]

/**
 * The house lights at [hour] of the day: full by day, lower from eleven at night until six, and
 * easing between the two over the hour either side.
 */
internal fun nightLights(hour: Float): Float {
    val h = ((hour % 24f) + 24f) % 24f
    val night = when {
        h >= 22f -> (h - 22f).coerceAtMost(1f)
        h < 7f -> 1f - (h - 6f).coerceIn(0f, 1f)
        else -> 0f
    }
    return 1f - (1f - NIGHT_LIGHTS) * night
}

/**
 * How far the clock has moved in from its corner after [minutes], across and up: two slow
 * triangle waves, so it passes over its whole box evenly and never rests (OLED care).
 */
internal fun captionDrift(minutes: Int): IntOffset {
    fun wave(period: Int, reach: Int): Int {
        val phase = (minutes % period).toFloat() / period
        return ((1f - kotlin.math.abs(2f * phase - 1f)) * reach).toInt()
    }
    return IntOffset(wave(48, 48), wave(34, 24))
}

/** Where the clock and its lines sit: which side, how far in from the corner, and how far they have faded for a crossing. */
@androidx.compose.runtime.Stable
internal class CaptionWander {
    var right by mutableStateOf(false)
    var drift by mutableStateOf(IntOffset.Zero)
    val alpha = androidx.compose.animation.core.Animatable(1f)
}

/** The clock's place, moved a little each minute and across the screen every [SIDE_MINUTES]. */
@Composable
internal fun rememberCaptionWander(): CaptionWander {
    val reduced = LoungeTheme.motion.reducedMotion
    val wander = remember { CaptionWander() }
    LaunchedEffect(wander, reduced) {
        var minutes = 0
        while (true) {
            delay(DRIFT_MS)
            minutes++
            if (minutes % SIDE_MINUTES == 0) {
                if (!reduced) wander.alpha.animateTo(0f, tween(900))
                wander.right = !wander.right
                wander.drift = captionDrift(minutes)
                if (!reduced) wander.alpha.animateTo(1f, tween(900))
            } else {
                wander.drift = captionDrift(minutes)
            }
        }
    }
    return wander
}

/**
 * The house lights over the screensaver, as a share of their full level: lower late at night, and
 * a step lower again after half an hour (DESIGN.md §5).
 */
@Composable
internal fun rememberHouseLights(): State<Float> {
    val late = remember { mutableFloatStateOf(1f) }
    val night = remember { mutableFloatStateOf(nightLights(localHour())) }
    LaunchedEffect(Unit) {
        delay(LATE_AFTER_MS)
        repeat(LATE_DIM_STEPS) { i ->
            late.floatValue = 1f - 0.2f * (i + 1) / LATE_DIM_STEPS
            delay(LATE_DIM_MS / LATE_DIM_STEPS)
        }
    }
    LaunchedEffect(Unit) {
        while (true) {
            delay(60_000L)
            night.floatValue = nightLights(localHour())
        }
    }
    return remember { derivedStateOf { late.floatValue * night.floatValue } }
}

/** What both kinds of screensaver share: the house lights, the clock and its place, and what is playing. */
@androidx.compose.runtime.Stable
internal class AmbientRoom(val lights: State<Float>, val wander: CaptionWander, val clock: State<String>, val playing: State<String?>)

@Composable
internal fun rememberAmbientRoom(core: CoreClient): AmbientRoom {
    val lights = rememberHouseLights()
    val wander = rememberCaptionWander()
    val clock = rememberAmbientClock()
    val snapshot = core.snapshots.collectAsStateWithLifecycle()
    val playing = remember { derivedStateOf { playingLine(snapshot.value.nowPlaying) } }
    return remember(lights, wander, clock, playing) { AmbientRoom(lights, wander, clock, playing) }
}

/** What is playing in one line, or null while nothing is. */
internal fun playingLine(np: NowPlaying?): String? {
    if (np == null || np.status != PlaybackStatus.PLAYING) return null
    return listOfNotNull(np.title.trim().ifEmpty { null }, np.subtitle?.trim()?.ifEmpty { null }).joinToString(" · ").ifEmpty { null }
}

/**
 * The screensaver's pictures: full-bleed artwork dimmed with the house lights, a slow stepped
 * push-in, the clock with two lines naming the picture. It carries on from the picture after the
 * last one shown, so every picture gets its turn across starts; a [preview] does not move that on.
 */
@Composable
internal fun AmbientScreen(core: CoreClient, room: AmbientRoom, preview: Boolean = false) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    val visuals = LoungeTheme.visuals
    val context = LocalContext.current
    val snapshot by core.snapshots.collectAsStateWithLifecycle()
    val slides = remember(snapshot) { ambientSlides(snapshot) }
    val memory = remember { context.getSharedPreferences(SceneMemory.FILE, android.content.Context.MODE_PRIVATE) }
    var slide by remember { mutableStateOf<AmbientSlide?>(null) }
    var count by remember { mutableIntStateOf(0) }
    val drift = rememberAmbientDrift()
    val brightness = 0.6f * room.lights.value
    val filter = remember(brightness) { lightsFilter(brightness, 0.95f) }
    val latest = androidx.compose.runtime.rememberUpdatedState(slides)

    fun show(next: AmbientSlide?) {
        if (next == null || next.art == slide?.art) return
        slide = next
        count++
        if (!preview) memory.edit().putString(LAST_PICTURE, next.art).apply()
    }
    LaunchedEffect(slides.isEmpty()) {
        if (slides.isEmpty()) return@LaunchedEffect
        if (slide == null) show(nextSlide(slides, withContext(Dispatchers.IO) { memory.getString(LAST_PICTURE, null) }))
        while (true) {
            delay(SLIDE_MS)
            show(nextSlide(latest.value, slide?.art))
        }
    }

    val stepMs = if (visuals.tierA) 250L else 500L
    // A panel with more pixels than the TV draws its apps at gets the pictures on a surface of
    // its own size; the captions stay in Compose over it.
    val panel = remember { PanelSize.of(context) }
    val window = remember { PanelSize.window(context) }
    val onPanel = panel.height > window.height && panel.width > 0
    if (onPanel) {
        Box(Modifier.fillMaxSize()) {
            PanelPictures(
                slide = slide,
                slideIndex = count,
                brightness = brightness,
                drift = drift,
                panel = panel,
                window = window,
                stepMs = stepMs,
                reducedMotion = motion.reducedMotion,
                modifier = Modifier.fillMaxSize(),
            )
            AmbientCaption(room, slide?.title, slide?.category)
        }
        return
    }
    Box(Modifier.fillMaxSize().background(Color.Black)) {
        Box(Modifier.fillMaxSize().offset { drift }) {
            Crossfade(targetState = slide, animationSpec = tween(if (motion.reducedMotion) 0 else 2_400, easing = FastOutSlowInEasing), label = "ambient") { s ->
                if (s != null) {
                    // 6 % over the slide, in steps too small to see; nothing re-renders between steps.
                    val scale = remember(s.art) { mutableFloatStateOf(1f) }
                    val origin = remember(s.art) { if (count % 2 == 0) TransformOrigin(0.35f, 0.4f) else TransformOrigin(0.65f, 0.55f) }
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
                            Brush.radialGradient(listOf(colors.houselight.copy(alpha = 0.28f * room.lights.value), colors.velvet), radius = 1400f),
                        ),
                    )
                }
            }
        }
        AmbientCaption(room, slide?.title, slide?.category)
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
internal fun rememberAmbientClock(): State<String> {
    val context = LocalContext.current
    val clock = remember { mutableStateOf(clockText(context)) }
    LaunchedEffect(Unit) {
        while (true) {
            clock.value = clockText(context)
            delay(1_000L * (61 - (System.currentTimeMillis() / 1_000 % 60)))
        }
    }
    return clock
}

/** The time as the top bar writes it, following the system's 24-hour setting. */
private fun clockText(context: android.content.Context): String {
    val pattern = if (android.text.format.DateFormat.is24HourFormat(context)) "HH:mm" else "h:mm"
    return java.time.LocalTime.now().format(java.time.format.DateTimeFormatter.ofPattern(pattern, java.util.Locale.getDefault()))
}

/** Two pixels of drift a minute for the pictures, so a frame's edge never sits still on an OLED panel. */
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

/**
 * The clock with two lines saying what is on screen and a third for what is playing, over a
 * scrim deep enough that the `dust` lines stay readable over the brightest picture or scene. It
 * sits bottom-left to begin with, wanders as the [room] says and dims with its house lights.
 */
@Composable
internal fun AmbientCaption(room: AmbientRoom, title: String?, line: String?) {
    val colors = LoungeTheme.colors
    val wander = room.wander
    val right = wander.right
    val playing = room.playing.value
    Box(Modifier.fillMaxSize()) {
        Box(
            Modifier
                .align(Alignment.BottomStart)
                .fillMaxWidth()
                .height(300.dp)
                .background(Brush.verticalGradient(0f to Color.Transparent, 0.35f to Color.Black.copy(alpha = 0.6f), 1f to Color.Black.copy(alpha = 0.92f))),
        )
        Column(
            Modifier
                .align(if (right) Alignment.BottomEnd else Alignment.BottomStart)
                .offset { IntOffset(if (right) -wander.drift.x else wander.drift.x, -wander.drift.y) }
                .graphicsLayer { alpha = wander.alpha.value * room.lights.value }
                .padding(start = 96.dp, end = 96.dp, bottom = 72.dp),
            horizontalAlignment = if (right) Alignment.End else Alignment.Start,
        ) {
            Text(room.clock.value, style = LoungeTheme.typography.clock, color = colors.limelight)
            if (title != null) {
                Spacer(Modifier.height(10.dp))
                Text(title, style = LoungeTheme.typography.meta, color = colors.limelight, maxLines = 1)
                if (line != null) Text(line, style = LoungeTheme.typography.caption, color = colors.dust, maxLines = 1)
            }
            if (playing != null) {
                Spacer(Modifier.height(10.dp))
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(8.dp)) {
                    Icon(LoungeIcons.Equaliser, contentDescription = stringResource(R.string.ambient_playing_description), tint = colors.dust, modifier = Modifier.size(16.dp))
                    Text(playing, style = LoungeTheme.typography.caption, color = colors.dust, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.widthIn(max = 760.dp))
                }
            }
        }
    }
}

/**
 * The screensaver chosen in Settings: Lounge's pictures, an abstract scene drawn on the GPU, or
 * every scene in shuffled order. A [preview] shows what the next start would without counting as one.
 */
@Composable
fun Screensaver(core: CoreClient, prefs: Prefs, preview: Boolean = false) {
    val context = LocalContext.current
    val chosen by prefs.ambientStyle.collectAsState(initial = null)
    val id = chosen
    val room = rememberAmbientRoom(core)
    if (id == null) {
        Box(Modifier.fillMaxSize().background(Color.Black))
        return
    }
    // The mix takes a turn each time it moves on, to the next scene or past one that cannot be drawn.
    var turn by remember(id) { mutableIntStateOf(0) }
    // Read off the main thread; the screen stays black until it is known, so pictures never flash
    // before a scene.
    val found by produceState<Triple<String, ShaderStyle?, Int>?>(null, id, turn) {
        val onScreen = value?.second?.id
        val style = withContext(Dispatchers.IO) {
            when (id) {
                ShaderStyles.PICTURES -> null
                ShaderStyles.MIX -> ShaderStyles.nextInMix(context, core, advance = !preview, after = onScreen)
                else -> ShaderStyles.find(context, core, id)
            }
        }
        value = Triple(id, style, turn)
    }
    val loaded = found
    if (loaded == null || loaded.first != id) {
        Box(Modifier.fillMaxSize().background(Color.Black))
        return
    }
    val style = loaded.second
    var unusable by remember(id) { mutableStateOf(false) }
    if (style == null || unusable) {
        AmbientScreen(core, room, preview)
        return
    }
    // A scene in the mix stays a few minutes, then goes down into black for the next.
    var leavingTurn by remember(id) { mutableIntStateOf(-1) }
    if (id == ShaderStyles.MIX) {
        LaunchedEffect(loaded.third) {
            delay(MIX_SCENE_MS)
            leavingTurn = loaded.third
            delay(SCENE_FADE_MS + 100L)
            turn++
        }
    }
    ShaderScreensaver(
        style,
        room,
        onUnusable = { why ->
            android.util.Log.w("Lounge", "screensaver: ${style.id} gives way: $why")
            // A compile error's own text is for the log, not the screen (DESIGN.md §8).
            SceneMemory.seen[style.id] = if ("heavy" in why) SceneRecord.HEAVY else SceneRecord.BROKEN
            if (id == ShaderStyles.MIX) turn++ else unusable = true
        },
        preview = preview,
        leaving = leavingTurn == loaded.third,
    )
}

/** Closes the full preview on the first key press, except volume and mute, which pass through. */
@Composable
fun AmbientPreviewKeys(onClose: () -> Unit) {
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
    )
    LaunchedEffect(Unit) { focus.requestWhenReady() }
}

/** The screensaver the TV starts: none, Lounge's, its own, or another app's by that app's name. */
sealed interface TvScreensaver {
    data object Off : TvScreensaver
    data object TvOwn : TvScreensaver
    data object Lounge : TvScreensaver
    data class App(val label: String) : TvScreensaver
}

/** The name Settings and the Screensaver sheet both give it. */
@Composable
internal fun TvScreensaver.words(): String = when (this) {
    TvScreensaver.Off -> stringResource(DesignR.string.common_off)
    TvScreensaver.TvOwn -> stringResource(R.string.ambient_tv_own)
    TvScreensaver.Lounge -> "Lounge"
    is TvScreensaver.App -> label
}

/** The screensaver the TV starts now, as Settings and the Screensaver sheet both show it. */
internal fun screensaverName(power: PowerSetup, context: android.content.Context): TvScreensaver {
    val enabled = runCatching { android.provider.Settings.Secure.getInt(context.contentResolver, "screensaver_enabled", 1) }.getOrDefault(1)
    val current = power.currentScreensaver
    return when {
        enabled == 0 -> TvScreensaver.Off
        current == null -> TvScreensaver.TvOwn
        current.contains(context.packageName) -> TvScreensaver.Lounge
        // A label that is only a package name says nothing to a person.
        else -> power.labelFor(current).takeUnless { it.contains('.') && ' ' !in it }?.let { TvScreensaver.App(it) } ?: TvScreensaver.TvOwn
    }
}

/** The styles the Screensaver sheet offers, in order: each one's id, and the scene where it is one. */
internal fun styleChoices(scenes: List<ShaderStyle>): List<Pair<String, ShaderStyle?>> = buildList {
    add(ShaderStyles.PICTURES to null)
    if (scenes.size > 1) add(ShaderStyles.MIX to null)
    scenes.forEach { add(it.id to it) }
}

/**
 * The Screensaver sheet: which screensaver the TV starts, which OK switches between Lounge's and
 * the one before through Power Setup, then the style, shown behind the sheet as it is chosen.
 * Back from the full preview lands on its row ([fromPreview]).
 */
@Composable
fun AmbientSheet(power: PowerSetup, prefs: Prefs, core: CoreClient, onPowerSetup: () -> Unit, onPreview: () -> Unit, firstFocus: FocusRequester, fromPreview: Boolean = false) {
    val colors = LoungeTheme.colors
    val sounds = LoungeTheme.sounds
    val context = LocalContext.current
    val resources = LocalResources.current
    val scope = rememberCoroutineScope()
    val styles by produceState(emptyList<ShaderStyle>()) { value = withContext(Dispatchers.IO) { ShaderStyles.all(context, core) } }
    val chosen by prefs.ambientStyle.collectAsState(initial = ShaderStyles.PICTURES)
    val choices = styleChoices(styles)
    val at = choices.indexOfFirst { it.first == chosen }.coerceAtLeast(0)
    val memory = remember { SceneMemory(context) }
    // Why the chosen scene is not what shows, as its record names it, read again when the preview behind finds a problem.
    val problem by produceState<String?>(null, chosen, styles, SceneMemory.seen.size) {
        val style = styles.firstOrNull { it.id == chosen }
        value = if (style == null) null else withContext(Dispatchers.IO) { memory.problem(style) }
    }
    fun step(by: Int) {
        sounds?.tick()
        val next = choices[(at + by).mod(choices.size)].first
        scope.launch { prefs.setAmbientStyle(next) }
    }
    val status by power.status.collectAsStateWithLifecycle()
    var name by remember { mutableStateOf(screensaverName(power, context)) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val ready = status is PowerStatus.Ready
    val ours = name == TvScreensaver.Lounge
    val notTaken = stringResource(R.string.ambient_not_taken)

    // Not every TV has a screen for choosing a screensaver, and some that do list only their own.
    val tvSettings = remember { Intent(android.provider.Settings.ACTION_DREAM_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK) }
    val tvHasSettings = remember { runCatching { context.packageManager.resolveActivity(tvSettings, 0) != null }.getOrDefault(false) }
    var sentToTv by remember { mutableStateOf(false) }
    var tvDidNotTake by remember { mutableStateOf(false) }
    val viaTv = !ours && status is PowerStatus.Unavailable && tvHasSettings && !tvDidNotTake

    LaunchedEffect(Unit) { power.refresh() }
    // How long the TV waits before any screensaver, read again on coming back from the TV's own settings.
    var after by remember { mutableStateOf(power.screensaverMinutes()) }
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) {
        name = screensaverName(power, context)
        after = power.screensaverMinutes()
        if (sentToTv && name != TvScreensaver.Lounge) tvDidNotTake = true
        sentToTv = false
    }

    /**
     * Runs a switch, then reads the TV's setting again; [block] returns what to say when it did not
     * take. Power Setup sees it through, so the screensaver before is saved even after Back.
     */
    fun run(block: suspend () -> String?) {
        if (busy) return
        busy = true
        error = null
        scope.launch {
            error = try {
                power.finish(block)
            } catch (e: PrivilegedException) {
                e.message
            }
            name = screensaverName(power, context)
            busy = false
        }
    }

    fun switch() {
        when {
            ours && ready -> run {
                // Lounge's own is never the one before, whatever was saved.
                val before = prefs.previousScreensaver.first()?.takeUnless { it.contains(context.packageName) }
                val r = power.run(power.restoreScreensaver(before, prefs.previousScreensaverEnabled.first()))
                if (r.ok && screensaverName(power, context) != TvScreensaver.Lounge) null else notTaken
            }
            ready -> run {
                val previous = power.currentScreensaver
                val r = power.run(power.useAmbient)
                // The one before is kept only once the switch has happened, so a failed one cannot lose
                // it, and never when it is Lounge's own with the screensaver turned off.
                val took = screensaverName(power, context) == TvScreensaver.Lounge
                if (took && previous?.contains(context.packageName) != true) {
                    prefs.setPreviousScreensaver(previous)
                    prefs.setPreviousScreensaverEnabled(PowerSetup.previousScreensaverEnabled(r.output))
                }
                if (r.ok && took) null else notTaken
            }
            viaTv -> runCatching {
                context.startActivity(tvSettings)
                sentToTv = true
            }.onFailure { tvDidNotTake = true }
            else -> onPowerSetup()
        }
    }

    SheetHeading(
        stringResource(DesignR.string.common_screensaver),
        stringResource(if (ours) R.string.ambient_subtitle_ours else R.string.ambient_subtitle_other),
    )
    Column(verticalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(8.dp)) {
        SheetRow(
            icon = LoungeIcons.Frame,
            title = stringResource(DesignR.string.common_screensaver),
            subtitle = stringResource(
                when {
                    busy -> R.string.ambient_switching
                    ours && ready -> R.string.ambient_ok_puts_back
                    ours -> R.string.ambient_power_puts_back
                    ready -> R.string.ambient_ok_makes_lounge
                    viaTv -> R.string.ambient_ok_opens_tv_settings
                    tvDidNotTake -> R.string.ambient_tv_did_not_take
                    status is PowerStatus.Unavailable -> R.string.ambient_tv_cannot_choose
                    else -> R.string.ambient_ok_opens_power_setup
                },
            ),
            subtitleLines = WHOLE,
            value = name.words(),
            accent = if (ours) colors.signal else colors.dust,
            onClick = { if (!busy) switch() },
            focusRequester = firstFocus.takeUnless { fromPreview },
        )
        // The wait is the TV's own setting, which only Power Setup may change.
        fun stepAfter(by: Int) {
            if (!ready) return onPowerSetup()
            val next = power.screensaverAfter(PowerSetup.nextWait(after, forward = by > 0)) ?: return
            sounds?.tick()
            run {
                val r = power.run(next)
                after = power.screensaverMinutes()
                if (r.ok) null else notTaken
            }
        }
        SheetRow(
            icon = SettingsGlyphs.Clock,
            title = stringResource(R.string.ambient_after_title),
            subtitle = stringResource(if (ready) R.string.ambient_after_subtitle else R.string.ambient_after_needs_power),
            subtitleLines = WHOLE,
            value = after?.let { pluralStringResource(R.plurals.ambient_after_minutes, it, it) },
            onClick = { if (!busy) stepAfter(1) },
            onCycle = if (ready) { by -> if (!busy) stepAfter(by) } else null,
        )
        val scene = choices[at].second
        SheetRow(
            icon = SettingsGlyphs.Scene,
            title = stringResource(R.string.ambient_style_title),
            value = when {
                scene != null -> scene.nameIn(resources)
                choices[at].first == ShaderStyles.MIX -> stringResource(R.string.ambient_style_mix)
                else -> stringResource(R.string.ambient_style_pictures)
            },
            subtitle = when {
                scene != null && problem == SceneRecord.HEAVY -> stringResource(R.string.ambient_style_too_heavy, scene.nameIn(resources))
                scene != null && problem == SceneRecord.BROKEN -> stringResource(R.string.ambient_style_not_drawn, scene.nameIn(resources))
                scene != null -> scene.aboutIn(resources)
                choices[at].first == ShaderStyles.MIX -> stringResource(R.string.ambient_style_mix_about)
                else -> stringResource(R.string.ambient_style_pictures_about)
            },
            subtitleLines = if (problem != null) WHOLE else 2,
            onClick = { step(1) },
            onCycle = { step(it) },
        )
        SheetRow(
            icon = LoungeIcons.Play,
            title = stringResource(R.string.ambient_preview_title),
            subtitle = stringResource(R.string.ambient_preview_subtitle),
            onClick = onPreview,
            focusRequester = firstFocus.takeIf { fromPreview },
        )
        if (error != null) Text(error!!, style = LoungeTheme.typography.caption, color = colors.ember)
    }
    SheetFootnote(
        stringResource(R.string.ambient_footnote_pictures),
        stringResource(R.string.ambient_footnote_when),
    )
}
