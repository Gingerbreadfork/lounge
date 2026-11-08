package tv.lounge.collector.tvprovider

import android.content.Context
import android.content.pm.PackageManager
import android.database.ContentObserver
import android.database.Cursor
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
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.channels.Channel as WorkChannel
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.buffer
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.channelFlow
import kotlinx.coroutines.flow.flowOn
import kotlinx.coroutines.launch
import kotlinx.coroutines.joinAll
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import tv.lounge.core.ProgramKind
import tv.lounge.core.ProgramRecord
import tv.lounge.core.WatchNextType

/**
 * Reads Watch Next, other apps' preview channels and programmes, and TIF channels with their
 * schedule under READ_TV_LISTINGS. Artwork is copied to local files. What ends up on screen is the
 * core's decision.
 */
class TvProviderCollector internal constructor(private val context: Context, private val artwork: TvArtwork) {
    constructor(context: Context) : this(context, CachedTvArtwork(context))

    private val resolver = context.contentResolver
    /** Provider reads are serialized; downloads never hold this mutex. */
    private val collecting = Mutex()

    fun hasPermission(): Boolean =
        ContextCompat.checkSelfPermission(context, TvContractCompat.PERMISSION_READ_TV_LISTINGS) == PackageManager.PERMISSION_GRANTED

    /**
     * Everything the provider has, using only artwork already on disk. Schedule entries cover
     * one hour back to [scheduleHours] ahead. [snapshots] fills in missing artwork separately.
     */
    suspend fun snapshot(nowMs: Long = System.currentTimeMillis(), scheduleHours: Int = 6): List<ProgramRecord> = withContext(Dispatchers.IO) {
        collecting.withLock {
            val drafts = collect(nowMs, scheduleHours)
            val paths = cachedPaths(drafts)
            drafts.map { it.toRecord(paths) }
        }
    }

    /**
     * Each request carries the preview channels enabled in Lounge. Metadata and cached art are
     * published first; four workers then fill in missing artwork and republish in batches. A
     * late image only ever dresses the latest listings.
     */
    fun snapshots(requests: Flow<Set<Long>>): Flow<List<ProgramRecord>> = channelFlow {
        val state = Mutex()
        var drafts = emptyList<Draft>()
        var paths = emptyMap<String, String>().toMutableMap()
        var previous: List<ProgramRecord>? = null
        val pending = ArrayDeque<String>()
        val inFlight = mutableSetOf<String>()
        val wakeWorkers = WorkChannel<Unit>(4)
        val artChanged = WorkChannel<Unit>(WorkChannel.CONFLATED)

        // Called under state so an older artwork publication cannot overtake newer metadata.
        suspend fun publish() {
            val records = drafts.map { it.toRecord(paths) }
            if (records != previous) {
                send(records)
                previous = records
            }
        }

        launch {
            for (change in artChanged) {
                delay(500)
                // Include all completions during this interval in a single publication.
                while (artChanged.tryReceive().isSuccess) { }
                state.withLock { publish() }
            }
        }
        val workers = List(4) {
            launch {
                for (wake in wakeWorkers) {
                    while (true) {
                        val uri = state.withLock {
                            pending.removeFirstOrNull()?.also { inFlight += it }
                        } ?: break
                        val path = try {
                            artwork.fetch(uri)
                        } catch (e: CancellationException) {
                            throw e
                        } catch (e: Exception) {
                            Log.w(TAG, "artwork download failed", e)
                            null
                        }
                        state.withLock {
                            inFlight -= uri
                            if (path != null && path != paths[uri] && drafts.any { uri in it.artUris() }) {
                                paths[uri] = path
                                artChanged.trySend(Unit)
                            }
                        }
                    }
                }
            }
        }
        requests.collect { enabledChannels ->
            try {
                val latest = collecting.withLock { collect(System.currentTimeMillis(), 6) }
                state.withLock {
                    drafts = latest
                    paths = cachedPaths(drafts).toMutableMap()
                    pending.clear()
                    pending.addAll(drafts.asSequence()
                        .filter { it.kind != ProgramKind.PREVIEW || it.channelId in enabledChannels }
                        .sortedBy { if (it.kind == ProgramKind.WATCH_NEXT) 0 else 1 }
                        .flatMap { it.artUris() }
                        .distinct()
                        .filter { it !in inFlight && artwork.cached(it, allowStale = false) == null }
                        .toList())
                    publish()
                    artwork.prune(paths.values)
                }
                repeat(4) { wakeWorkers.trySend(Unit) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                Log.w(TAG, "TV listings refresh", e)
            }
        }
        wakeWorkers.close()
        workers.joinAll()
        artChanged.close()
    }.buffer(WorkChannel.CONFLATED).flowOn(Dispatchers.IO)

    private fun cachedPaths(drafts: List<Draft>): Map<String, String> =
        drafts.flatMap { it.artUris() }.distinct().mapNotNull { uri -> artwork.cached(uri)?.let { uri to it } }.toMap()

    private fun collect(nowMs: Long, scheduleHours: Int): List<Draft> {
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
        Log.d(TAG, "TV listings: ${drafts.groupingBy { it.kind }.eachCount()}")
        return drafts
    }

    /**
     * Refresh after registering, as well as on changes. The home stops observing while a player
     * is in front, so each new subscription must catch writes made while it was away. Register
     * first so a write between the refresh and observer setup cannot be missed.
     */
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
        if (hasPermission()) {
            uris.forEach { uri ->
                runCatching { resolver.registerContentObserver(uri, true, observer) }
                    .onFailure { Log.w(TAG, "observe $uri", it) }
            }
            trySend(Unit)
        }
        awaitClose { runCatching { resolver.unregisterContentObserver(observer) } }
    }

    private fun watchNext(): List<Draft> {
        val out = ArrayList<Draft>()
        val uri = TvContractCompat.WatchNextPrograms.CONTENT_URI
        resolver.query(uri, WatchNextProgram.PROJECTION, null, null, null)?.use { c ->
            c.readRows(uri) {
                val p = WatchNextProgram.fromCursor(c)
                if (!p.isBrowsable) return@readRows
                val title = p.title?.takeIf { it.isNotBlank() } ?: return@readRows
                out += Draft(
                    id = watchNextId(p),
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

    /**
     * The provider's row id changes whenever an app republishes an entry, so hiding one would not
     * stick; the app's own id for it does not, when the app gives one.
     */
    private fun watchNextId(p: WatchNextProgram): String {
        val own = p.internalProviderId?.takeIf { it.isNotBlank() }
        val pkg = p.packageName?.takeIf { it.isNotBlank() }
        return if (own != null && pkg != null) "wn:$pkg:$own" else "wn:${p.id}"
    }

    private fun channels(): List<Channel> {
        val out = ArrayList<Channel>()
        val uri = TvContractCompat.Channels.CONTENT_URI
        resolver.query(uri, Channel.PROJECTION, null, null, null)?.use { c ->
            c.readRows(uri) {
                val ch = Channel.fromCursor(c)
                // Preview rows are opt-in through Lounge's own channel settings. The system's
                // browsable flag reflects the stock launcher's choices, not Lounge's layout.
                if (ch.type == TvContractCompat.Channels.TYPE_PREVIEW || ch.isBrowsable) out += ch
            }
        }
        return out
    }

    private fun previewPrograms(ch: Channel): List<Draft> {
        val out = ArrayList<Draft>()
        val uri = TvContractCompat.buildPreviewProgramsUriForChannel(ch.id)
        resolver.query(uri, PreviewProgram.PROJECTION, null, null, null)?.use { c ->
            c.readRows(uri) {
                val p = PreviewProgram.fromCursor(c)
                if (!p.isBrowsable) return@readRows
                val title = p.title?.takeIf { it.isNotBlank() } ?: return@readRows
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
            c.readRows(uri) {
                val p = Program.fromCursor(c)
                val title = p.title?.takeIf { it.isNotBlank() } ?: return@readRows
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

    /** One app's malformed metadata must not discard the other apps' rows in this table. */
    private inline fun Cursor.readRows(uri: Uri, read: () -> Unit) {
        while (moveToNext()) {
            try {
                read()
            } catch (e: RuntimeException) {
                Log.w(TAG, "Skipping malformed row $position in $uri", e)
            }
        }
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
        fun artUris(): List<String> = listOfNotNull(posterUri, thumbnailUri, logoUri)

        fun toRecord(paths: Map<String, String>) = toRecord(posterUri?.let(paths::get), thumbnailUri?.let(paths::get), logoUri?.let(paths::get))

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
