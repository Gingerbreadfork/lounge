package tv.lounge.collector.tvprovider

import android.content.ContentProvider
import android.content.ContentResolver
import android.content.ContentValues
import android.content.ContextWrapper
import android.content.pm.PackageManager
import android.database.Cursor
import android.database.MatrixCursor
import android.net.Uri
import android.util.Log
import androidx.test.filters.SdkSuppress
import androidx.test.platform.app.InstrumentationRegistry
import androidx.tvprovider.media.tv.TvContractCompat
import androidx.tvprovider.media.tv.WatchNextProgram
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.produceIn
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.channels.Channel as TestChannel
import kotlinx.coroutines.channels.ReceiveChannel
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.*
import org.junit.Test
import tv.lounge.core.ProgramKind
import tv.lounge.core.ProgramRecord
import tv.lounge.core.WatchNextType

@SdkSuppress(minSdkVersion = 29)
class TvProviderCollectorTest {
    private val context get() = InstrumentationRegistry.getInstrumentation().targetContext

    @Test fun usesTheSystemProvider() {
        assertEquals("content://android.media.tv/watch_next_program", TvContractCompat.WatchNextPrograms.CONTENT_URI.toString())
        assertEquals("android.media.tv", TvContractCompat.Channels.CONTENT_URI.authority)
        assertEquals("android.media.tv", TvContractCompat.PreviewPrograms.CONTENT_URI.authority)
        assertEquals("android.media.tv", TvContractCompat.Programs.CONTENT_URI.authority)
    }

    @Test fun readsPublishedResumeAndSkipsHiddenOrUntitledRows() = runBlocking {
        val collector = fixture(listOf(row(7), row(8, browsable = 0), row(9, title = " ")))
        val records = collector.snapshot()
        assertEquals(1, records.size)
        val record = records.single()
        assertEquals("wn:com.netflix.ninja:episode-1", record.id)
        assertEquals(ProgramKind.WATCH_NEXT, record.kind)
        assertEquals(WatchNextType.CONTINUE, record.watchNextType)
        assertEquals("com.netflix.ninja", record.`package`)
        assertEquals(30_000L, record.positionMs)
        assertEquals(120_000L, record.durationMs)
        assertEquals("nflx://episode-1", record.intentUri)
    }

    @Test fun republishingKeepsTheStableId() = runBlocking {
        assertEquals(fixture(listOf(row(7))).snapshot().single().id, fixture(listOf(row(99))).snapshot().single().id)
    }

    @Test fun malformedEntryDoesNotDiscardOtherAppsEntries() = runBlocking {
        val bad = row(8).apply { this[6] = "invalid position" }
        val good = row(9).apply { this[1] = "org.example.player" }
        val records = fixture(listOf(row(7), bad, good)).snapshot()
        assertEquals(listOf("com.netflix.ninja", "org.example.player"), records.map { it.`package` })
    }

    @Test fun refreshesOnEverySubscriptionWithoutAProviderNotification() = runBlocking {
        val collector = fixture(emptyList())
        // Mirrors STARTED -> STOPPED -> STARTED when the viewer briefly opens Netflix.
        repeat(2) { withTimeout(3_000) { collector.changes().first() } }
    }

    @Test fun discoversPreviewChannelsEvenWhenHiddenByTheStockLauncher() = runBlocking {
        val channels = listOf(
            arrayOf<Any?>(33L, "com.netflix.ninja", TvContractCompat.Channels.TYPE_PREVIEW, 0, "Recommendations"),
            arrayOf<Any?>(34L, "org.example.tuner", TvContractCompat.Channels.TYPE_OTHER, 0, "Hidden tuner"),
        )
        val records = fixture(emptyList(), channels, listOf(row(10), row(11, browsable = 0))).snapshot()
        val preview = records.single()
        assertEquals(ProgramKind.PREVIEW, preview.kind)
        assertEquals(33L, preview.channelId)
        assertEquals("Recommendations", preview.channelName)
    }

    @Test fun slowArtworkDoesNotBlockMetadataOrOtherImagesAndCannotRestoreRemovedRows() = runBlocking {
        val slow = CompletableDeferred<Unit>()
        val fast = CompletableDeferred<Unit>()
        val started = TestChannel<String>(TestChannel.UNLIMITED)
        val art = FakeArtwork { uri ->
            started.send(uri)
            if (uri == "slow") slow.await() else fast.await()
            "/cache/$uri.png"
        }
        val rows = mutableListOf(row(1, poster = "slow"), row(2, poster = "fast"))
        val requests = TestChannel<Set<Long>>(TestChannel.UNLIMITED)
        val snapshots = fixture(rows, artwork = art).snapshots(requests.receiveAsFlow()).produceIn(this)
        try {
            requests.send(emptySet())
            val initial = snapshots.next()
            assertEquals(2, initial.size)
            assertTrue(initial.all { it.posterPath == null && it.intentUri != null })
            withTimeout(3_000) { repeat(2) { started.receive() } }

            // Remove one title and update the other's progress while BOTH downloads are stalled.
            rows.removeAt(0)
            rows[0][2] = "Updated title"
            rows[0][6] = 60_000
            requests.send(emptySet())
            val updated = snapshots.next().single()
            assertEquals("Updated title", updated.title)
            assertEquals(60_000L, updated.positionMs)
            assertNull(updated.posterPath)

            fast.complete(Unit)
            val dressed = snapshots.next().single()
            assertEquals("/cache/fast.png", dressed.posterPath)
            assertEquals(60_000L, dressed.positionMs)
            assertFalse(slow.isCompleted)

            slow.complete(Unit)
            requests.close()
            withTimeout(3_000) {
                for (late in snapshots) {
                    assertEquals("Updated title", late.single().title)
                    assertEquals("/cache/fast.png", late.single().posterPath)
                }
            }
        } finally {
            snapshots.cancel()
            requests.cancel()
        }
    }

    @Test fun staleArtworkRemainsVisibleWhenRefreshFails() = runBlocking {
        val art = FakeArtwork { throw java.io.IOException("offline") }
        art.stale["poster"] = "/cache/old-poster.png"
        val collector = fixture(listOf(row(1, poster = "poster")), artwork = art)
        assertEquals("/cache/old-poster.png", collector.snapshot().single().posterPath)
        assertTrue(art.fetched.isEmpty())
        val snapshots = withTimeout(3_000) { collector.snapshots(flowOf(emptySet())).toList() }
        assertTrue(snapshots.isNotEmpty())
        assertTrue(snapshots.all { it.single().posterPath == "/cache/old-poster.png" })
        assertEquals(listOf("poster"), art.fetched.toList())
    }

    @Test fun missingArtworkKeepsAPlayableTitleWhenDownloadFails() = runBlocking {
        val collector = fixture(listOf(row(1, poster = "missing")), artwork = FakeArtwork { null })
        val snapshots = withTimeout(3_000) { collector.snapshots(flowOf(emptySet())).toList() }
        assertTrue(snapshots.isNotEmpty())
        snapshots.forEach {
            assertEquals("Test episode", it.single().title)
            assertEquals("nflx://episode-1", it.single().intentUri)
            assertNull(it.single().posterPath)
        }
    }

    @Test fun disabledPreviewArtIsNotDownloadedAndSharedImagesAreFetchedOnceWhenEnabled() = runBlocking {
        val art = FakeArtwork { "/cache/$it.png" }
        val channels = listOf(arrayOf<Any?>(33L, "com.netflix.ninja", TvContractCompat.Channels.TYPE_PREVIEW, 0, "Recommendations"))
        val collector = fixture(emptyList(), channels, listOf(row(1, poster = "shared"), row(2, poster = "shared")), art)
        withTimeout(3_000) { collector.snapshots(flowOf(emptySet())).toList() }
        assertTrue(art.fetched.isEmpty())
        val enabled = withTimeout(3_000) { collector.snapshots(flowOf(setOf(33L))).toList() }
        assertEquals(listOf("shared"), art.fetched.toList())
        assertTrue(enabled.last().all { it.posterPath == "/cache/shared.png" })
    }

    @Test fun downloadsStayBoundedWhileAllWorkersAreBusy() = runBlocking {
        val release = CompletableDeferred<Unit>()
        val started = TestChannel<Unit>(TestChannel.UNLIMITED)
        val active = java.util.concurrent.atomic.AtomicInteger()
        val maximum = java.util.concurrent.atomic.AtomicInteger()
        val art = FakeArtwork { uri ->
            val count = active.incrementAndGet()
            maximum.updateAndGet { maxOf(it, count) }
            try {
                started.send(Unit)
                release.await()
                "/cache/$uri.png"
            } finally {
                active.decrementAndGet()
            }
        }
        val rows = (1L..6L).map { row(it, poster = "poster-$it") }
        val requests = TestChannel<Set<Long>>(TestChannel.UNLIMITED)
        val snapshots = fixture(rows, artwork = art).snapshots(requests.receiveAsFlow()).produceIn(this)
        try {
            requests.send(emptySet())
            assertEquals(6, snapshots.next().size)
            withTimeout(3_000) { repeat(4) { started.receive() } }
            assertEquals(4, art.fetched.size)
            rows[0][6] = 70_000
            requests.send(emptySet())
            assertEquals(70_000L, snapshots.next().first().positionMs)
            requests.close()
            release.complete(Unit)
            withTimeout(3_000) { for (ignored in snapshots) { } }
            assertTrue(maximum.get() <= 4)
            assertEquals(6, art.fetched.size)
        } finally {
            snapshots.cancel()
            requests.cancel()
        }
    }

    @Test fun diskFallbackKeepsStaleImagesButRejectsEmptyFilesAndTextRedirects() = runBlocking {
        val dir = java.io.File(context.cacheDir, "art-fallback-${java.util.UUID.randomUUID()}").apply { mkdirs() }
        try {
            fun seed(uri: String, bytes: ByteArray): java.io.File {
                val hash = java.security.MessageDigest.getInstance("SHA-1").digest(uri.toByteArray()).joinToString("") { "%02x".format(it) }
                return java.io.File(dir, "$hash.img").apply {
                    writeBytes(bytes)
                    setLastModified(System.currentTimeMillis() - 10_000)
                }
            }
            val image = seed("test://stale", android.util.Base64.decode(
                "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aD1sAAAAASUVORK5CYII=", 0))
            seed("test://empty", byteArrayOf())
            seed("test://redirect", "https://example.invalid/image.jpg".toByteArray())
            val fetcher = tv.lounge.platform.ArtworkFetcher(context.contentResolver, dir, maxAgeMs = 1) { false }
            assertEquals(image.absolutePath, fetcher.cached("test://stale"))
            assertNull(fetcher.cached("test://stale", allowStale = false))
            assertNull(fetcher.cached("test://empty"))
            assertNull(fetcher.cached("test://redirect"))
            // Unsupported schemes fail immediately; the old image must still be returned.
            assertEquals(image.absolutePath, fetcher.fetch("test://stale"))
            // Item image addresses are resized before hashing; a refused connection must still find the stale copy.
            val item = "http://127.0.0.1:9/Items/1/Images/Primary"
            val sized = seed(tv.lounge.platform.sizedForTiles(item), image.readBytes())
            assertEquals(sized.absolutePath, fetcher.cached(item))
            assertEquals(sized.absolutePath, fetcher.fetch(item))
        } finally {
            dir.deleteRecursively()
        }
    }

    private suspend fun ReceiveChannel<List<ProgramRecord>>.next() = withTimeout(3_000) { receive() }

    private class FakeArtwork(private val download: suspend (String) -> String?) : TvArtwork {
        val stale = java.util.concurrent.ConcurrentHashMap<String, String>()
        private val fresh = java.util.concurrent.ConcurrentHashMap<String, String>()
        val fetched = java.util.Collections.synchronizedList(mutableListOf<String>())
        override fun cached(uri: String, allowStale: Boolean) = fresh[uri] ?: if (allowStale) stale[uri] else null
        override suspend fun fetch(uri: String): String? {
            fetched += uri
            return download(uri)?.also { fresh[uri] = it }
        }
        override fun prune(keep: Collection<String>) { }
    }

    /** Read-only device probe, explicitly opt in with -e tvProviderProbe true. */
    @Test fun deviceProviderDiagnostics() {
        org.junit.Assume.assumeTrue(InstrumentationRegistry.getArguments().getString("tvProviderProbe") == "true")
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        instrumentation.uiAutomation.grantRuntimePermission(context.packageName, TvContractCompat.PERMISSION_READ_TV_LISTINGS)
        assertTrue(TvProviderCollector(context).hasPermission())
        val resolver = context.contentResolver
        for (uri in listOf(TvContractCompat.WatchNextPrograms.CONTENT_URI, TvContractCompat.Channels.CONTENT_URI, TvContractCompat.PreviewPrograms.CONTENT_URI)) {
            resolver.query(uri, arrayOf("_id", "package_name", "browsable"), null, null, null)!!.use { cursor ->
                val counts = mutableMapOf<String, Int>()
                while (cursor.moveToNext()) {
                    val key = "${cursor.getString(1)} browsable=${cursor.getInt(2)}"
                    counts[key] = (counts[key] ?: 0) + 1
                }
                Log.i("LoungeTvTest", "$uri: $counts")
            }
        }
        resolver.query(TvContractCompat.WatchNextPrograms.CONTENT_URI, WatchNextProgram.PROJECTION, null, null, null)!!.use { cursor ->
            var visible = 0
            while (cursor.moveToNext()) {
                val p = WatchNextProgram.fromCursor(cursor)
                if (p.isBrowsable && !p.title.isNullOrBlank()) visible++
            }
            Log.i("LoungeTvTest", "Watch Next rows accepted by Lounge: $visible")
        }
    }

    private fun row(id: Long, browsable: Int = 1, title: String = "Test episode", poster: String? = null) = arrayOf<Any?>(
        id, "com.netflix.ninja", title, browsable, "episode-1", 0, 30_000, 120_000, 1_000L, "nflx://episode-1", poster,
    )

    private fun fixture(
        rows: List<Array<Any?>>,
        channels: List<Array<Any?>> = emptyList(),
        previews: List<Array<Any?>> = emptyList(),
        artwork: TvArtwork? = null,
    ): TvProviderCollector {
        val provider = object : ContentProvider() {
            override fun onCreate() = true
            override fun query(uri: Uri, projection: Array<out String>?, selection: String?, args: Array<out String>?, sort: String?): Cursor {
                if (uri == TvContractCompat.Channels.CONTENT_URI) {
                    return MatrixCursor(arrayOf("_id", "package_name", "type", "browsable", "display_name")).apply {
                        channels.forEach { addRow(it) }
                    }
                }
                val columns = arrayOf("_id", "package_name", "title", "browsable", "internal_provider_id", "watch_next_type",
                    "last_playback_position_millis", "duration_millis", "last_engagement_time_utc_millis", "intent_uri", "poster_art_uri")
                return MatrixCursor(columns).apply {
                    if (uri == TvContractCompat.WatchNextPrograms.CONTENT_URI) rows.forEach { addRow(it) }
                    if (uri.path == TvContractCompat.PreviewPrograms.CONTENT_URI.path) previews.forEach { addRow(it) }
                }
            }
            override fun getType(uri: Uri): String? = null
            override fun insert(uri: Uri, values: ContentValues?): Uri? = error("read only")
            override fun delete(uri: Uri, selection: String?, args: Array<out String>?): Int = error("read only")
            override fun update(uri: Uri, values: ContentValues?, selection: String?, args: Array<out String>?): Int = error("read only")
        }
        provider.attachInfo(context, android.content.pm.ProviderInfo().apply { authority = TvContractCompat.AUTHORITY })
        val resolver = ContentResolver.wrap(provider)
        val fixtureContext = object : ContextWrapper(context) {
            override fun getContentResolver() = resolver
            override fun checkPermission(permission: String, pid: Int, uid: Int) = PackageManager.PERMISSION_GRANTED
        }
        return if (artwork != null) TvProviderCollector(fixtureContext, artwork) else TvProviderCollector(fixtureContext)
    }
}
