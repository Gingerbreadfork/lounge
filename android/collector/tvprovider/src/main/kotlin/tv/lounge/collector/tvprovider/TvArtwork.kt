package tv.lounge.collector.tvprovider

import android.content.Context
import tv.lounge.platform.ArtworkFetcher
import java.io.File

/** The disk cache and downloader are separate so reading listings never needs the network. */
internal interface TvArtwork {
    fun cached(uri: String, allowStale: Boolean = true): String?
    suspend fun fetch(uri: String): String?
    fun prune(keep: Collection<String>)
}

internal class CachedTvArtwork(context: Context) : TvArtwork {
    private val fetcher = ArtworkFetcher(context, File(context.cacheDir, "tvart"))
    override fun cached(uri: String, allowStale: Boolean) = fetcher.cached(uri, allowStale)
    override suspend fun fetch(uri: String) = fetcher.fetch(uri)
    override fun prune(keep: Collection<String>) = fetcher.prune(keep)
}
