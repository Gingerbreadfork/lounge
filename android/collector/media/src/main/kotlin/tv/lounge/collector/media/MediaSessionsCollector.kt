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

    /** Transport command for the session owned by [packageName]: play-pause, play, pause, next, previous, stop. */
    fun send(packageName: String, command: String): Boolean {
        val c = controllers.firstOrNull { it.packageName == packageName } ?: return false
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
        val position = ps?.let { state ->
            if (state.position < 0) null else if (status == PlaybackStatus.PLAYING && state.lastPositionUpdateTime > 0) {
                state.position + ((android.os.SystemClock.elapsedRealtime() - state.lastPositionUpdateTime) * state.playbackSpeed).toLong()
            } else {
                state.position
            }
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
            updatedMs = System.currentTimeMillis(),
        )
    }

    private fun art(pkg: String, title: String?, md: MediaMetadata?): String? {
        md ?: return null
        val bitmap: Bitmap = md.getBitmap(MediaMetadata.METADATA_KEY_ART)
            ?: md.getBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART)
            ?: md.getBitmap(MediaMetadata.METADATA_KEY_DISPLAY_ICON)
            ?: return md.getString(MediaMetadata.METADATA_KEY_ART_URI)?.takeIf { it.startsWith("http") || it.startsWith("content") }
        val key = ((title ?: "") + bitmap.width + bitmap.height + bitmap.generationId).hashCode().toUInt().toString(16)
        val file = File(artDir, "$pkg-$key.png")
        if (!file.exists()) {
            runCatching {
                val tmp = File(file.path + ".tmp")
                FileOutputStream(tmp).use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
                if (!tmp.renameTo(file)) { tmp.copyTo(file, overwrite = true); tmp.delete() }
                artDir.listFiles()?.filter { it.name.startsWith("$pkg-") && it != file }?.forEach { it.delete() }
            }.onFailure { Log.w(TAG, "art for $pkg", it) }
        }
        return file.takeIf { it.exists() }?.absolutePath
    }

    private fun appLabel(pkg: String): String =
        runCatching { pm.getApplicationLabel(pm.getApplicationInfo(pkg, 0)).toString() }.getOrDefault(pkg)

    private companion object {
        const val TAG = "LoungeMedia"
    }
}
