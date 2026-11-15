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
 * core's decision. A table is read again when the provider says it changed; the rest is kept
 * from the read before.
 */
class TvProviderCollector internal constructor(private val context: Context, private val artwork: TvArtwork) {
    constructor(context: Context) : this(context, CachedTvArtwork(context))

    private val resolver = context.contentResolver
    /** Provider reads are serialized; downloads never hold this mutex. It also guards [kept]. */
    private val collecting = Mutex()
    private val kept = Listings()
    /** The tables the provider has said changed since they were last read. */
    private val changed = java.util.EnumSet.noneOf(Table::class.java)

    fun hasPermission(): Boolean =
        ContextCompat.checkSelfPermission(context, TvContractCompat.PERMISSION_READ_TV_LISTINGS) == PackageManager.PERMISSION_GRANTED

    /**
     * Everything the provider has, using only artwork already on disk. Schedule entries cover
     * one hour back to [scheduleHours] ahead. [snapshots] fills in missing artwork separately.
     */
    suspend fun snapshot(nowMs: Long = System.currentTimeMillis(), scheduleHours: Int = 6): List<ProgramRecord> = withContext(Dispatchers.IO) {
        collecting.withLock {
            val drafts = collect(nowMs, scheduleHours, everything = true)
            val paths = cachedPaths(drafts)
            drafts.map { it.toRecord(paths) }
        }
    }

    /**
     * Each request carries the preview channels enabled in Lounge, and reads again the tables
     * [changes] has noted since the last read, or every table when it has noted none. Metadata and
     * cached art are published first; four workers then fill in missing artwork and republish in
     * batches. A late image only ever dresses the latest listings.
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

    /**
     * The listings, reading from the provider only the tables noted as changed, or all of them
     * when [everything], when none is noted or on the first read. The schedule is also read again
     * once its window has moved on. A table that fails to read keeps what it held and is read
     * again next time. Called with [collecting] held.
     */
    private fun collect(nowMs: Long, scheduleHours: Int, everything: Boolean = false): List<Draft> {
        if (!hasPermission()) {
            kept.clear()
            return emptyList()
        }
        val noted = synchronized(changed) { changed.toSet().also { changed.clear() } }
        val all = everything || noted.isEmpty() || !kept.read
        val failed = java.util.EnumSet.noneOf(Table::class.java)
        fun read(table: Table, block: () -> Unit) {
            try {
                block()
            } catch (e: RuntimeException) {
                Log.w(TAG, "reading $table", e)
                failed += table
            }
        }
        if (all || Table.WatchNext in noted) read(Table.WatchNext) { kept.watchNext = watchNext() }
        // The channels decide which previews and which schedule there are.
        val channels = all || Table.Channels in noted
        if (channels) read(Table.Channels) { kept.channels = channels() }
        if (channels || Table.Previews in noted) {
            // One channel that will not read does not cost the others their programmes.
            kept.previews = kept.channels.filter { it.isPreview() }.flatMap { ch ->
                runCatching { previewPrograms(ch) }.onFailure { Log.w(TAG, "preview ${ch.id}", it) }.getOrDefault(emptyList())
            }
        }
        if (channels || Table.Schedule in noted || nowMs - kept.scheduleReadAt > SCHEDULE_KEPT_MS) {
            read(Table.Schedule) {
                kept.schedule = schedule(kept.channels.filterNot { it.isPreview() }, nowMs - 3_600_000L, nowMs + scheduleHours * 3_600_000L)
                kept.scheduleReadAt = nowMs
            }
        }
        kept.read = true
        if (failed.isNotEmpty()) synchronized(changed) { changed += failed }
        val drafts = kept.watchNext + kept.previews + kept.channels.filterNot { it.isPreview() }.map { it.toDraft() } + kept.schedule
        Log.d(TAG, "TV listings: ${drafts.groupingBy { it.kind }.eachCount()}, read ${if (all) "in full" else noted.toString()}")
        return drafts
    }

    private fun Channel.isPreview(): Boolean = type == TvContractCompat.Channels.TYPE_PREVIEW

    /** Notes the table a changed row belongs to; a change that names no row is a change to them all. */
    internal fun note(uri: Uri?) {
        val table = when (uri?.pathSegments?.firstOrNull()) {
            "watch_next_program" -> Table.WatchNext
            "channel" -> Table.Channels
            "preview_program" -> Table.Previews
            "program" -> Table.Schedule
            else -> null
        }
        synchronized(changed) { if (table != null) changed += table else changed += Table.entries }
    }

    /**
     * Emits once on registering and then at each change, noting which table changed so the next
     * read takes only that one. Collected for as long as the access is granted, in front or not,
     * so a write made while a player was in front is known on the way back. It registers first so
     * a write between the refresh and observer setup cannot be missed.
     */
    fun changes(): Flow<Unit> = callbackFlow {
        val observer = object : ContentObserver(Handler(Looper.getMainLooper())) {
            override fun onChange(selfChange: Boolean, uri: Uri?) {
                note(uri)
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
            // Nothing was watching until now, so anything may have changed.
            note(null)
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

    /**
     * The schedule of [channels] between [fromMs] and [toMs], in one query for them all. A
     * provider that will not answer that is asked a channel at a time.
     */
    private fun schedule(channels: List<Channel>, fromMs: Long, toMs: Long): List<Draft> {
        if (channels.isEmpty()) return emptyList()
        val window = TvContract.Programs.CONTENT_URI.buildUpon()
            .appendQueryParameter(PARAM_START_TIME, fromMs.toString())
            .appendQueryParameter(PARAM_END_TIME, toMs.toString())
            .build()
        return try {
            schedule(window, channels.associateBy { it.id }, fromMs, toMs)
        } catch (e: RuntimeException) {
            Log.w(TAG, "schedule in one query", e)
            channels.flatMap { ch ->
                runCatching { schedule(TvContract.buildProgramsUriForChannel(ch.id, fromMs, toMs), mapOf(ch.id to ch), fromMs, toMs) }
                    .onFailure { Log.w(TAG, "schedule ${ch.id}", it) }
                    .getOrDefault(emptyList())
            }
        }
    }

    /** The programmes [uri] lists that air on one of [channels] between [fromMs] and [toMs]. */
    private fun schedule(uri: Uri, channels: Map<Long, Channel>, fromMs: Long, toMs: Long): List<Draft> {
        val out = ArrayList<Draft>()
        resolver.query(uri, Program.PROJECTION, null, null, null)?.use { c ->
            c.readRows(uri) {
                val p = Program.fromCursor(c)
                val ch = channels[p.channelId] ?: return@readRows
                // The provider is asked for this window; one that ignores the request is held to it here.
                if (p.endTimeUtcMillis < fromMs || p.startTimeUtcMillis > toMs) return@readRows
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

    /** The provider's tables, as far as a change to one means reading it again. */
    private enum class Table { WatchNext, Channels, Previews, Schedule }

    /** What was last read from each table. */
    private class Listings {
        var watchNext: List<Draft> = emptyList()
        var channels: List<Channel> = emptyList()
        var previews: List<Draft> = emptyList()
        var schedule: List<Draft> = emptyList()
        var scheduleReadAt = 0L
        /** False until the first read, and again once the access is taken away. */
        var read = false

        fun clear() {
            watchNext = emptyList()
            channels = emptyList()
            previews = emptyList()
            schedule = emptyList()
            scheduleReadAt = 0L
            read = false
        }
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
        /** The provider's query parameters for a window of programmes. */
        const val PARAM_START_TIME = "start_time"
        const val PARAM_END_TIME = "end_time"
        /** How long a schedule is kept before its window is read again. */
        const val SCHEDULE_KEPT_MS = 15 * 60_000L
    }
}
