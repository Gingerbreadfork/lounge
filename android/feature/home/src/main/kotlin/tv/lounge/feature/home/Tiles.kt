package tv.lounge.feature.home

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.background
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
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
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
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.launch
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
import tv.lounge.design.Lights
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme
import tv.lounge.design.ProgressBar
import tv.lounge.design.argb
import tv.lounge.design.focusRim
import tv.lounge.design.lightGlow
import tv.lounge.design.lightState
import tv.lounge.design.lightsFilter

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
 * while its menu is open, [nudge] is a horizontal offset in px read while drawing.
 */
@Composable
fun TileView(
    tile: Tile,
    kind: TileKind,
    size: DpSize,
    lights: State<Lights>,
    idle: Boolean,
    rowName: String,
    onFocused: (Boolean, Rect) -> Unit,
    onClick: () -> Unit,
    onLongPress: (Rect) -> Unit,
    modifier: Modifier = Modifier,
    focusRequester: FocusRequester? = null,
    chrome: Boolean = true,
    held: Boolean = false,
    nudge: (() -> Float)? = null,
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

    // Scale, glow and the colour filter are read while drawing, and the house lights arrive as a
    // State, so a focus change recomposes this tile once and a neighbour's change not at all.
    val targetScale = if (lit) (if (kind == TileKind.COMPACT) 1.04f else 1.06f) else 1f
    val scale = animateFloatAsState(targetScale, if (lit) motion.snap() else motion.settle(), label = "tileScale")
    val glow = animateFloatAsState(
        if (lit) visuals.glowAlpha * (if (idle) 0.5f else 1f) else 0f,
        when {
            idle -> motion.dim()
            lit -> motion.snap()
            else -> motion.settle()
        },
        label = "tileGlow",
    )
    val rim = animateFloatAsState(if (lit) 1f else 0f, if (lit) motion.snap() else motion.settle(), label = "tileRim")
    val initial = remember { (if (lit) Lights.Focused else lights.value).dimmed(idle) }
    val animBrightness = remember { Animatable(initial.first) }
    val animSaturation = remember { Animatable(initial.second) }
    // Keyed on the State too: a row hands a new one when the room dims or the tile moves.
    LaunchedEffect(lit, idle, lights) {
        snapshotFlow { (if (lit) Lights.Focused else lights.value).dimmed(idle) }.collectLatest { (b, sat) ->
            val spec = if (idle) motion.dim<Float>() else motion.settle()
            launch { animBrightness.animateTo(b, spec) }
            animSaturation.animateTo(sat, spec)
        }
    }
    val filter: () -> androidx.compose.ui.graphics.ColorFilter? = remember(animBrightness, animSaturation) {
        { lightsFilter(animBrightness.value, animSaturation.value) }
    }
    val brightnessNow: () -> Float = remember(animBrightness) { { animBrightness.value } }

    val description = buildString {
        append(tile.title)
        append(", ").append(rowName)
        tile.subtitle?.let { append(", ").append(it) }
        when (tile.badge) {
            Badge.LIVE -> append(", live")
            Badge.NEW -> append(", new")
            Badge.CONNECTED -> append(", connected")
            else -> Unit
        }
    }

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
            .tilePress(onClick) { onLongPress(bounds()) }
            .focusable()
            .semantics { contentDescription = description }
            // Two layers on purpose: the glow redraws every frame of a focus change, and inside the
            // tile's own layer it re-records only this tile. The clip is the inner layer.
            .graphicsLayer {
                scaleX = scale.value
                scaleY = scale.value
                translationX = nudge?.invoke() ?: 0f
            }
            .lightGlow(lightColor, { glow.value }, cornerRadius = Dimens.tileRadius)
            .graphicsLayer {
                this.shape = shape
                clip = true
            }
            .focusRim(colors, { rim.value }, Dimens.tileRadius)
            .size(size)
            .then(if (needsCurtain) Modifier.background(colors.curtain) else Modifier),
    ) {
        when (kind) {
            TileKind.POSTER -> PosterContent(tile, filter, idle, size)
            TileKind.LANDSCAPE -> LandscapeContent(tile, filter, brightnessNow, idle, chrome, size)
            TileKind.APP_BANNER -> AppBannerContent(tile, filter, size)
            TileKind.COMPACT -> CompactContent(tile, filter)
        }
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
    if (path == null) {
        // No picture: a diagonal of the item's light into curtain, opaque, so the tile reads as made, not missing.
        Box(modifier.fillMaxSize().background(Brush.linearGradient(listOf(tint.copy(alpha = 1f).compositeOver(colors.curtain, 0.4f), colors.curtain))))
        return
    }
    FilteredPicture(path, filter, remember(tint) { ColorPainter(tint) }, size, ContentScale.Crop, modifier.fillMaxSize())
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
 * never mid-word (DESIGN.md §4.2). Measured once per (text, width) and remembered.
 */
@Composable
fun rememberFittedLine(text: String, style: TextStyle, width: Dp): String {
    val measurer = rememberTextMeasurer()
    val density = LocalDensity.current
    return remember(text, style, width, density) {
        val maxWidth = with(density) { width.roundToPx() }.coerceAtLeast(1)
        fun overflows(s: String) = measurer.measure(AnnotatedString(s), style, maxLines = 1, softWrap = false, constraints = Constraints(maxWidth = maxWidth)).hasVisualOverflow
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
private fun PosterContent(tile: Tile, filter: () -> androidx.compose.ui.graphics.ColorFilter?, idle: Boolean, size: DpSize) {
    val colors = LoungeTheme.colors
    Box(Modifier.fillMaxSize()) {
        Artwork(tile.artworkPath, filter, LoungeTheme.colors.light(tile.lightArgb).copy(alpha = 0.25f), size)
        TileBadge(tile.badge, idle, Modifier.align(Alignment.TopStart).padding(16.dp))
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
    idle: Boolean,
    chrome: Boolean,
    size: DpSize,
) {
    val colors = LoungeTheme.colors
    // A banner names itself; the strip is a filmstrip of pictures. Text and scrim only where they add something.
    val showText = chrome && !tile.artIsBanner()
    val live = tile.badge == Badge.LIVE
    Box(Modifier.fillMaxSize()) {
        Artwork(tile.artworkPath, filter, LoungeTheme.colors.light(tile.lightArgb).copy(alpha = 0.25f), size)
        if (showText && tile.artworkPath != null) {
            Box(
                Modifier
                    .align(Alignment.BottomCenter)
                    .fillMaxWidth()
                    .height(size.height * 0.6f)
                    .background(Brush.verticalGradient(0f to Color.Transparent, 0.45f to colors.velvet.copy(alpha = 0.55f), 1f to colors.velvet.copy(alpha = 0.90f))),
            )
        }
        // A live tile carries the channel's logo top-left and its badge top-right (DESIGN.md §7.1).
        if (live && tile.iconPath != null && chrome) {
            FilteredPicture(
                tile.iconPath!!,
                filter,
                null,
                DpSize(56.dp, 56.dp),
                ContentScale.Fit,
                Modifier.align(Alignment.TopStart).padding(16.dp).size(56.dp).clip(RoundedCornerShape(8.dp)),
            )
        }
        if (chrome) TileBadge(tile.badge, idle, Modifier.align(if (live) Alignment.TopEnd else Alignment.TopStart).padding(16.dp))
        if (showText) {
            val title = rememberFittedLine(tile.title, LoungeTheme.typography.title, size.width - 40.dp)
            Column(
                Modifier.align(Alignment.BottomStart).padding(start = 20.dp, end = 20.dp, bottom = if (tile.progress != null) 32.dp else 20.dp),
                verticalArrangement = Arrangement.spacedBy(2.dp),
            ) {
                Text(title, style = LoungeTheme.typography.title, color = colors.limelight, maxLines = 1, overflow = TextOverflow.Ellipsis)
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
                Icon(LoungeIcons.Apps, contentDescription = null, tint = colors.dust, modifier = Modifier.size(48.dp))
            }
            Text(tile.title, style = LoungeTheme.typography.title, color = colors.limelight, maxLines = 2, overflow = TextOverflow.Ellipsis)
        }
    }
}

@Composable
private fun CompactContent(tile: Tile, filter: () -> androidx.compose.ui.graphics.ColorFilter?) {
    val colors = LoungeTheme.colors
    Row(
        Modifier.fillMaxSize().padding(horizontal = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        when {
            tile.iconPath != null -> FilteredPicture(tile.iconPath!!, filter, null, DpSize(48.dp, 48.dp), ContentScale.Fit, Modifier.size(48.dp).clip(RoundedCornerShape(12.dp)))
            tile.id.startsWith("device:") -> Icon(LoungeIcons.Tv, contentDescription = null, tint = colors.houselight, modifier = Modifier.size(32.dp))
            tile.id.startsWith("notice:") -> Icon(LoungeIcons.Info, contentDescription = null, tint = colors.houselight, modifier = Modifier.size(32.dp))
            else -> Icon(LoungeIcons.Apps, contentDescription = null, tint = colors.dust, modifier = Modifier.size(32.dp))
        }
        Column(Modifier.weight(1f)) {
            Text(tile.title, style = LoungeTheme.typography.title, color = colors.limelight, maxLines = 1, overflow = TextOverflow.Ellipsis)
            tile.subtitle?.let { Text(it, style = LoungeTheme.typography.caption, color = colors.dust, maxLines = 1, overflow = TextOverflow.Ellipsis) }
        }
        if (tile.badge == Badge.CONNECTED) tv.lounge.design.Dot(colors.signal)
    }
}

@Composable
private fun TileBadge(badge: Badge?, idle: Boolean, modifier: Modifier) {
    val colors = LoungeTheme.colors
    when (badge) {
        Badge.LIVE -> BadgeView("Live", dot = colors.ember, modifier = modifier, idle = idle)
        Badge.NEW -> BadgeView("New", textColor = colors.houselight, modifier = modifier)
        else -> Unit
    }
}

/** Title that appears under a focused poster tile (DESIGN.md §7.1). */
@Composable
fun FocusedTitle(visible: Boolean, title: String, width: Dp) {
    AnimatedVisibility(visible, enter = fadeIn(LoungeTheme.motion.snap()), exit = fadeOut(LoungeTheme.motion.settle())) {
        Text(
            title,
            style = LoungeTheme.typography.meta,
            color = LoungeTheme.colors.limelight,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.width(width).padding(top = 12.dp),
        )
    }
}
