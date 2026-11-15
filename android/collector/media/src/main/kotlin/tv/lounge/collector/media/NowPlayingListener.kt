package tv.lounge.collector.media

import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification

/**
 * Lounge's notification access. Granting it unlocks other apps' media sessions, and while
 * Settings, Notifications is on it also feeds [NotificationFeed], which keeps what apps post in
 * memory for the home and nowhere else. The class keeps its name: the grant is made to it.
 */
class NowPlayingListener : NotificationListenerService() {
    override fun onListenerConnected() = NotificationFeed.connected(this)

    override fun onListenerDisconnected() = NotificationFeed.disconnected(this)

    override fun onNotificationPosted(sbn: StatusBarNotification?, rankingMap: RankingMap?) {
        if (sbn != null) NotificationFeed.posted(this, sbn, rankingMap)
    }

    override fun onNotificationRemoved(sbn: StatusBarNotification?, rankingMap: RankingMap?) {
        if (sbn != null) NotificationFeed.removed(this, sbn.key)
    }

    override fun onNotificationRankingUpdate(rankingMap: RankingMap?) {
        if (rankingMap != null) NotificationFeed.ranked(this, rankingMap)
    }

    override fun onDestroy() {
        NotificationFeed.disconnected(this)
        super.onDestroy()
    }
}
