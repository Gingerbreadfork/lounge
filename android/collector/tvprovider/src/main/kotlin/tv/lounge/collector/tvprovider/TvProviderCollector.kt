package tv.lounge.collector.tvprovider

import android.content.Context
import android.content.pm.PackageManager
import android.database.ContentObserver
import android.media.tv.TvContract
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.core.content.ContextCompat
import androidx.tvprovider.media.tv.Channel
import androidx.tvprovider.media.tv.PreviewProgram
import androidx.tvprovider.media.tv.Program
import androidx.tvprovider.media.tv.TvContractCompat
import androidx.tvprovider.media.tv.WatchNextProgram
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import tv.lounge.core.ProgramKind
import tv.lounge.core.ProgramRecord
import tv.lounge.core.WatchNextType
import java.io.File

/**
 * Reads Watch Next, other apps' preview channels and programmes, and TIF channels with their
 * schedule under READ_TV_LISTINGS. Artwork is copied to local files. What ends up on screen is the
 * core's decision.
 */
class TvProviderCollector(private val context: Context) {
    private val resolver = context.contentResolver
    private val artwork = ArtworkFetcher(resolver, File(context.cacheDir, "tvart")) { authority ->
        runCatching { context.packageManager.resolveContentProvider(authority, 0)?.packageName == context.packageName }.getOrDefault(true)
    }
    /** One snapshot at a time: they share the artwork cache, and pruning must not see another's downloads. */
    private val collecting = Mutex()

    fun hasPermission(): Boolean =
        ContextCompat.checkSelfPermission(context, TvContractCompat.PERMISSION_READ_TV_LISTINGS) == PackageManager.PERMISSION_GRANTED

    /**
     * Everything the provider has, with artwork fetched. Schedule entries cover one hour back to
     * [scheduleHours] ahead.
     */
    suspend fun snapshot(nowMs: Long = System.currentTimeMillis(), scheduleHours: Int = 6): List<ProgramRecord> = withContext(Dispatchers.IO) {
        collecting.withLock { collect(nowMs, scheduleHours) }
    }

    private suspend fun collect(nowMs: Long, scheduleHours: Int): List<ProgramRecord> {
        if (!hasPermission()) return emptyList()
        val drafts = ArrayList<Draft>()
        runCatching { drafts += watchNext() }.onFailure { Log.w(TAG, "watch next", it) }
        val channels = runCatching { channels() }.getOrElse { Log.w(TAG, "channels", it); emptyList() }
        for (ch in channels) {
            if (ch.type == TvContractCompat.Channels.TYPE_PREVIEW) {
                runCatching { drafts += previewPrograms(ch) }.onFailure { Log.w(TAG, "preview ${ch.id}", it) }
            } else {
                drafts += ch.toDraft()
                runCatching { drafts += schedule(ch, nowMs - 3_600_000L, nowMs + scheduleHours * 3_600_000L) }
                    .onFailure { Log.w(TAG, "schedule ${ch.id}", it) }
            }
        }
        val posters = artwork.fetchAll(drafts.map { it.posterUri })
        val thumbnails = artwork.fetchAll(drafts.map { it.thumbnailUri })
        val logos = artwork.fetchAll(drafts.map { it.logoUri })
        artwork.prune(posters.filterNotNull() + thumbnails.filterNotNull() + logos.filterNotNull())
        return drafts.mapIndexed { i, d -> d.toRecord(posters[i], thumbnails[i], logos[i]) }
    }

    /** Emits when any TV provider table changes; callers should debounce. */
    fun changes(): Flow<Unit> = callbackFlow {
        val observer = object : ContentObserver(Handler(Looper.getMainLooper())) {
            override fun onChange(selfChange: Boolean, uri: Uri?) {
                trySend(Unit)
            }
        }
        val uris = listOf(
            TvContractCompat.WatchNextPrograms.CONTENT_URI,
            TvContractCompat.Channels.CONTENT_URI,
            TvContractCompat.PreviewPrograms.CONTENT_URI,
            TvContractCompat.Programs.CONTENT_URI,
        )
        if (hasPermission()) uris.forEach { runCatching { resolver.registerContentObserver(it, true, observer) } }
        awaitClose { runCatching { resolver.unregisterContentObserver(observer) } }
    }

    private fun watchNext(): List<Draft> {
        val out = ArrayList<Draft>()
        resolver.query(TvContractCompat.WatchNextPrograms.CONTENT_URI, WatchNextProgram.PROJECTION, null, null, null)?.use { c ->
            while (c.moveToNext()) {
                val p = WatchNextProgram.fromCursor(c)
                if (!p.isBrowsable) continue
                val title = p.title?.takeIf { it.isNotBlank() } ?: continue
                out += Draft(
                    id = "wn:${p.id}",
                    kind = ProgramKind.WATCH_NEXT,
                    pkg = p.packageName ?: "",
                    title = title,
                    subtitle = p.episodeTitle?.takeIf { it.isNotBlank() } ?: p.description?.takeIf { it.isNotBlank() },
                    season = p.seasonNumber?.takeIf { it.isNotBlank() },
                    episode = p.episodeNumber?.takeIf { it.isNotBlank() },
                    watchNextType = when (p.watchNextType) {
                        TvContractCompat.WatchNextPrograms.WATCH_NEXT_TYPE_CONTINUE -> WatchNextType.CONTINUE
                        TvContractCompat.WatchNextPrograms.WATCH_NEXT_TYPE_NEXT -> WatchNextType.NEXT
                        TvContractCompat.WatchNextPrograms.WATCH_NEXT_TYPE_NEW -> WatchNextType.NEW
                        TvContractCompat.WatchNextPrograms.WATCH_NEXT_TYPE_WATCHLIST -> WatchNextType.WATCHLIST
                        else -> WatchNextType.UNKNOWN
                    },
                    positionMs = p.lastPlaybackPositionMillis.takeIf { it > 0 }?.toLong(),
                    durationMs = p.durationMillis.takeIf { it > 0 }?.toLong(),
                    engagedMs = p.lastEngagementTimeUtcMillis.takeIf { it > 0 },
                    posterUri = p.posterArtUri?.toString(),
                    posterIsPortrait = isPortrait(p.posterArtAspectRatio),
                    thumbnailUri = p.thumbnailUri?.toString(),
                    logoUri = p.logoUri?.toString(),
                    intentUri = p.intentUri?.toString(),
                )
            }
        }
        return out
    }

    private fun channels(): List<Channel> {
        val out = ArrayList<Channel>()
        resolver.query(TvContractCompat.Channels.CONTENT_URI, Channel.PROJECTION, null, null, null)?.use { c ->
            while (c.moveToNext()) {
                val ch = Channel.fromCursor(c)
                if (ch.isBrowsable) out += ch
            }
        }
        return out
    }

    private fun previewPrograms(ch: Channel): List<Draft> {
        val out = ArrayList<Draft>()
        val uri = TvContractCompat.buildPreviewProgramsUriForChannel(ch.id)
        resolver.query(uri, PreviewProgram.PROJECTION, null, null, null)?.use { c ->
            while (c.moveToNext()) {
                val p = PreviewProgram.fromCursor(c)
                if (!p.isBrowsable) continue
                val title = p.title?.takeIf { it.isNotBlank() } ?: continue
                out += Draft(
                    id = "preview:${p.id}",
                    kind = ProgramKind.PREVIEW,
                    pkg = p.packageName ?: ch.packageName ?: "",
                    title = title,
                    subtitle = p.episodeTitle?.takeIf { it.isNotBlank() } ?: p.description?.takeIf { it.isNotBlank() },
                    season = p.seasonNumber?.takeIf { it.isNotBlank() },
                    episode = p.episodeNumber?.takeIf { it.isNotBlank() },
                    watchNextType = WatchNextType.UNKNOWN,
                    positionMs = p.lastPlaybackPositionMillis.takeIf { it > 0 }?.toLong(),
                    durationMs = p.durationMillis.takeIf { it > 0 }?.toLong(),
                    engagedMs = null,
                    posterUri = p.posterArtUri?.toString(),
                    posterIsPortrait = isPortrait(p.posterArtAspectRatio),
                    thumbnailUri = p.thumbnailUri?.toString(),
                    logoUri = p.logoUri?.toString(),
                    intentUri = p.intentUri?.toString(),
                    channelId = ch.id,
                    channelName = ch.displayName,
                    weight = p.weight,
                )
            }
        }
        return out
    }

    private fun Channel.toDraft(): Draft = Draft(
        id = "chan:$id",
        kind = ProgramKind.CHANNEL,
        pkg = packageName ?: "",
        title = displayName?.takeIf { it.isNotBlank() } ?: displayNumber ?: "Channel $id",
        subtitle = description?.takeIf { it.isNotBlank() },
        season = null,
        episode = null,
        watchNextType = WatchNextType.UNKNOWN,
        positionMs = null,
        durationMs = null,
        engagedMs = null,
        posterUri = null,
        posterIsPortrait = false,
        thumbnailUri = null,
        logoUri = TvContract.buildChannelLogoUri(id).toString(),
        intentUri = TvContract.buildChannelUri(id).toString(),
        channelId = id,
        channelName = displayName,
        channelNumber = displayNumber,
        inputId = inputId,
    )

    private fun schedule(ch: Channel, fromMs: Long, toMs: Long): List<Draft> {
        val out = ArrayList<Draft>()
        val uri = TvContract.buildProgramsUriForChannel(ch.id, fromMs, toMs)
        resolver.query(uri, Program.PROJECTION, null, null, null)?.use { c ->
            while (c.moveToNext()) {
                val p = Program.fromCursor(c)
                val title = p.title?.takeIf { it.isNotBlank() } ?: continue
                out += Draft(
                    id = "prog:${p.id}",
                    kind = ProgramKind.SCHEDULE,
                    pkg = ch.packageName ?: "",
                    title = title,
                    subtitle = p.episodeTitle?.takeIf { it.isNotBlank() } ?: p.description?.takeIf { it.isNotBlank() },
                    season = p.seasonNumber?.takeIf { it.isNotBlank() },
                    episode = p.episodeNumber?.takeIf { it.isNotBlank() },
                    watchNextType = WatchNextType.UNKNOWN,
                    positionMs = null,
                    durationMs = (p.endTimeUtcMillis - p.startTimeUtcMillis).takeIf { it > 0 },
                    engagedMs = null,
                    posterUri = p.posterArtUri?.toString(),
                    posterIsPortrait = false,
                    thumbnailUri = p.thumbnailUri?.toString(),
                    logoUri = null,
                    intentUri = TvContract.buildChannelUri(ch.id).toString(),
                    channelId = ch.id,
                    channelName = ch.displayName,
                    channelNumber = ch.displayNumber,
                    inputId = ch.inputId,
                    startMs = p.startTimeUtcMillis,
                    endMs = p.endTimeUtcMillis,
                )
            }
        }
        return out
    }

    private fun isPortrait(aspect: Int): Boolean = when (aspect) {
        TvContractCompat.PreviewPrograms.ASPECT_RATIO_2_3,
        TvContractCompat.PreviewPrograms.ASPECT_RATIO_1_1,
        TvContractCompat.PreviewPrograms.ASPECT_RATIO_MOVIE_POSTER,
        -> true
        else -> false
    }

    /** A record before its artwork has been fetched. */
    private class Draft(
        val id: String,
        val kind: ProgramKind,
        val pkg: String,
        val title: String,
        val subtitle: String?,
        val season: String?,
        val episode: String?,
        val watchNextType: WatchNextType,
        val positionMs: Long?,
        val durationMs: Long?,
        val engagedMs: Long?,
        val posterUri: String?,
        val posterIsPortrait: Boolean,
        val thumbnailUri: String?,
        val logoUri: String?,
        val intentUri: String?,
        val channelId: Long? = null,
        val channelName: String? = null,
        val channelNumber: String? = null,
        val inputId: String? = null,
        val startMs: Long? = null,
        val endMs: Long? = null,
        val weight: Int = 0,
    ) {
        fun toRecord(posterPath: String?, thumbnailPath: String?, logoPath: String?) = ProgramRecord(
            id = id,
            kind = kind,
            `package` = pkg,
            title = title,
            subtitle = subtitle,
            season = season,
            episode = episode,
            watchNextType = watchNextType,
            positionMs = positionMs,
            durationMs = durationMs,
            engagedMs = engagedMs,
            posterPath = posterPath,
            posterIsPortrait = posterIsPortrait,
            thumbnailPath = thumbnailPath,
            logoPath = logoPath,
            intentUri = intentUri,
            channelId = channelId,
            channelName = channelName,
            channelNumber = channelNumber,
            inputId = inputId,
            startMs = startMs,
            endMs = endMs,
            weight = weight,
        )
    }

    private companion object {
        const val TAG = "LoungeTv"
    }
}
