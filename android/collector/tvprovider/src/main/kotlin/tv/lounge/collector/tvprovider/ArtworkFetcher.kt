package tv.lounge.collector.tvprovider

import android.content.ContentResolver
import android.net.Uri
import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.withContext
import java.io.File
import java.io.InputStream
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest

/**
 * Copies artwork from http(s), content and android.resource URIs into a cache directory so the
 * artwork pipeline reads plain files. A URI is fetched once and refreshed after [maxAgeMs];
 * failures are remembered for an hour.
 */
class ArtworkFetcher(
    private val resolver: ContentResolver,
    private val dir: File,
    private val maxAgeMs: Long = 7L * 86_400_000L,
) {
    private val failures = HashMap<String, Long>()
    private val gate = Semaphore(4)

    /** Fetch every URI in parallel (four at a time); returns a local path per input, or null. */
    suspend fun fetchAll(uris: List<String?>): List<String?> = coroutineScope {
        uris.map { uri -> async { if (uri == null) null else gate.withPermit { fetch(uri) } } }.awaitAll()
    }

    suspend fun fetch(uri: String): String? = withContext(Dispatchers.IO) {
        val file = File(dir, "${sha1(uri)}.img")
        if (file.exists() && System.currentTimeMillis() - file.lastModified() < maxAgeMs && file.length() > 0 && urlInside(file) == null) {
            return@withContext file.absolutePath
        }
        val failedAt = synchronized(failures) { failures[uri] }
        if (failedAt != null && System.currentTimeMillis() - failedAt < 3_600_000L) return@withContext file.takeIf { it.exists() }?.absolutePath
        dir.mkdirs()
        val tmp = File(dir, file.name + ".tmp")
        try {
            open(Uri.parse(uri)).use { input -> tmp.outputStream().use { input.copyTo(it) } }
            if (tmp.length() == 0L) throw IllegalStateException("empty")
            // Some providers answer a logo request with the logo's URL as text; follow it once.
            urlInside(tmp)?.let { target ->
                open(Uri.parse(target)).use { input -> tmp.outputStream().use { input.copyTo(it) } }
                if (tmp.length() == 0L || urlInside(tmp) != null) throw IllegalStateException("empty redirect")
            }
            if (!tmp.renameTo(file)) {
                tmp.copyTo(file, overwrite = true)
                tmp.delete()
            }
            file.absolutePath
        } catch (e: Exception) {
            tmp.delete()
            synchronized(failures) { failures[uri] = System.currentTimeMillis() }
            Log.d(TAG, "artwork $uri: ${e.javaClass.simpleName} ${e.message}")
            file.takeIf { it.exists() }?.absolutePath
        }
    }

    /** The URL a tiny text "image" points at, or null when the file holds real image bytes. */
    private fun urlInside(file: File): String? {
        if (file.length() == 0L || file.length() > 2048) return null
        val text = runCatching { file.readBytes().toString(Charsets.UTF_8).trim() }.getOrNull() ?: return null
        return text.takeIf { (it.startsWith("http://") || it.startsWith("https://")) && !it.contains('\n') }
    }

    private fun open(uri: Uri): InputStream = when (uri.scheme?.lowercase()) {
        "http", "https" -> {
            val conn = URL(uri.toString()).openConnection() as HttpURLConnection
            conn.connectTimeout = 8_000
            conn.readTimeout = 8_000
            conn.instanceFollowRedirects = true
            conn.setRequestProperty("User-Agent", "Lounge/0.1 (Android TV)")
            if (conn.responseCode !in 200..299) throw IllegalStateException("HTTP ${conn.responseCode}")
            conn.inputStream
        }
        "content", "android.resource", "file" -> resolver.openInputStream(uri) ?: throw IllegalStateException("no stream")
        else -> throw IllegalArgumentException("unsupported scheme")
    }

    /** Delete cached files older than [maxAgeMs] that are not in [keep]. */
    fun prune(keep: Collection<String>) {
        val keepNames = keep.map { File(it).name }.toHashSet()
        val cutoff = System.currentTimeMillis() - maxAgeMs
        dir.listFiles()?.forEach { f ->
            if (f.name !in keepNames && (f.lastModified() < cutoff || f.name.endsWith(".tmp"))) f.delete()
        }
    }

    private fun sha1(s: String): String =
        MessageDigest.getInstance("SHA-1").digest(s.toByteArray()).joinToString("") { "%02x".format(it) }

    private companion object {
        const val TAG = "LoungeArt"
    }
}
