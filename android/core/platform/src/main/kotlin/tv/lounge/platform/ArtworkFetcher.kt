package tv.lounge.platform

import android.content.ContentResolver
import android.content.Context
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
 * failures are remembered for an hour. The URIs come from other apps' rows and are read as Lounge,
 * so file URIs and the providers [isOwnAuthority] names as Lounge's are refused.
 */
class ArtworkFetcher(
    private val resolver: ContentResolver,
    private val dir: File,
    private val maxAgeMs: Long = 7L * 86_400_000L,
    private val isOwnAuthority: (String) -> Boolean,
) {
    /** A fetcher reading as [context]'s app, which refuses the providers that app publishes. */
    constructor(context: Context, dir: File, maxAgeMs: Long = 7L * 86_400_000L) : this(context.contentResolver, dir, maxAgeMs, { authority ->
        runCatching { context.packageManager.resolveContentProvider(authority, 0)?.packageName == context.packageName }.getOrDefault(true)
    })

    private val failures = HashMap<String, Long>()
    private val gate = Semaphore(4)

    /** Fetch every distinct URI in parallel (four at a time); returns a local path per input, or null. */
    suspend fun fetchAll(uris: List<String?>): List<String?> = coroutineScope {
        val paths = uris.filterNotNull().distinct().map { uri -> uri to async { gate.withPermit { fetch(uri) } } }
            .associate { (uri, path) -> uri to path.await() }
        uris.map { uri -> uri?.let { paths[it] } }
    }

    suspend fun fetch(uri: String): String? = withContext(Dispatchers.IO) {
        val file = File(dir, "${sha1(uri)}.img")
        if (file.exists() && System.currentTimeMillis() - file.lastModified() < maxAgeMs && file.length() > 0 && urlInside(file) == null) {
            return@withContext file.absolutePath
        }
        val failedAt = synchronized(failures) { failures[uri] }
        if (failedAt != null && System.currentTimeMillis() - failedAt < 3_600_000L) return@withContext file.takeIf { it.exists() }?.absolutePath
        dir.mkdirs()
        val tmp = File.createTempFile(file.name + ".", ".tmp", dir)
        val started = System.currentTimeMillis()
        try {
            open(Uri.parse(uri)).use { input -> copyBounded(input, tmp, started) }
            if (tmp.length() == 0L) throw IllegalStateException("empty")
            // Some providers answer a logo request with the logo's URL as text; follow it once.
            urlInside(tmp)?.let { target ->
                open(Uri.parse(target)).use { input -> copyBounded(input, tmp, started) }
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

    /**
     * Copies at most [MAX_BYTES], and only until [DEADLINE_MS] after [started]: no picture needs
     * more, and a server that trickles must not hold up the rest of the listings.
     */
    private fun copyBounded(input: InputStream, file: File, started: Long) {
        file.outputStream().use { out ->
            val buf = ByteArray(64 * 1024)
            var total = 0L
            while (true) {
                val n = input.read(buf)
                if (n < 0) break
                total += n
                if (total > MAX_BYTES) throw IllegalStateException("larger than any picture")
                if (System.currentTimeMillis() - started > DEADLINE_MS) throw IllegalStateException("too slow")
                out.write(buf, 0, n)
            }
        }
    }

    /** The URL a tiny text "image" points at, or null when the file holds real image bytes. */
    private fun urlInside(file: File): String? {
        if (file.length() == 0L || file.length() > 2048) return null
        val text = runCatching { file.readBytes().toString(Charsets.UTF_8).trim() }.getOrNull() ?: return null
        return text.takeIf { (it.startsWith("http://") || it.startsWith("https://")) && !it.contains('\n') }
    }

    private fun open(uri: Uri): InputStream = when (uri.scheme?.lowercase()) {
        "content" -> {
            // A user id before the authority still reaches the provider it names.
            val authority = uri.authority?.substringAfterLast('@').orEmpty()
            if (authority.isEmpty() || isOwnAuthority(authority)) throw IllegalArgumentException("not another app's provider")
            resolver.openInputStream(uri) ?: throw IllegalStateException("no stream")
        }
        "android.resource" -> resolver.openInputStream(uri) ?: throw IllegalStateException("no stream")
        "http", "https" -> {
            val conn = URL(uri.toString()).openConnection() as HttpURLConnection
            conn.connectTimeout = 8_000
            conn.readTimeout = 8_000
            conn.instanceFollowRedirects = true
            conn.setRequestProperty("User-Agent", "Lounge/0.1 (Android TV)")
            if (conn.responseCode !in 200..299) throw IllegalStateException("HTTP ${conn.responseCode}")
            val type = conn.contentType.orEmpty().lowercase()
            if (type.startsWith("video/") || type.startsWith("audio/")) throw IllegalStateException("not a picture: $type")
            conn.inputStream
        }
        else -> throw IllegalArgumentException("unsupported scheme")
    }

    /** Delete cached files older than [maxAgeMs] that are not in [keep], and downloads abandoned an hour ago. */
    fun prune(keep: Collection<String>) {
        val keepNames = keep.map { File(it).name }.toHashSet()
        val now = System.currentTimeMillis()
        dir.listFiles()?.forEach { f ->
            val stale = if (f.name.endsWith(".tmp")) f.lastModified() < now - 3_600_000L else f.lastModified() < now - maxAgeMs
            if (f.name !in keepNames && stale) f.delete()
        }
    }

    private fun sha1(s: String): String =
        MessageDigest.getInstance("SHA-1").digest(s.toByteArray()).joinToString("") { "%02x".format(it) }

    private companion object {
        const val TAG = "LoungeArt"
        /** The most the artwork pipeline will decode. */
        const val MAX_BYTES = 12L * 1024 * 1024
        const val DEADLINE_MS = 30_000L
    }
}
