package tv.lounge.collector.media

import android.app.ActivityOptions
import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.pm.ApplicationInfo
import android.os.Build
import android.service.notification.NotificationListenerService.Ranking
import android.service.notification.NotificationListenerService.RankingMap
import android.service.notification.StatusBarNotification
import android.util.Log
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import tv.lounge.platform.AppNotification
import tv.lounge.platform.NotificationSource

/** What the listener knows of one posted notification: enough to say whether the home lists it. */
internal data class Posted(
    val packageName: String,
    /** The group it was posted in, or null for one posted on its own. */
    val group: String?,
    val summary: Boolean,
    /** A state that is still going on, such as a download or a running service, not a message. */
    val ongoing: Boolean,
    /** What is playing, which the Now Playing chip already shows. */
    val media: Boolean,
    /** Android's importance for it, or null where Android gave none. */
    val importance: Int?,
    val suspended: Boolean,
    /** Whether it has a title or a text of its own. One with neither is Android's own marker for a group. */
    val says: Boolean,
    val shown: AppNotification,
)

/**
 * The notifications the home lists, newest first: messages from other apps that Android would
 * show a person. States still going on, what is playing, anything Android keeps out of sight and
 * a group's summary are left out.
 */
internal fun listed(posted: Collection<Posted>, own: String): List<AppNotification> {
    val messages = posted.filter { p ->
        p.packageName != own && !p.ongoing && !p.media && !p.suspended &&
            (p.importance == null || p.importance >= NotificationManager.IMPORTANCE_LOW) && p.says
    }
    val groupsWithMembers = messages.filter { !it.summary }.mapNotNullTo(HashSet()) { it.group }
    return messages.filter { !(it.summary && it.group in groupsWithMembers) }.map { it.shown }.sortedByDescending { it.postedAt }
}

/** The title and whole text a notification gives itself; either may be empty. */
internal fun wordsOf(n: Notification): Pair<String, String> {
    val extras = n.extras
    val title = extras.getCharSequence(Notification.EXTRA_TITLE)?.toString()?.trim().orEmpty()
    val text = extras.getCharSequence(Notification.EXTRA_BIG_TEXT)?.toString()
        ?: extras.getCharSequenceArray(Notification.EXTRA_TEXT_LINES)?.joinToString("\n")
        ?: extras.getCharSequence(Notification.EXTRA_TEXT)?.toString()
    return title to text.orEmpty().trim()
}

/** The buttons a remote can press: those that send something and ask for nothing typed. */
internal fun buttonsOf(n: Notification): List<AppNotification.Action> =
    n.actions.orEmpty().mapIndexedNotNull { i, a ->
        val label = a.title?.toString()?.trim().orEmpty()
        val asksForText = !a.remoteInputs.isNullOrEmpty()
        val suggested = Build.VERSION.SDK_INT >= 29 && a.isContextual
        if (a.actionIntent == null || label.isEmpty() || asksForText || suggested) null else AppNotification.Action(i, label)
    }

private const val TV_EXTENSIONS = "android.tv.EXTENSIONS"
private const val TV_CONTENT_INTENT = "content_intent"

/** Where a notification leads: its own intent, or the one it carries for a TV. */
internal fun opensWith(n: Notification): PendingIntent? = n.contentIntent ?: when {
    Build.VERSION.SDK_INT >= 35 -> Notification.TvExtender(n).contentIntent
    else -> @Suppress("DEPRECATION") n.extras.getBundle(TV_EXTENSIONS)?.getParcelable(TV_CONTENT_INTENT)
}

private fun isMedia(n: Notification): Boolean =
    n.category == Notification.CATEGORY_TRANSPORT || n.extras.containsKey(Notification.EXTRA_MEDIA_SESSION) ||
        n.extras.getString(Notification.EXTRA_TEMPLATE).orEmpty().endsWith("MediaStyle")

/**
 * The notifications on this TV, kept in memory for the home while [reading] is on and dropped the
 * moment it is turned off. Used from the main thread, where the listener is called.
 */
object NotificationFeed : NotificationSource {
    private const val TAG = "Lounge"
    private val shown = MutableStateFlow<List<AppNotification>>(emptyList())
    override val posted: StateFlow<List<AppNotification>> = shown

    private class Entry(val sbn: StatusBarNotification, val posted: Posted)

    private val active = HashMap<String, Entry>()
    private val appNames = HashMap<String, Pair<String, Boolean>>()
    private var listener: NowPlayingListener? = null

    /** Whether Android has the listener bound, which it does only while access is given. */
    internal val connected: Boolean get() = listener != null

    /** Whether notifications are read at all. Off, nothing of them is held. */
    var reading: Boolean = false
        set(on) {
            if (field == on) return
            field = on
            if (on) readAll() else forgetAll()
        }

    internal fun connected(service: NowPlayingListener) {
        listener = service
        if (reading) readAll()
    }

    internal fun disconnected(service: NowPlayingListener) {
        if (listener === service) listener = null
        forgetAll()
    }

    internal fun posted(service: NowPlayingListener, sbn: StatusBarNotification, ranking: RankingMap?) {
        if (!reading) return
        active[sbn.key] = entryOf(service, sbn, ranking, active[sbn.key])
        publish(service)
    }

    internal fun removed(service: NowPlayingListener, key: String) {
        if (active.remove(key) != null) publish(service)
    }

    internal fun ranked(service: NowPlayingListener, ranking: RankingMap) {
        if (!reading || active.isEmpty()) return
        for ((key, entry) in active.entries.toList()) active[key] = entryOf(service, entry.sbn, ranking, entry)
        publish(service)
    }

    private fun readAll() {
        val service = listener ?: return
        val all = runCatching { service.activeNotifications }.getOrNull().orEmpty()
        val ranking = runCatching { service.currentRanking }.getOrNull()
        active.clear()
        for (sbn in all) active[sbn.key] = entryOf(service, sbn, ranking, null)
        publish(service)
    }

    private fun forgetAll() {
        active.clear()
        shown.value = emptyList()
    }

    private fun publish(context: Context) {
        shown.value = listed(active.values.map { it.posted }, context.packageName)
    }

    private fun entryOf(context: Context, sbn: StatusBarNotification, ranking: RankingMap?, before: Entry?): Entry {
        val n = sbn.notification
        val rank = Ranking().takeIf { ranking?.getRanking(sbn.key, it) == true }
        val (app, system) = appNames.getOrPut(sbn.packageName) { appName(context, sbn.packageName) }
        // Only Android's own apps may stand under another name, as the system shows them.
        val name = n.extras.getString(SUBSTITUTE_APP_NAME)?.takeIf { system && it.isNotBlank() } ?: app
        val (title, text) = wordsOf(n)
        // An update that asks not to alert again keeps the time it first asked.
        val quietUpdate = before != null && n.flags and Notification.FLAG_ONLY_ALERT_ONCE != 0
        return Entry(
            sbn,
            Posted(
                packageName = sbn.packageName,
                // Android groups an app's loose notifications itself and says so only in the ranking.
                group = rank?.overrideGroupKey ?: sbn.groupKey.takeIf { sbn.isGroup },
                summary = n.flags and Notification.FLAG_GROUP_SUMMARY != 0,
                ongoing = n.flags and (Notification.FLAG_ONGOING_EVENT or Notification.FLAG_FOREGROUND_SERVICE) != 0,
                media = isMedia(n),
                importance = rank?.importance,
                suspended = rank?.isSuspended == true,
                says = title.isNotEmpty() || text.isNotEmpty(),
                shown = AppNotification(
                    key = sbn.key,
                    packageName = sbn.packageName,
                    app = name,
                    // One with a text and no title is titled with its app.
                    title = title.ifEmpty { name },
                    text = text,
                    postedAt = if (quietUpdate) before!!.posted.shown.postedAt else sbn.postTime,
                    dismissible = sbn.isClearable,
                    opens = opensWith(n) != null,
                    actions = buttonsOf(n),
                ),
            ),
        )
    }

    private const val SUBSTITUTE_APP_NAME = "android.substName"

    /** The app's name, and whether it is part of Android. */
    private fun appName(context: Context, packageName: String): Pair<String, Boolean> {
        val pm = context.packageManager
        val info = runCatching { pm.getApplicationInfo(packageName, 0) }.getOrNull() ?: return packageName to false
        return pm.getApplicationLabel(info).toString() to (info.flags and ApplicationInfo.FLAG_SYSTEM != 0)
    }

    override fun open(key: String): Boolean {
        val entry = active[key] ?: return false
        val n = entry.sbn.notification
        val intent = opensWith(n) ?: return false
        if (!send(intent)) return false
        if (n.flags and Notification.FLAG_AUTO_CANCEL != 0) dismiss(listOf(key))
        return true
    }

    override fun press(key: String, action: Int): Boolean {
        val intent = active[key]?.sbn?.notification?.actions?.getOrNull(action)?.actionIntent ?: return false
        return send(intent)
    }

    override fun dismiss(keys: Collection<String>) {
        val service = listener ?: return
        val gone = keys.filter { active[it]?.sbn?.isClearable == true }
        if (gone.isEmpty()) return
        runCatching { service.cancelNotifications(gone.toTypedArray()) }.onFailure { Log.w(TAG, "notifications not dismissed", it) }
        // Taken off the list at once; Android's own word that they are gone follows.
        gone.forEach { active.remove(it) }
        publish(service)
    }

    /** Sends an app's intent with leave to bring its screen forward, which Lounge has while it is in front. */
    private fun send(intent: PendingIntent): Boolean {
        val context = listener ?: return false
        val options = when {
            Build.VERSION.SDK_INT >= 36 -> ActivityOptions.makeBasic().setPendingIntentBackgroundActivityStartMode(ActivityOptions.MODE_BACKGROUND_ACTIVITY_START_ALLOW_IF_VISIBLE).toBundle()
            Build.VERSION.SDK_INT >= 34 -> @Suppress("DEPRECATION") ActivityOptions.makeBasic().setPendingIntentBackgroundActivityStartMode(ActivityOptions.MODE_BACKGROUND_ACTIVITY_START_ALLOWED).toBundle()
            else -> null
        }
        return try {
            intent.send(context, 0, null, null, null, null, options)
            true
        } catch (e: PendingIntent.CanceledException) {
            Log.w(TAG, "a notification's intent was taken back")
            false
        }
    }
}
