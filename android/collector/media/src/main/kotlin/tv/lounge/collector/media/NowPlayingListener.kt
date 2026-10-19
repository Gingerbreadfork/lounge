package tv.lounge.collector.media

import android.service.notification.NotificationListenerService

/**
 * Exists only so the user can grant notification access, which unlocks other apps' media sessions.
 * Notifications are never read or stored.
 */
class NowPlayingListener : NotificationListenerService()
