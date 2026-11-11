package tv.lounge.feature.home

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.background
import androidx.compose.foundation.basicMarquee
import androidx.compose.foundation.focusable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.geometry.isUnspecified
import androidx.compose.ui.graphics.drawscope.clipRect
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.graphics.painter.Painter
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.onPlaced
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp
import androidx.tv.material3.Icon
import kotlinx.coroutines.delay
import androidx.tv.material3.Text
import androidx.compose.ui.graphics.painter.ColorPainter
import coil3.compose.rememberAsyncImagePainter
import coil3.request.ImageRequest
import coil3.size.Size as CoilSize
import tv.lounge.core.Badge
import tv.lounge.core.RowSize
import tv.lounge.core.Tile
import tv.lounge.core.TileKind
import tv.lounge.design.Badge as BadgeView
import tv.lounge.design.Dimens
import tv.lounge.design.HouseLight
import tv.lounge.design.Lights
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme
import tv.lounge.design.ProgressBar
import tv.lounge.design.argb
import tv.lounge.design.focusRim
import tv.lounge.design.lightGlow
import tv.lounge.design.lightState

/** Tile sizes at 1080p (DESIGN.md §5.3). */
fun tileSize(kind: TileKind, size: RowSize): DpSize = when (kind) {
    TileKind.POSTER -> when (size) {
        RowSize.S -> DpSize(176.dp, 264.dp); RowSize.M -> DpSize(208.dp, 312.dp); RowSize.L -> DpSize(248.dp, 372.dp)
    }
    TileKind.LANDSCAPE -> when (size) {
        RowSize.S -> DpSize(320.dp, 180.dp); RowSize.M -> DpSize(384.dp, 216.dp); RowSize.L -> DpSize(448.dp, 252.dp)
    }
    TileKind.APP_BANNER -> when (size) {
        RowSize.S -> DpSize(256.dp, 144.dp); RowSize.M -> DpSize(288.dp, 162.dp); RowSize.L -> DpSize(320.dp, 180.dp)
    }
    TileKind.COMPACT -> when (size) {
        RowSize.S -> DpSize(288.dp, 72.dp); RowSize.M -> DpSize(320.dp, 80.dp); RowSize.L -> DpSize(352.dp, 88.dp)
    }
}

/** An app's artwork is its banner, which already carries its name. */
private fun Tile.artIsBanner(): Boolean = artworkPath != null && id.startsWith("app:")

/**
 * One tile. Focus is light: scale, glow and the backdrop spill; everything else is a brightness
 * state (DESIGN.md §6.1). [chrome] adds the title block to landscape tiles, [held] keeps a tile lit
 * while its menu is open, [nudge] is a horizontal offset in px read while drawing. [badged] is
 * false in a row that wears the tile's badge itself, so it is said once. [shown] is false while the
 * tile is kept off screen. [description] replaces what is read out about it, for a tile whose
 * words are drawn elsewhere.
 */
@Composable
fun TileView(
    tile: Tile,
    kind: TileKind,
    size: DpSize,
    lights: State<Lights>,
    rowName: String,
    onFocused: (Boolean, Rect) -> Unit,
    onClick: () -> Unit,
    onLongPress: (Rect) -> Unit,
    modifier: Modifier = Modifier,
    focusRequester: FocusRequester? = null,
    chrome: Boolean = true,
    held: Boolean = false,
    nudge: (() -> Float)? = null,
    badged: Boolean = true,
    shown: () -> Boolean = { true },
    description: String? = null,
) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    val visuals = LoungeTheme.visuals
    val light = LoungeTheme.lightState
    var focused by remember { mutableStateOf(false) }
    // Where the tile is, read only when a menu or the light needs it: no state write per layout pass.
    val placed = remember { arrayOfNulls<LayoutCoordinates>(1) }
    fun bounds(): Rect = placed[0]?.takeIf { it.isAttached }?.boundsInRoot() ?: Rect.Zero
    val lightColor = LoungeTheme.colors.light(tile.lightArgb)
    val lit = focused || held

    // Scale, rim and glow follow one level and the house light eases as it draws, all read while
    // drawing: a focus change recomposes this tile once and a neighbour's change not at all.
    val litLevel = animateFloatAsState(if (lit) 1f else 0f, if (lit) motion.snap() else motion.settle(), label = "tileLit")
    val litScale = if (kind == TileKind.COMPACT) 0.04f else 0.06f
    // The idle level is read only by a lit tile, so the room going idle redraws no other glow.
    val glowAlpha = visuals.glowAlpha
    val glow: () -> Float = remember(litLevel, light, glowAlpha) {
        { litLevel.value.let { if (it <= 0.001f) 0f else it * glowAlpha * light.idleLevel } }
    }
    // Read unobserved: a change of light reaches the drawing below and never recomposes the tile.
    val house = remember { HouseLight(Snapshot.withoutReadObservation { if (lit) Lights.Focused else lights.value }) }
    val litNow = rememberUpdatedState(lit)
    // A row hands a new State when the tile moves; the light eases on from where it was.
    val lightsNow = rememberUpdatedState(lights)
    // Off screen the state is read unobserved, so a change in it does not draw the tile again.
    val state: (Boolean) -> Lights = remember(litNow, lightsNow) {
        { seen ->
            val read = { if (litNow.value) Lights.Focused else lightsNow.value.value }
            if (seen) read() else Snapshot.withoutReadObservation(read)
        }
    }
    // Whether it is on screen is watched only while it is not: coming back draws the tile, going away does not.
    val seen: () -> Boolean = remember(shown) { { Snapshot.withoutReadObservation(shown).also { if (!it) shown() } } }
    val filter: () -> androidx.compose.ui.graphics.ColorFilter? = remember(house, light, motion, seen) {
        { seen().let { house.filter(state(it), light, motion, it) } }
    }
    val brightnessNow: () -> Float = remember(house, light, motion, seen) {
        { seen().let { house.brightness(state(it), light, motion, it) } }
    }

    val spoken = description ?: tileDescription(tile, rowName)
    // A compact tile grows with large text rather than cutting its two lines (DESIGN.md §4.1).
    val type = LoungeTheme.typography
    val compactHeight = with(LocalDensity.current) { type.title.lineHeight.toDp() + type.caption.lineHeight.toDp() } + 12.dp
    val drawn = if (kind == TileKind.COMPACT && compactHeight > size.height) size.copy(height = compactHeight) else size

    // The curtain only shows where no picture covers it; an opaque JPEG over it would be pure overdraw.
    val needsCurtain = kind == TileKind.COMPACT || tile.artworkPath == null
    val shape = remember { RoundedCornerShape(Dimens.tileRadius) }
    Box(
        modifier
            .then(if (focusRequester != null) Modifier.focusRequester(focusRequester) else Modifier)
            .onPlaced { coords ->
                placed[0] = coords
                if (focused) light.set(coords, lightColor)
            }
            .onFocusChanged {
                focused = it.isFocused
                if (it.isFocused) placed[0]?.let { c -> light.set(c, lightColor) }
                onFocused(it.isFocused, bounds())
            }
            .tilePress(onClick, { onLongPress(bounds()) }, clickLabel = "Open", longPressLabel = if (hasMenu(tile)) "Options" else null)
            .focusable()
            .semantics { contentDescription = spoken }
            // Two layers on purpose: the glow redraws every frame of a focus change, and inside the
            // tile's own layer it re-records only this tile. The clip is the inner layer.
            .graphicsLayer {
                val scale = 1f + litScale * litLevel.value
                scaleX = scale
                scaleY = scale
                translationX = nudge?.invoke() ?: 0f
                // An app hidden from every row sits back until it has focus.
                if (tile.hidden) alpha = 0.45f + 0.55f * litLevel.value
            }
            .lightGlow(lightColor, glow, cornerRadius = Dimens.tileRadius)
            .graphicsLayer {
                this.shape = shape
                clip = true
            }
            .focusRim(colors, { litLevel.value }, Dimens.tileRadius)
            .size(drawn)
            .then(if (needsCurtain) Modifier.background(colors.curtain) else Modifier),
    ) {
        when (kind) {
            TileKind.POSTER -> PosterContent(tile, filter, size)
            TileKind.LANDSCAPE -> LandscapeContent(tile, filter, brightnessNow, chrome, size, badged, focused)
            TileKind.APP_BANNER -> AppBannerContent(tile, filter, size)
            TileKind.COMPACT -> CompactContent(tile, filter, drawn, focused)
        }
        // A pinned tile says so where it sits, so the word in the menu has something to point at.
        // The live badge owns the top-right corner, so the mark steps down beside the progress bar.
        if (kind != TileKind.COMPACT && (tile.pinned || tile.stagePinned)) {
            val live = tile.badge == Badge.LIVE && chrome && badged
            PinMark(Modifier.align(if (live) Alignment.BottomEnd else Alignment.TopEnd).padding(horizontal = 10.dp, vertical = if (live) 22.dp else 10.dp))
        }
    }
}

/** What is read out for a tile: "<title>, <row name>, <state>" (DESIGN.md §11). */
internal fun tileDescription(tile: Tile, rowName: String): String = buildString {
    append(tile.title)
    append(", ").append(rowName)
    tile.subtitle?.let { append(", ").append(spokenEpisodes(it)) }
    tile.timeLeft?.takeIf { it != tile.subtitle }?.let { append(", ").append(it) }
    when (tile.badge) {
        Badge.LIVE -> append(", live")
        Badge.NEW -> append(", new")
        Badge.CONNECTED -> append(", connected")
        else -> Unit
    }
    if (tile.stagePinned) append(", pinned to the Stage")
    if (tile.pinned) append(", pinned")
}

private val episodeCode = Regex("\\bS(\\d+) E(\\d+)\\b")

/** "S2 E4" as "Season 2, Episode 4", so a screen reader does not spell it out. */
internal fun spokenEpisodes(text: String): String =
    text.replace(episodeCode) { "Season ${it.groupValues[1]}, Episode ${it.groupValues[2]}" }

/** A small pin on a `velvet` disc, the same on every tile style (DESIGN.md §7.3). */
@Composable
private fun PinMark(modifier: Modifier) {
    val colors = LoungeTheme.colors
    Box(modifier.size(26.dp).background(colors.velvet.copy(alpha = 0.72f), androidx.compose.foundation.shape.CircleShape), contentAlignment = Alignment.Center) {
        Icon(LoungeIcons.Pin, contentDescription = null, tint = colors.limelight, modifier = Modifier.size(15.dp))
    }
}

/**
 * The tile's picture over its tint. The painter is drawn by hand so the house-light filter is read
 * while drawing, never in composition. The tint is the painter's placeholder, so nothing is drawn
 * under the picture once it is up.
 */
@Composable
private fun Artwork(path: String?, filter: () -> androidx.compose.ui.graphics.ColorFilter?, tint: Color, size: DpSize, modifier: Modifier = Modifier) {
    val colors = LoungeTheme.colors
    // A picture kept as a PNG can be seen through, so it gets the same body as a tile with none.
    if (path == null || path.endsWith(".png")) {
        // A diagonal of the item's light into curtain, opaque, so the tile reads as made, not missing.
        Box(modifier.fillMaxSize().background(Brush.linearGradient(listOf(tint.copy(alpha = 1f).compositeOver(colors.curtain, 0.4f), colors.curtain))))
    }
    if (path == null) return
    FilteredPicture(path, filter, if (path.endsWith(".png")) null else remember(tint) { ColorPainter(tint) }, size, ContentScale.Crop, modifier.fillMaxSize())
}

/** A picture drawn with [filter] read at draw time, cover-cropped or fitted inside its box. */
@Composable
private fun FilteredPicture(
    path: String,
    filter: () -> androidx.compose.ui.graphics.ColorFilter?,
    placeholder: Painter?,
    size: DpSize,
    contentScale: ContentScale,
    modifier: Modifier,
) {
    val context = LocalContext.current
    val density = LocalDensity.current
    val request = remember(path, size, density) {
        val (w, h) = with(density) { size.width.roundToPx() to size.height.roundToPx() }
        ImageRequest.Builder(context).data(path).size(CoilSize(w.coerceAtLeast(1), h.coerceAtLeast(1))).build()
    }
    val painter = rememberAsyncImagePainter(model = request, placeholder = placeholder, error = placeholder, contentScale = contentScale)
    Box(
        modifier.drawBehind {
            val intrinsic = painter.intrinsicSize
            val cf = filter()
            if (intrinsic.isUnspecified || intrinsic.width <= 0f || intrinsic.height <= 0f) {
                with(painter) { draw(this@drawBehind.size, colorFilter = cf) }
                return@drawBehind
            }
            val sx = this.size.width / intrinsic.width
            val sy = this.size.height / intrinsic.height
            val scale = if (contentScale == ContentScale.Fit) minOf(sx, sy) else maxOf(sx, sy)
            val drawn = Size(intrinsic.width * scale, intrinsic.height * scale)
            val dx = (this.size.width - drawn.width) / 2f
            val dy = (this.size.height - drawn.height) / 2f
            clipRect {
                translate(dx, dy) { with(painter) { draw(drawn, colorFilter = cf) } }
            }
        },
    )
}

private fun Color.compositeOver(base: Color, amount: Float): Color = Color(
    red = base.red + (red - base.red) * amount,
    green = base.green + (green - base.green) * amount,
    blue = base.blue + (blue - base.blue) * amount,
    alpha = 1f,
)

/**
 * A one-line title cut at a word boundary with an ellipsis when it would overflow [width],
 * never mid-word (DESIGN.md §4.2). Measured once for each text and width, and remembered.
 */
@Composable
fun rememberFittedLine(text: String, style: TextStyle, width: Dp): String = rememberFittedLines(text, style, width, 1)

/**
 * Text that wraps to at most [maxLines] lines within [width] and, when it would need more, is cut
 * at a word boundary with an ellipsis, never mid-word (DESIGN.md §4.2).
 */
@Composable
fun rememberFittedLines(text: String, style: TextStyle, width: Dp, maxLines: Int): String {
    val measurer = rememberTextMeasurer()
    val density = LocalDensity.current
    return remember(text, style, width, density, maxLines) {
        val maxWidth = with(density) { width.roundToPx() }.coerceAtLeast(1)
        fun overflows(s: String) = measurer.measure(AnnotatedString(s), style, maxLines = maxLines, softWrap = maxLines > 1, constraints = Constraints(maxWidth = maxWidth)).hasVisualOverflow
        if (!overflows(text)) return@remember text
        var cut = text.lastIndexOf(' ')
        while (cut > 0) {
            val candidate = text.substring(0, cut).trimEnd(' ', ',', ':', ';', '-') + "…"
            if (!overflows(candidate)) return@remember candidate
            cut = text.lastIndexOf(' ', cut - 1)
        }
        text
    }
}

@Composable
private fun PosterContent(tile: Tile, filter: () -> androidx.compose.ui.graphics.ColorFilter?, size: DpSize) {
    val colors = LoungeTheme.colors
    Box(Modifier.fillMaxSize()) {
        Artwork(tile.artworkPath, filter, LoungeTheme.colors.light(tile.lightArgb).copy(alpha = 0.25f), size)
        TileBadge(tile.badge, Modifier.align(Alignment.TopStart).padding(16.dp))
        if (tile.artworkPath == null) {
            Text(
                tile.title,
                style = LoungeTheme.typography.title,
                color = colors.limelight,
                maxLines = 3,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.align(Alignment.BottomStart).padding(16.dp),
            )
        }
    }
}

@Composable
private fun LandscapeContent(
    tile: Tile,
    filter: () -> androidx.compose.ui.graphics.ColorFilter?,
    brightness: () -> Float,
    chrome: Boolean,
    size: DpSize,
    badged: Boolean,
    focused: Boolean,
) {
    val colors = LoungeTheme.colors
    // A banner names itself and the filmstrip speaks through its pictures; text only with chrome or when the art is missing.
    // A borrowed banner always carries its title, or the strip would show the app twice over.
    val showText = (chrome || tile.artworkPath == null || tile.borrowedArt) && !tile.artIsBanner()
    val live = tile.badge == Badge.LIVE
    // Over a picture that says some of the title, only the rest of it, which may be nothing.
    val words = tile.artTitle ?: tile.title
    // A channel's card, lent to what is on: it names the channel, so it is kept readable.
    val card = live && tile.borrowedArt
    // Where the words sit, measured up from the tile's foot, so the scrim can be deep enough behind each line.
    val type = LoungeTheme.typography
    val density = LocalDensity.current
    val titleLine = if (words.isNotEmpty()) with(density) { type.title.lineHeight.toDp() } + 2.dp else 0.dp
    val metaLine = if (tile.subtitle != null) with(density) { type.meta.lineHeight.toDp() } else 0.dp
    val wordsHeight = (if (tile.progress != null) 32.dp else 20.dp) + titleLine + metaLine
    val plainScrim = (wordsHeight + ScrimFade).coerceAtMost(size.height)
    Box(Modifier.fillMaxSize()) {
        Artwork(tile.artworkPath, filter, LoungeTheme.colors.light(tile.lightArgb).copy(alpha = 0.25f), size)
        if (showText && tile.artworkPath != null) {
            val scrim = when {
                // With no title to carry, the card needs only a foot for the line of time left.
                card && words.isEmpty() -> Brush.verticalGradient(0f to Color.Transparent, 1f to colors.velvet.copy(alpha = 0.85f))
                // Its upper half stays clear, and the words over the lower half stand off it.
                card -> Brush.verticalGradient(0f to Color.Transparent, 0.40f to colors.velvet.copy(alpha = 0.25f), 0.62f to colors.velvet.copy(alpha = 0.78f), 1f to colors.velvet.copy(alpha = 0.92f))
                // A borrowed banner is a backdrop, not the picture, so its scrim starts higher and ends deeper.
                tile.borrowedArt ->
                    Brush.verticalGradient(0f to colors.velvet.copy(alpha = 0.10f), 0.35f to colors.velvet.copy(alpha = 0.70f), 1f to colors.velvet.copy(alpha = 0.94f))
                else -> {
                    val wordsTop = plainScrim - wordsHeight
                    wordsScrim(colors.velvet, plainScrim, titleTop = wordsTop.takeIf { titleLine > 0.dp }, metaTop = (wordsTop + titleLine).takeIf { metaLine > 0.dp })
                }
            }
            Box(
                Modifier
                    .align(Alignment.BottomCenter)
                    .fillMaxWidth()
                    .height(
                        when {
                            card && words.isEmpty() -> size.height * 0.35f
                            tile.borrowedArt -> size.height
                            else -> plainScrim
                        },
                    )
                    .background(scrim),
            )
        }
        // A live tile carries the channel's logo: large above the words when it is all the picture
        // there is, else top-left over the programme's own (DESIGN.md §7.1).
        if (live && tile.iconPath != null && chrome) {
            if (tile.artworkPath == null) {
                val logo = DpSize(176.dp, 92.dp)
                FilteredPicture(tile.iconPath!!, filter, null, logo, ContentScale.Fit, Modifier.align(Alignment.TopCenter).padding(top = 24.dp).size(logo))
            } else {
                FilteredPicture(
                    tile.iconPath!!,
                    filter,
                    null,
                    DpSize(56.dp, 56.dp),
                    ContentScale.Fit,
                    Modifier.align(Alignment.TopStart).padding(16.dp).size(56.dp).clip(RoundedCornerShape(8.dp)),
                )
            }
        }
        if (chrome && badged) TileBadge(tile.badge, Modifier.align(if (live) Alignment.TopEnd else Alignment.TopStart).padding(16.dp))
        if (showText) {
            Column(
                Modifier.align(Alignment.BottomStart).padding(start = 20.dp, end = 20.dp, bottom = if (tile.progress != null) 32.dp else 20.dp),
                verticalArrangement = Arrangement.spacedBy(2.dp),
            ) {
                if (words.isNotEmpty()) RunningTitle(words, LoungeTheme.typography.title, size.width - 40.dp, focused)
                tile.subtitle?.let { Text(it, style = LoungeTheme.typography.meta, color = colors.dust, maxLines = 1, overflow = TextOverflow.Ellipsis) }
            }
        }
        tile.progress?.let {
            ProgressBar(
                it,
                Modifier.align(Alignment.BottomCenter).padding(start = 16.dp, end = 16.dp, bottom = 12.dp).fillMaxWidth(),
                thickness = 4.dp,
                brightness = brightness,
            )
        }
    }
}

/** How far above the words a landscape tile's scrim starts to fall in. */
private val ScrimFade = 56.dp

/**
 * The scrim under a landscape tile's words: clear above them, deep enough by [titleTop] for
 * `limelight` and by [metaTop] for `dust` to reach 4.5 to 1 over white artwork.
 */
private fun wordsScrim(velvet: Color, height: Dp, titleTop: Dp?, metaTop: Dp?): Brush {
    val stops = buildList {
        add(0f to Color.Transparent)
        titleTop?.let { add((it / height).coerceIn(0.01f, 0.98f) to velvet.copy(alpha = 0.66f)) }
        metaTop?.let { add((it / height).coerceIn(0.02f, 0.99f) to velvet.copy(alpha = 0.86f)) }
        add(1f to velvet.copy(alpha = 0.92f))
    }.distinctBy { it.first }.sortedBy { it.first }
    return Brush.verticalGradient(*stops.toTypedArray())
}

@Composable
private fun AppBannerContent(tile: Tile, filter: () -> androidx.compose.ui.graphics.ColorFilter?, size: DpSize) {
    val colors = LoungeTheme.colors
    if (tile.artworkPath != null) {
        Artwork(tile.artworkPath, filter, LoungeTheme.colors.light(tile.lightArgb).copy(alpha = 0.25f), size)
    } else {
        Row(
            Modifier.fillMaxSize().padding(horizontal = 20.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            if (tile.iconPath != null) {
                FilteredPicture(tile.iconPath!!, filter, null, DpSize(72.dp, 72.dp), ContentScale.Fit, Modifier.size(72.dp).clip(RoundedCornerShape(16.dp)))
            } else {
                Icon(LoungeIcons.Apps, contentDescription = null, tint = if (tile.id == "control:allapps") colors.houselight else colors.dust, modifier = Modifier.size(48.dp))
            }
            Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(tile.title, style = LoungeTheme.typography.title, color = colors.limelight, maxLines = 2, overflow = TextOverflow.Ellipsis)
                // Only the All apps tile has a line under its name: how many it opens onto.
                tile.subtitle?.let { Text(it, style = LoungeTheme.typography.caption, color = colors.dust, maxLines = 1) }
            }
        }
    }
}

/** An icon and two lines; both are cut at a word and run across once the tile has held focus a while. */
@Composable
private fun CompactContent(tile: Tile, filter: () -> androidx.compose.ui.graphics.ColorFilter?, size: DpSize, focused: Boolean) {
    val colors = LoungeTheme.colors
    val pinned = tile.pinned || tile.stagePinned
    val connected = tile.badge == Badge.CONNECTED
    // The row's padding, the icon, the gaps and whatever trails the words.
    val wordsWidth = size.width - 32.dp - (if (tile.iconPath != null) 48.dp else 32.dp) - 14.dp -
        (if (pinned) 15.dp + 14.dp else 0.dp) - (if (connected) 10.dp + 14.dp else 0.dp)
    Row(
        Modifier.fillMaxSize().padding(horizontal = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        when {
            tile.iconPath != null -> FilteredPicture(tile.iconPath!!, filter, null, DpSize(48.dp, 48.dp), ContentScale.Fit, Modifier.size(48.dp).clip(RoundedCornerShape(12.dp)))
            tile.id.startsWith("device:") -> Icon(LoungeIcons.Tv, contentDescription = null, tint = colors.houselight, modifier = Modifier.size(32.dp))
            tile.id.startsWith("notice:") -> Icon(LoungeIcons.Info, contentDescription = null, tint = colors.houselight, modifier = Modifier.size(32.dp))
            // A tile to read that brings no picture is its words alone.
            tile.action.isEmpty() -> Unit
            else -> Icon(LoungeIcons.Apps, contentDescription = null, tint = colors.dust, modifier = Modifier.size(32.dp))
        }
        Column(Modifier.weight(1f)) {
            RunningTitle(tile.title, LoungeTheme.typography.title, wordsWidth, focused)
            tile.subtitle?.let { RunningTitle(it, LoungeTheme.typography.caption, wordsWidth, focused, color = colors.dust) }
        }
        if (pinned) Icon(LoungeIcons.Pin, contentDescription = null, tint = colors.dust, modifier = Modifier.size(15.dp))
        if (connected) tv.lounge.design.Dot(colors.signal)
    }
}

/** A badge on a `velvet` plate, as the pin is, so its word reads over a pale picture (DESIGN.md §7.3). */
@Composable
private fun TileBadge(badge: Badge?, modifier: Modifier) {
    val colors = LoungeTheme.colors
    if (badge != Badge.LIVE && badge != Badge.NEW) return
    Box(modifier.background(colors.velvet.copy(alpha = 0.85f), RoundedCornerShape(14.dp)).padding(horizontal = 10.dp, vertical = 2.dp)) {
        when (badge) {
            // Reads idle here, so only the badge recomposes for it.
            Badge.LIVE -> BadgeView("Live", dot = colors.ember, idle = LoungeTheme.lightState.idle)
            else -> BadgeView("New", textColor = colors.houselight)
        }
    }
}

/** Title that appears under a focused poster tile (DESIGN.md §7.1). */
@Composable
fun FocusedTitle(visible: Boolean, title: String, width: Dp) {
    AnimatedVisibility(visible, enter = fadeIn(LoungeTheme.motion.snap()), exit = fadeOut(LoungeTheme.motion.settle())) {
        RunningTitle(title, LoungeTheme.typography.meta, width, visible, Modifier.padding(top = 12.dp))
    }
}

/** How long a tile is sat on before a title cut short starts across. */
private const val TITLE_RUNS_AFTER_MS = 1_200L

/**
 * A one-line title [width] wide. One cut short runs across, over and over, once its tile has been
 * [focused] a while, so all of it can be read; with reduced motion it stays cut (DESIGN.md §4.2).
 */
@Composable
private fun RunningTitle(text: String, style: TextStyle, width: Dp, focused: Boolean, modifier: Modifier = Modifier, color: Color = LoungeTheme.colors.limelight) {
    val fitted = rememberFittedLine(text, style, width)
    val cut = fitted != text
    val still = LoungeTheme.motion.reducedMotion
    var running by remember { mutableStateOf(false) }
    LaunchedEffect(focused, cut, still) {
        running = false
        if (focused && cut && !still) {
            delay(TITLE_RUNS_AFTER_MS)
            running = true
        }
    }
    if (running) {
        Text(
            text,
            style = style,
            color = color,
            maxLines = 1,
            softWrap = false,
            modifier = modifier.width(width).fadedEnds().basicMarquee(iterations = Int.MAX_VALUE, repeatDelayMillis = 1_500, initialDelayMillis = 0, velocity = 72.dp),
        )
    } else {
        Text(fitted, style = style, color = color, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = modifier)
    }
}

/** Fades what is drawn out towards both ends, so words running across arrive and leave softly. */
private fun Modifier.fadedEnds(): Modifier = graphicsLayer { compositingStrategy = CompositingStrategy.Offscreen }.drawWithContent {
    drawContent()
    val fade = (20.dp.toPx() / size.width).coerceAtMost(0.25f)
    drawRect(Brush.horizontalGradient(0f to Color.Transparent, fade to Color.Black, 1f - fade to Color.Black, 1f to Color.Transparent), blendMode = BlendMode.DstIn)
}
