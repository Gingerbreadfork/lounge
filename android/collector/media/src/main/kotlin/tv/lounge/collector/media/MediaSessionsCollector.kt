package tv.lounge.collector.media

import android.content.ComponentName
import android.content.Context
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.media.MediaMetadata
import android.media.session.MediaController
import android.media.session.MediaSessionManager
import android.media.session.PlaybackState
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.core.app.NotificationManagerCompat
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.launch
import kotlinx.coroutines.flow.callbackFlow
import tv.lounge.core.PlaybackStatus
import tv.lounge.core.SessionRecord
import java.io.File
import java.io.FileOutputStream

/**
 * Observes active media sessions once notification access is granted: what is playing, where it is
 * up to, which transport commands work. Album art goes to a cache file for the artwork pipeline.
 */
class MediaSessionsCollector(private val context: Context) {
    private val manager = context.getSystemService(Context.MEDIA_SESSION_SERVICE) as MediaSessionManager
    private val component = ComponentName(context, NowPlayingListener::class.java)
    private val artDir = File(context.cacheDir, "sessions").apply { mkdirs() }
    private val pm: PackageManager = context.packageManager
    private val main = Handler(Looper.getMainLooper())

    @Volatile private var controllers: List<MediaController> = emptyList()
    /** When each package's session was first seen, for sessions that report no time of their own. */
    private val firstSeen = java.util.concurrent.ConcurrentHashMap<String, Long>()

    fun isEnabled(): Boolean =
        NotificationManagerCompat.getEnabledListenerPackages(context).contains(context.packageName)

    /** Current sessions, re-emitted on every metadata or playback-state change. Empty without access. */
    fun sessions(): Flow<List<SessionRecord>> = callbackFlow {
        if (!isEnabled()) {
            trySend(emptyList())
            awaitClose { }
            return@callbackFlow
        }
        val callbacks = HashMap<MediaController, MediaController.Callback>()
        // Album art is encoded to disk here; that and the metadata reads leave the main thread.
        fun publish() {
            val current = controllers
            launch(Dispatchers.IO) { trySend(current.mapNotNull { record(it) }) }
        }
        fun attach(list: List<MediaController>) {
            callbacks.forEach { (c, cb) -> c.unregisterCallback(cb) }
            callbacks.clear()
            controllers = list
            for (c in list) {
                val cb = object : MediaController.Callback() {
                    override fun onMetadataChanged(metadata: MediaMetadata?) = publish()
                    override fun onPlaybackStateChanged(state: PlaybackState?) = publish()
                    override fun onSessionDestroyed() = publish()
                }
                c.registerCallback(cb, main)
                callbacks[c] = cb
            }
            publish()
        }
        val listener = MediaSessionManager.OnActiveSessionsChangedListener { list -> attach(list ?: emptyList()) }
        try {
            manager.addOnActiveSessionsChangedListener(listener, component, main)
            attach(manager.getActiveSessions(component))
        } catch (e: SecurityException) {
            Log.w(TAG, "notification access not granted", e)
            trySend(emptyList())
        }
        awaitClose {
            runCatching { manager.removeOnActiveSessionsChangedListener(listener) }
            callbacks.forEach { (c, cb) -> runCatching { c.unregisterCallback(cb) } }
        }
    }

    /**
     * Transport command for the session owned by [packageName]: play-pause, play, pause, next, previous, stop.
     * The sessions are asked for afresh, since [sessions] is only collected while the home is on screen.
     */
    fun send(packageName: String, command: String): Boolean {
        val current = runCatching { manager.getActiveSessions(component) }.getOrNull() ?: controllers
        val c = current.firstOrNull { it.packageName == packageName } ?: return false
        val t = c.transportControls
        when (command) {
            "play-pause" -> if (c.playbackState?.state == PlaybackState.STATE_PLAYING) t.pause() else t.play()
            "play" -> t.play()
            "pause" -> t.pause()
            "next" -> t.skipToNext()
            "previous" -> t.skipToPrevious()
            "stop" -> t.stop()
            else -> return false
        }
        return true
    }

    private fun record(c: MediaController): SessionRecord? {
        val md = c.metadata
        val ps = c.playbackState
        val title = md?.getString(MediaMetadata.METADATA_KEY_DISPLAY_TITLE)?.takeIf { it.isNotBlank() }
            ?: md?.getString(MediaMetadata.METADATA_KEY_TITLE)?.takeIf { it.isNotBlank() }
        val subtitle = md?.getString(MediaMetadata.METADATA_KEY_DISPLAY_SUBTITLE)?.takeIf { it.isNotBlank() }
            ?: md?.getString(MediaMetadata.METADATA_KEY_ARTIST)?.takeIf { it.isNotBlank() }
            ?: md?.getString(MediaMetadata.METADATA_KEY_ALBUM)?.takeIf { it.isNotBlank() }
        val status = when (ps?.state) {
            PlaybackState.STATE_PLAYING, PlaybackState.STATE_BUFFERING, PlaybackState.STATE_FAST_FORWARDING,
            PlaybackState.STATE_REWINDING, PlaybackState.STATE_SKIPPING_TO_NEXT, PlaybackState.STATE_SKIPPING_TO_PREVIOUS,
            -> PlaybackStatus.PLAYING
            PlaybackState.STATE_PAUSED -> PlaybackStatus.PAUSED
            else -> PlaybackStatus.STOPPED
        }
        val actions = ps?.actions ?: 0L
        // A position and the time it was true. A playing session is in use now, so its position is
        // carried to now; any other keeps the time its state last changed, so a paused session does
        // not look recently used.
        val now = System.currentTimeMillis()
        val stateTime = ps?.lastPositionUpdateTime?.takeIf { it > 0 }
        val playing = status == PlaybackStatus.PLAYING
        val position = ps?.let { s ->
            s.position.takeIf { it >= 0 }?.let { p ->
                if (playing && stateTime != null) p + ((android.os.SystemClock.elapsedRealtime() - stateTime) * s.playbackSpeed).toLong() else p
            }
        }
        val updated = when {
            playing -> now
            stateTime != null -> now - (android.os.SystemClock.elapsedRealtime() - stateTime)
            else -> firstSeen.getOrPut(c.packageName) { now }
        }
        return SessionRecord(
            `package` = c.packageName,
            appLabel = appLabel(c.packageName),
            title = title,
            subtitle = subtitle,
            artPath = art(c.packageName, title, md),
            durationMs = md?.getLong(MediaMetadata.METADATA_KEY_DURATION)?.takeIf { it > 0 },
            positionMs = position,
            status = status,
            canPause = actions and (PlaybackState.ACTION_PAUSE or PlaybackState.ACTION_PLAY_PAUSE) != 0L,
            canPlay = actions and (PlaybackState.ACTION_PLAY or PlaybackState.ACTION_PLAY_PAUSE) != 0L,
            canSkipNext = actions and PlaybackState.ACTION_SKIP_TO_NEXT != 0L,
            canSkipPrevious = actions and PlaybackState.ACTION_SKIP_TO_PREVIOUS != 0L,
            updatedMs = updated,
        )
    }

    private fun art(pkg: String, title: String?, md: MediaMetadata?): String? {
        md ?: return null
        val bitmap: Bitmap = md.getBitmap(MediaMetadata.METADATA_KEY_ART)
            ?: md.getBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART)
            ?: md.getBitmap(MediaMetadata.METADATA_KEY_DISPLAY_ICON)
            ?: return md.getString(MediaMetadata.METADATA_KEY_ART_URI)?.takeIf { it.startsWith("http") || it.startsWith("content") }
        // Keyed on what the metadata says, not the bitmap object: every fetch is a fresh copy.
        val described = listOf(
            title,
            md.getString(MediaMetadata.METADATA_KEY_MEDIA_ID),
            md.getString(MediaMetadata.METADATA_KEY_ARTIST),
            md.getString(MediaMetadata.METADATA_KEY_ALBUM),
            md.getString(MediaMetadata.METADATA_KEY_ART_URI),
            md.getString(MediaMetadata.METADATA_KEY_ALBUM_ART_URI),
            md.getString(MediaMetadata.METADATA_KEY_DISPLAY_ICON_URI),
        ).joinToString("\u0000") { it.orEmpty() }
        val key = (described + bitmap.width + "x" + bitmap.height).hashCode().toUInt().toString(16)
        val file = File(artDir, "$pkg-$key.png")
        // Metadata and state changes publish together, so two writers can want the same file.
        synchronized(artDir) {
            if (!file.exists()) {
                runCatching {
                    val tmp = File(file.path + ".tmp")
                    FileOutputStream(tmp).use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
                    if (!tmp.renameTo(file)) { tmp.copyTo(file, overwrite = true); tmp.delete() }
                    artDir.listFiles()?.filter { it.name.startsWith("$pkg-") && it != file }?.forEach { it.delete() }
                }.onFailure { Log.w(TAG, "art for $pkg", it) }
            }
        }
        return file.takeIf { it.exists() }?.absolutePath
    }

    private fun appLabel(pkg: String): String =
        runCatching { pm.getApplicationLabel(pm.getApplicationInfo(pkg, 0)).toString() }.getOrDefault(pkg)

    private companion object {
        const val TAG = "LoungeMedia"
    }
}
