package tv.lounge.feature.home

import androidx.compose.foundation.layout.requiredHeight
import androidx.compose.foundation.layout.requiredSize
import androidx.compose.ui.layout.layout
import androidx.compose.runtime.derivedStateOf
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.focusGroup
import androidx.compose.foundation.focusable
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.foundation.gestures.LocalBringIntoViewSpec
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.wrapContentSize
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.layout.LazyLayoutCacheWindow
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.focus.focusRestorer
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.Density
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.Outline
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.geometry.RoundRect
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp
import androidx.tv.material3.Icon
import androidx.tv.material3.Text
import androidx.compose.ui.graphics.painter.BrushPainter
import coil3.SingletonImageLoader
import coil3.compose.AsyncImage
import coil3.request.ImageRequest
import tv.lounge.core.SpotlightKind
import tv.lounge.core.Stage
import tv.lounge.core.StageSpotlight
import tv.lounge.core.Tile
import tv.lounge.design.ButtonKind
import tv.lounge.design.Dimens
import tv.lounge.design.Lights
import tv.lounge.design.LocalSwapSlotActive
import tv.lounge.design.LoungeButton
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme
import tv.lounge.design.ProgressBar
import tv.lounge.design.BillLight
import tv.lounge.design.LocalLightState
import androidx.compose.runtime.DisposableEffect
import androidx.compose.ui.layout.onPlaced
import androidx.compose.ui.layout.positionInRoot
import tv.lounge.design.SpillMask
import tv.lounge.design.Swap
import tv.lounge.design.argb
import tv.lounge.design.drawSpill
import tv.lounge.design.glowRoom
import tv.lounge.design.lightGlow

/** The playbill's text clears in the first 40 % of a swap and the new text starts at 35 %, so the two never overlap. */
private const val TEXT_OUT = 0.4f
private const val TEXT_IN_DELAY = 0.35f

/** Expanded Stage height: artwork 630 + gap 32 + strip 130 + gap 32 (DESIGN.md §5.1). */
val StageExpandedHeight = 630.dp + 32.dp + Dimens.stripTileHeight + 32.dp
val StageCollapsedHeight = Dimens.collapsedStageHeight + 32.dp

/**
 * The Stage: playbill, artwork and strip when expanded; a 200 dp band when focus is in the rows.
 * The proscenium lowers over 260 ms as one picture: the frame travels from beside the playbill to
 * the full-width band, cropping to the focal line, while the playbill and strip fade (DESIGN.md
 * §5.2, §6.3).
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun StageView(
    stage: Stage,
    selectedIndex: Int,
    selected: androidx.compose.runtime.IntState,
    expanded: Boolean,
    height: State<Dp>,
    idle: Boolean,
    indexed: Boolean,
    stripRequester: (Int) -> FocusRequester,
    stripFocus: () -> FocusRequester,
    playbillFocus: FocusRequester,
    upFocus: FocusRequester,
    onStripFocused: (Int, Tile) -> Unit,
    onStripClick: (Tile) -> Unit,
    onStripLongPress: (Tile, Rect, FocusRequester) -> Unit,
    onStripMoved: () -> Unit,
    onPrimary: (StageSpotlight) -> Unit,
    onSecondary: (StageSpotlight) -> Unit,
    onBandFocused: () -> Unit,
    onPlaybillFocusChanged: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
) {
    val motion = LoungeTheme.motion
    // The height is read in the layout phase and the progress in layout and draw, so the
    // proscenium animation never recomposes the Stage.
    val spotlight = stage.spotlights.getOrNull(selectedIndex) ?: stage.spotlights.firstOrNull()
    val progress = animateFloatAsState(if (expanded) 1f else 0f, motion.proscenium(), label = "stageProgress")
    val read: () -> Float = remember(progress) { { progress.value } }
    // The playbill and strip go in the first 60 % of the lowering and come back in the last 60 % of the raising.
    val shown: () -> Float = remember(progress) { { ((progress.value - 0.4f) / 0.6f).coerceIn(0f, 1f) } }
    // Read when focus searches, so the strip can refuse focus without recomposing its tiles.
    val expandedNow = rememberUpdatedState(expanded)
    val canTakeFocus: () -> Boolean = remember(expandedNow) { { expandedNow.value } }

    PrefetchStageArt(stage, selectedIndex)

    // The box never changes size and everything that moves here moves as a layer property, so a
    // frame of the lowering lays out nothing.
    val density = LocalDensity.current
    val geometry = remember(density, spotlight?.focalY) { FrameGeometry(density, spotlight?.focalY ?: 0.5f) }
    Box(modifier.fillMaxWidth().requiredHeight(StageExpandedHeight)) {
        if (spotlight != null) {
            StageFrame(
                spotlight = spotlight,
                index = stage.spotlights.indexOfFirst { it.id == spotlight.id },
                progress = read,
                geometry = geometry,
            )
            // The band's chrome is its own node at the band's place, so UP from the first row finds
            // something strictly above it. It stays composed while the Stage is up, transparent and
            // unfocusable.
            run {
                Box(
                    Modifier
                        .padding(start = Dimens.safeX)
                        .graphicsLayer {
                            clip = true
                            shape = geometry.frameShape(read(), originX = Dimens.safeX.toPx())
                        },
                ) {
                    BandChrome(spotlight, read, !expanded, onBandFocused, LoungeTheme.colors)
                }
            }
        }
        // The playbill and strip stay composed while lowered, transparent and unfocusable, so
        // raising the Stage composes nothing.
        ExpandedStage(
            stage = stage,
            spotlight = spotlight,
            selected = selected,
            idle = idle,
            indexed = indexed,
            stripRequester = stripRequester,
            stripFocus = stripFocus,
            playbillFocus = playbillFocus,
            upFocus = upFocus,
            onStripFocused = onStripFocused,
            onStripClick = onStripClick,
            onStripLongPress = onStripLongPress,
            onStripMoved = onStripMoved,
            onPrimary = onPrimary,
            onSecondary = onSecondary,
            onPlaybillFocusChanged = onPlaybillFocusChanged,
            // Alpha modulates each draw, no offscreen layer.
            modifier = Modifier
                .fillMaxWidth()
                .requiredHeight(StageExpandedHeight)
                .graphicsLayer {
                    alpha = shown()
                    compositingStrategy = CompositingStrategy.ModulateAlpha
                },
            interactive = expanded,
            shown = shown,
            canTakeFocus = canTakeFocus,
        )
    }
}

/** Decoded Stage pictures ahead of the selection, so a change never shows a placeholder first. */
@Composable
private fun PrefetchStageArt(stage: Stage, selectedIndex: Int) {
    val context = LocalContext.current
    val all = LoungeTheme.visuals.tierA
    val key = if (all) stage.spotlights.map { it.artworkPath } else listOf(selectedIndex, stage.spotlights.map { it.artworkPath })
    LaunchedEffect(key) {
        val loader = SingletonImageLoader.get(context)
        val wanted = if (all) stage.spotlights.indices else (selectedIndex - 1..selectedIndex + 1)
        for (i in wanted) {
            val sp = stage.spotlights.getOrNull(i) ?: continue
            listOfNotNull(sp.artworkPath, sp.bandPath).forEach { path ->
                loader.enqueue(ImageRequest.Builder(context).data(path).build())
            }
        }
    }
}

/**
 * The one picture. Its frame interpolates between the artwork's place beside the playbill and the
 * band. The picture is laid out once at the band's width and only scaled and shifted, and the frame
 * is a clip outline on a layer, so a frame of the lowering lays out nothing. The band's chrome
 * lives inside the clip at its final size.
 */
@Composable
private fun StageFrame(
    spotlight: StageSpotlight,
    index: Int,
    progress: () -> Float,
    geometry: FrameGeometry,
) {
    Box(
        Modifier
            .fillMaxWidth()
            .requiredHeight(Dimens.stageArtworkHeight)
            .graphicsLayer {
                clip = true
                shape = geometry.frameShape(progress())
            },
    ) {
        Box(
            Modifier
                // Taller than the frame, so it must be pinned to the top: an oversized child is
                // otherwise centred and the frame loses the picture's top.
                .wrapContentSize(Alignment.TopStart, unbounded = true)
                .requiredSize(BandWidth, BandWidth * 9 / 16)
                .graphicsLayer {
                    val p = progress()
                    val s = geometry.scale(p)
                    scaleX = s
                    scaleY = s
                    transformOrigin = TransformOrigin(0f, 0f)
                    translationX = geometry.left(p)
                    translationY = geometry.pictureTop(p)
                },
        ) {
            StageArtworkCrossfade(spotlight, index, Modifier.fillMaxSize())
        }
    }
}

/** The frame's rectangle and the picture's placement at a given progress (1 expanded, 0 lowered), in px. */
private class FrameGeometry(density: Density, private val focalY: Float) {
    private val safe = with(density) { Dimens.safeX.toPx() }
    private val bandW = with(density) { BandWidth.toPx() }
    private val bandH = with(density) { Dimens.collapsedStageHeight.toPx() }
    private val artW = with(density) { Dimens.stageArtworkWidth.toPx() }
    private val artH = with(density) { Dimens.stageArtworkHeight.toPx() }
    private val artX = with(density) { (Dimens.safeX + Dimens.playbillWidth + 48.dp).toPx() }
    private val radius = with(density) { Dimens.stageRadius.toPx() }
    private val pictureH = bandW * 9f / 16f

    fun left(p: Float) = safe + (artX - safe) * p
    fun width(p: Float) = bandW + (artW - bandW) * p
    fun height(p: Float) = bandH + (artH - bandH) * p
    fun scale(p: Float) = width(p) / bandW

    /** The picture's top edge: its focal line at the frame's middle, never showing the frame's background. */
    fun pictureTop(p: Float): Float {
        val fh = height(p)
        val ph = pictureH * scale(p)
        return (fh / 2f - focalY * ph).coerceIn(fh - ph, 0f)
    }

    /** The frame's outline, in the coordinates of a node whose left edge sits at [originX]. */
    fun frameShape(p: Float, originX: Float = 0f): Shape = FrameShape(left(p) - originX, width(p), height(p), radius)
}

private data class FrameShape(val left: Float, val width: Float, val height: Float, val radius: Float) : Shape {
    override fun createOutline(size: Size, layoutDirection: LayoutDirection, density: Density): Outline =
        Outline.Rounded(RoundRect(left, 0f, left + width, height, CornerRadius(radius)))
}


/** The band spans the safe width of a 1920-wide canvas. */
private val BandWidth = 1920.dp - Dimens.safeX * 2

/**
 * What the band adds when lowered: house-light dimming, a velvet fade on the left with the title
 * and a play arrow, the progress bar, and the focusable surface that raises the Stage. It stays
 * focusable until it leaves composition so focus hands over to the strip.
 */
@Composable
private fun BandChrome(
    spotlight: StageSpotlight,
    progress: () -> Float,
    interactive: Boolean,
    onFocused: () -> Unit,
    colors: tv.lounge.design.LoungeColors,
) {
    // Laid out once at the band's final size; the frame clips it while it is still growing.
    Box(
        Modifier
            .size(BandWidth, Dimens.collapsedStageHeight)
            .graphicsLayer {
                alpha = (1f - progress() * 2f).coerceIn(0f, 1f)
                compositingStrategy = CompositingStrategy.ModulateAlpha
            }
            .onFocusChanged { if (it.isFocused) onFocused() }
            .then(if (interactive) Modifier.focusable() else Modifier),
    ) {
        spotlight.bandPath?.let { band ->
            // The sharp, index-time slice replaces the scaled hero once the frame has settled.
            AsyncImage(
                model = band,
                contentDescription = null,
                contentScale = ContentScale.Crop,
                modifier = Modifier.fillMaxSize().graphicsLayer { alpha = (1f - progress() * 6f).coerceIn(0f, 1f) },
            )
        }
        Box(Modifier.fillMaxSize().background(colors.velvet.copy(alpha = 0.22f)))
        Box(
            Modifier.fillMaxSize().background(
                Brush.horizontalGradient(0f to colors.velvet.copy(alpha = 0.82f), 0.4f to colors.velvet.copy(alpha = 0.35f), 0.7f to Color.Transparent),
            ),
        )
        Column(Modifier.align(Alignment.CenterStart).padding(start = 32.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(14.dp)) {
                Text(spotlight.title, style = LoungeTheme.typography.headline, color = colors.limelight, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.width(900.dp).wrapContentWidthStart())
                Icon(LoungeIcons.Play, contentDescription = null, tint = colors.dust, modifier = Modifier.size(22.dp))
            }
            spotlight.progress?.let { ProgressBar(it, Modifier.width(420.dp), thickness = 6.dp) }
        }
    }
}

private fun Modifier.wrapContentWidthStart(): Modifier = this.then(
    Modifier.layout { measurable, constraints ->
        val placeable = measurable.measure(constraints.copy(minWidth = 0))
        layout(placeable.width, placeable.height) { placeable.place(0, 0) }
    },
)

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun ExpandedStage(
    stage: Stage,
    spotlight: StageSpotlight?,
    selected: androidx.compose.runtime.IntState,
    idle: Boolean,
    indexed: Boolean,
    stripRequester: (Int) -> FocusRequester,
    stripFocus: () -> FocusRequester,
    playbillFocus: FocusRequester,
    upFocus: FocusRequester,
    onStripFocused: (Int, Tile) -> Unit,
    onStripClick: (Tile) -> Unit,
    onStripLongPress: (Tile, Rect, FocusRequester) -> Unit,
    onStripMoved: () -> Unit,
    onPrimary: (StageSpotlight) -> Unit,
    onSecondary: (StageSpotlight) -> Unit,
    onPlaybillFocusChanged: (Boolean) -> Unit,
    modifier: Modifier,
    interactive: Boolean,
    shown: () -> Float,
    canTakeFocus: () -> Boolean,
) {
    Column(modifier) {
        Row(Modifier.padding(horizontal = Dimens.safeX).height(630.dp)) {
            Box(
                Modifier
                    .width(Dimens.playbillWidth)
                    .fillMaxSize()
                    .onFocusChanged { onPlaybillFocusChanged(it.hasFocus) }
                    // Once for the group itself, once for the buttons inside it: either way Down
                    // reaches the strip's selected tile, which overlaps this box's light room. The
                    // second also keeps the lowered playbill's buttons out of focus.
                    .focusProperties { down = stripFocus(); up = upFocus }
                    .focusGroup()
                    .focusProperties { down = stripFocus(); up = upFocus; if (!canTakeFocus()) canFocus = false },
            ) {
                if (spotlight != null) {
                    Playbill(spotlight, onPrimary, onSecondary, interactive, playbillFocus, shown)
                } else {
                    EmptyPlaybill(indexed)
                }
            }
            Spacer(Modifier.width(48.dp))
            when {
                spotlight == null -> Box(Modifier.size(Dimens.stageArtworkWidth, Dimens.stageArtworkHeight).clip(RoundedCornerShape(Dimens.stageRadius))) {
                    EmptyStageArt(LoungeTheme.colors.houselight)
                }
                spotlight.framed && spotlight.artworkPath != null -> FramedBanner(spotlight, Modifier)
                else -> Unit
            }
        }
        Spacer(Modifier.height(32.dp))
        StageStrip(
            strip = stage.strip,
            selected = selected,
            idle = idle,
            upFocus = if (spotlight != null) playbillFocus else upFocus,
            canTakeFocus = canTakeFocus,
            stripRequester = stripRequester,
            onStripFocused = onStripFocused,
            onStripClick = onStripClick,
            onStripLongPress = onStripLongPress,
            onStripMoved = onStripMoved,
        )
    }
}

/**
 * The strip under the playbill. Nothing here changes when the selection moves, so a move
 * recomposes no tile: each tile's light is a State on the selection. [canTakeFocus] is read when
 * focus searches, so the lowered strip refuses focus without recomposing.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun StageStrip(
    strip: List<Tile>,
    selected: androidx.compose.runtime.IntState,
    idle: Boolean,
    upFocus: FocusRequester,
    canTakeFocus: () -> Boolean,
    stripRequester: (Int) -> FocusRequester,
    onStripFocused: (Int, Tile) -> Unit,
    onStripClick: (Tile) -> Unit,
    onStripLongPress: (Tile, Rect, FocusRequester) -> Unit,
    onStripMoved: () -> Unit,
) {
    val safeXPx = with(LocalDensity.current) { Dimens.safeX.toPx() }
    val stripState = remember { LazyListState(LazyLayoutCacheWindow(ahead = 540.dp, behind = 270.dp), 0, 0) }
    KeepStart(stripState, strip.firstOrNull()?.id)
    CompositionLocalProvider(LocalBringIntoViewSpec provides pivotSpec(safeXPx)) {
        LazyRow(
            state = stripState,
            contentPadding = PaddingValues(start = Dimens.safeX, end = Dimens.safeX, top = Dimens.glowRoom, bottom = Dimens.glowRoom),
            horizontalArrangement = Arrangement.spacedBy(Dimens.gutter),
            modifier = Modifier
                .fillMaxWidth()
                .glowRoom()
                .onPreviewKeyEvent { e ->
                    if (e.type == KeyEventType.KeyDown && (e.key == Key.DirectionLeft || e.key == Key.DirectionRight)) onStripMoved()
                    false
                }
                .focusRestorer()
                .focusGroup()
                // After the group, so it reaches every tile.
                .focusProperties { up = upFocus; if (!canTakeFocus()) canFocus = false },
        ) {
            itemsIndexed(strip, key = { _, t -> t.id }) { index, tile ->
                val lights = remember(index) {
                    derivedStateOf { if (kotlin.math.abs(index - selected.intValue) == 1) Lights.Neighbour else Lights.Rested }
                }
                TileView(
                    tile = tile,
                    kind = tile.kind,
                    size = DpSize(Dimens.stripTileWidth, Dimens.stripTileHeight),
                    lights = lights,
                    idle = idle,
                    rowName = "Stage",
                    focusRequester = stripRequester(index),
                    onFocused = { focused, _ -> if (focused) onStripFocused(index, tile) },
                    onClick = { onStripClick(tile) },
                    onLongPress = { bounds -> onStripLongPress(tile, bounds, stripRequester(index)) },
                    // On the tile itself: a lowered strip's tiles lie below the rows in layout
                    // and a search downward from the last row would otherwise land on one.
                    modifier = Modifier.focusProperties { if (!canTakeFocus()) canFocus = false },
                    chrome = false,
                )
            }
        }
    }
}

/**
 * The Stage change (DESIGN.md §6.3): the outgoing artwork slides 12 px and fades while the incoming
 * one rises from 92 % and fades in. Alpha is applied per draw, with no offscreen layer.
 */
@Composable
private fun StageArtworkCrossfade(spotlight: StageSpotlight?, index: Int, modifier: Modifier) {
    val travel = remember { Travel(index, spotlight?.id) }
    if (spotlight?.id != travel.id) {
        travel.direction = if (index < travel.index) -1 else 1
        travel.index = index
        travel.id = spotlight?.id
    }
    Swap(
        spotlight,
        modifier = modifier,
        rise = 0.dp,
        shiftX = (12 * travel.direction).dp,
        fromScale = 0.92f,
        outFraction = 1f,
        modulateAlpha = true,
        keyOf = { it?.id },
    ) { sp ->
        if (sp != null) StageArtwork(sp, Modifier.fillMaxSize())
    }
}

/** Where the selection was and which way it last moved, for the artwork's slide. */
private class Travel(var index: Int, var id: String?) {
    var direction = 1
}

/**
 * The picture inside the frame. Key art fills it, vignette baked in at index time. A framed
 * spotlight shows its blurred backdrop here, lit by its own colour; its crisp banner is drawn over
 * the frame by the expanded Stage so it is never scaled with the proscenium.
 */
@Composable
private fun StageArtwork(spotlight: StageSpotlight, modifier: Modifier) {
    val colors = LoungeTheme.colors
    val light = LoungeTheme.colors.light(spotlight.lightArgb)
    val path = spotlight.artworkPath
    val framed = spotlight.framed
    val lit = remember(light, colors.velvet) { Brush.linearGradient(listOf(light.copy(alpha = 0.45f), colors.velvet)) }
    Box(modifier.then(if (path == null || framed) Modifier.background(lit) else Modifier)) {
        when {
            path == null -> EmptyStageArt(light)
            framed -> Backdrop(spotlight.backdropPath ?: path, alpha = 0.7f, Modifier.fillMaxSize())
            else -> {
                val litPainter = remember(lit) { BrushPainter(lit) }
                AsyncImage(
                    model = path,
                    contentDescription = null,
                    contentScale = ContentScale.Crop,
                    placeholder = litPainter,
                    error = litPainter,
                    modifier = Modifier.fillMaxSize(),
                )
            }
        }
    }
}

/** A framed spotlight's banner at its native 640×360, lit by a shadow, over the frame's backdrop. */
@Composable
private fun FramedBanner(spotlight: StageSpotlight, modifier: Modifier) {
    val frame = when {
        spotlight.kind == SpotlightKind.APP || spotlight.kind == SpotlightKind.GAME || spotlight.kind == SpotlightKind.DEVICE -> DpSize(640.dp, 360.dp)
        spotlight.artAspect > 0f -> DpSize((630f * spotlight.artAspect).coerceAtMost(1120f).dp, 630.dp)
        else -> DpSize(640.dp, 360.dp)
    }
    Box(modifier.size(Dimens.stageArtworkWidth, Dimens.stageArtworkHeight), contentAlignment = Alignment.Center) {
        AsyncImage(
            model = spotlight.artworkPath,
            contentDescription = null,
            contentScale = ContentScale.Fit,
            modifier = Modifier
                .size(frame)
                .lightGlow(Color.Black, 0.45f, radius = 60.dp, cornerRadius = 20.dp)
                .clip(RoundedCornerShape(20.dp)),
        )
    }
}

/**
 * A pre-blurred 160×90 thumbnail scaled across a large area: a soft backdrop for the price of one
 * tiny texture, with no render-time blur.
 */
@Composable
private fun Backdrop(path: String, alpha: Float, modifier: Modifier) {
    AsyncImage(
        model = path,
        contentDescription = null,
        contentScale = ContentScale.Crop,
        filterQuality = FilterQuality.Medium,
        alpha = alpha,
        modifier = modifier,
    )
}

@Composable
private fun EmptyStageArt(light: Color) {
    Box(
        Modifier.fillMaxSize().drawBehind {
            drawSpill(SpillMask.soft, Offset(size.width * 0.5f, size.height * 0.62f), size.width * 0.55f, size.height * 0.62f, light, 0.35f)
        },
    )
}

@Composable
private fun Playbill(
    spotlight: StageSpotlight,
    onPrimary: (StageSpotlight) -> Unit,
    onSecondary: (StageSpotlight) -> Unit,
    interactive: Boolean,
    primaryFocus: FocusRequester,
    shown: () -> Float,
) {
    val colors = LoungeTheme.colors
    val light = LoungeTheme.colors.light(spotlight.lightArgb)

    val lightState = LocalLightState.current
    DisposableEffect(lightState) { onDispose { lightState.bill = null } }
    Box(Modifier.fillMaxSize()) {
        Box(
            Modifier
                .fillMaxSize()
                .onPlaced { coords ->
                    // The light field draws this light with everything else; hand it the geometry,
                    // and the playbill's own fade so the light goes down with the Stage.
                    if (lightState.fieldDrawsLights) {
                        val origin = coords.positionInRoot()
                        val w = coords.size.width.toFloat()
                        val h = coords.size.height.toFloat()
                        lightState.bill = BillLight(Offset(origin.x + w + 48f, origin.y + h * 0.45f), w * 1.15f, h * 0.45f, light, shown)
                    }
                }
                .drawBehind {
                    // Light from the artwork's edge, as a flat ellipse that reaches zero before the
                    // box's top and bottom so it never shows a clipped edge.
                    if (!lightState.fieldDrawsLights) {
                        drawSpill(
                            SpillMask.soft,
                            center = Offset(size.width + 48f, size.height * 0.45f),
                            rx = size.width * 1.15f,
                            ry = size.height * 0.45f,
                            color = light,
                            alpha = 0.22f * colors.spillScale,
                        )
                    }
                },
        )
        // The source chip sits level with the artwork's top edge: it says where OK will take you.
        PlaybillPart(spotlight) { s ->
            s.source?.let { source ->
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    s.sourceIconPath?.let { icon ->
                        AsyncImage(model = icon, contentDescription = null, contentScale = ContentScale.Fit, modifier = Modifier.size(40.dp).clip(RoundedCornerShape(10.dp)))
                    }
                    Text(source, style = LoungeTheme.typography.meta, color = colors.dust, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
        }
        // Every slot has a fixed height, so a one-line title or a missing line moves nothing else.
        val type = LoungeTheme.typography
        val metaLine = with(LocalDensity.current) { type.meta.lineHeight.toDp() }
        val titleBlock = with(LocalDensity.current) { type.displayXl.lineHeight.toDp() } * 2
        val captionLine = with(LocalDensity.current) { type.caption.lineHeight.toDp() }
        Column(Modifier.fillMaxSize().padding(bottom = 6.dp), verticalArrangement = Arrangement.Bottom) {
            PlaybillPart(spotlight) { s ->
                Column {
                    Box(Modifier.height(metaLine)) {
                        Text(s.category, style = type.meta, color = colors.dust, maxLines = 1)
                    }
                    Spacer(Modifier.height(16.dp))
                    Box(Modifier.height(titleBlock), contentAlignment = Alignment.BottomStart) {
                        Text(s.title, style = type.displayXl, color = colors.limelight, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    }
                }
            }
            Spacer(Modifier.height(16.dp))
            PlaybillPart(spotlight, delayMs = 40) { s ->
                Column {
                    Column(Modifier.height(metaLine * 2 + 4.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        s.lines.take(2).forEachIndexed { i, line ->
                            Text(line, style = type.meta, color = if (i == 0) colors.limelight else colors.dust, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        }
                    }
                    Spacer(Modifier.height(12.dp))
                    Box(Modifier.height(6.dp)) {
                        s.progress?.let { ProgressBar(it, Modifier.width(420.dp), thickness = 6.dp) }
                    }
                }
            }
            Spacer(Modifier.height(24.dp))
            PlaybillPart(spotlight, delayMs = 80) { s ->
                Column {
                    Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                        // Only the live copy answers the playbill's focus requester; a hidden one cannot take focus.
                        val live = LocalSwapSlotActive.current.value
                        LoungeButton(s.primary.label, onClick = { onPrimary(s) }, light = colors.light(s.lightArgb), enabled = interactive && live, focusRequester = if (live) primaryFocus else null)
                        s.secondary?.let { LoungeButton(it.label, onClick = { onSecondary(s) }, kind = ButtonKind.Secondary, enabled = interactive && live) }
                    }
                    Spacer(Modifier.height(16.dp))
                    Box(Modifier.height(captionLine)) {
                        s.explanation?.let { why ->
                            Text(why, style = type.caption, color = colors.dust, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        }
                    }
                }
            }
        }
    }
}

/** One part of the playbill: the old text fades out first, then the new fades in where it sits. */
@Composable
private fun PlaybillPart(spotlight: StageSpotlight, delayMs: Int = 0, content: @Composable (StageSpotlight) -> Unit) =
    Swap(spotlight, delayMs = delayMs, rise = 0.dp, outFraction = TEXT_OUT, inDelayFraction = TEXT_IN_DELAY, keepSlots = true, keyOf = { it.id }, content = content)

/** Nothing on the Stage yet: still indexing, or genuinely nothing to show (DESIGN.md §8). */
@Composable
private fun EmptyPlaybill(indexed: Boolean) {
    val colors = LoungeTheme.colors
    Column(Modifier.fillMaxSize().padding(bottom = 6.dp), verticalArrangement = Arrangement.Bottom) {
        Text("Your home screen", style = LoungeTheme.typography.meta, color = colors.dust)
        Spacer(Modifier.height(16.dp))
        if (indexed) {
            Text("Nothing here yet", style = LoungeTheme.typography.displayXl, color = colors.limelight, maxLines = 2)
            Spacer(Modifier.height(16.dp))
            Text("Install an app and it will show up here.", style = LoungeTheme.typography.meta, color = colors.dust)
        } else {
            Text("Setting up your home screen", style = LoungeTheme.typography.displayXl, color = colors.limelight, maxLines = 2)
            Spacer(Modifier.height(16.dp))
            Text("Looking at what is on this TV.", style = LoungeTheme.typography.meta, color = colors.dust)
        }
    }
}
