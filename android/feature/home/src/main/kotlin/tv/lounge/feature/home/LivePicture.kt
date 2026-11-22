package tv.lounge.feature.home

import androidx.annotation.StringRes
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.focusable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.tv.material3.Text
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withContext
import tv.lounge.core.CoreClient
import tv.lounge.design.Dimens
import tv.lounge.design.LoungeTheme

/** How long a live picture leaves between one fetch starting and the next. */
internal const val LIVE_EVERY_MS = 1_000L

/** How long after the last key a live picture goes on fetching. */
internal const val LIVE_RESTS_AFTER_MS = 5 * 60_000L

/** The widest a live picture is decoded; a larger one is halved until it fits. */
private const val LIVE_MAX_WIDTH = 1920

/** How much to shrink a picture [width] wide by, as the power of two a decoder takes. */
internal fun liveSampleSize(width: Int, max: Int = LIVE_MAX_WIDTH): Int {
    var by = 1
    while (width / by > max) by *= 2
    return by
}

/** What the screen says under the title, if anything. */
@StringRes
internal fun liveNote(hasPicture: Boolean, resting: Boolean, misses: Int): Int? = when {
    resting -> R.string.live_picture_resting
    misses >= 3 && hasPicture -> R.string.live_picture_stopped
    misses >= 3 -> R.string.live_picture_not_arriving
    !hasPicture -> R.string.live_picture_waiting
    else -> null
}

/** Decodes pictures of one size into two bitmaps turn about, so a new picture every second leaves nothing to collect. */
private class LiveFrames {
    private val spare = arrayOfNulls<Bitmap>(2)
    private var next = 0

    fun decode(bytes: ByteArray): Bitmap? {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        if (bounds.outWidth <= 0) return null
        val options = BitmapFactory.Options().apply {
            inSampleSize = liveSampleSize(bounds.outWidth)
            inMutable = true
            inBitmap = spare[next]
        }
        val decoded = try {
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options)
        } catch (_: IllegalArgumentException) {
            // The picture has changed size, so the old bitmap cannot hold it.
            options.inBitmap = null
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options)
        } ?: return null
        spare[next] = decoded
        next = 1 - next
        return decoded
    }
}

/**
 * A plugin's still picture, full screen, fetched again about once a second. It rests after five
 * minutes without a key and Back closes it. When the address stops answering, the core asks the
 * plugin for a new one.
 */
@Composable
fun LivePictureScreen(
    core: CoreClient,
    plugin: String,
    title: String,
    url: String,
    visibleState: MutableTransitionState<Boolean>,
    onClose: () -> Unit,
) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    var frame by remember(plugin, url) { mutableStateOf<ImageBitmap?>(null) }
    var misses by remember(plugin, url) { mutableStateOf(0) }
    var resting by remember(plugin, url) { mutableStateOf(false) }
    var lastKeyAt by remember(plugin, url) { mutableLongStateOf(System.currentTimeMillis()) }
    val focus = remember { FocusRequester() }

    WhileStarted(plugin, url) {
        val frames = LiveFrames()
        var address = url
        while (true) {
            if (System.currentTimeMillis() - lastKeyAt >= LIVE_RESTS_AFTER_MS) {
                resting = true
                snapshotFlow { lastKeyAt }.first { System.currentTimeMillis() - it < LIVE_RESTS_AFTER_MS }
                resting = false
            }
            val started = System.currentTimeMillis()
            val picture = withContext(Dispatchers.IO) { core.pluginPicture(plugin, address).mapCatching { frames.decode(it) ?: error("not a picture") } }
            picture.onSuccess {
                frame = it.asImageBitmap()
                misses = 0
            }.onFailure {
                misses++
                // An address is good for a few minutes; the plugin gives the one that works now.
                withContext(Dispatchers.IO) { core.pluginLive(plugin, address) }.onSuccess { fresh -> address = fresh }
            }
            val wait = if (misses > 0) (LIVE_EVERY_MS * misses).coerceAtMost(8_000L) else LIVE_EVERY_MS
            delay((wait - (System.currentTimeMillis() - started)).coerceAtLeast(100L))
        }
    }

    AnimatedVisibility(visibleState, enter = fadeIn(motion.curtain()), exit = fadeOut(motion.settle())) {
        Box(
            Modifier
                .fillMaxSize()
                .background(Color.Black)
                .semantics { paneTitle = title }
                .onPreviewKeyEvent { e ->
                    when {
                        e.key == Key.VolumeUp || e.key == Key.VolumeDown || e.key == Key.VolumeMute -> false
                        e.isBack() -> {
                            if (e.type == KeyEventType.KeyUp) onClose()
                            true
                        }
                        else -> {
                            if (e.type == KeyEventType.KeyDown) lastKeyAt = System.currentTimeMillis()
                            true
                        }
                    }
                }
                .focusRequester(focus)
                .focusable(),
        ) {
            frame?.let { Image(bitmap = it, contentDescription = null, contentScale = ContentScale.Fit, modifier = Modifier.fillMaxSize()) }
            val note = liveNote(frame != null, resting, misses)
            Column(
                Modifier
                    .align(Alignment.BottomStart)
                    .fillMaxWidth()
                    .background(Brush.verticalGradient(listOf(Color.Transparent, Color.Black.copy(alpha = 0.7f))))
                    .padding(start = Dimens.safeX, end = Dimens.safeX, top = 72.dp, bottom = Dimens.safeY),
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                Text(title, style = LoungeTheme.typography.headline, color = colors.limelight, maxLines = 1, overflow = TextOverflow.Ellipsis)
                if (note != null) Text(stringResource(note), style = LoungeTheme.typography.meta, color = colors.dust, modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite })
            }
        }
    }
    LaunchedEffect(Unit) { focus.requestWhenReady() }
}
