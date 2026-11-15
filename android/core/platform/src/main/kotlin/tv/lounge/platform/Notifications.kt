package tv.lounge.platform

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/** One notification an app has posted, as the home shows it. Held in memory and never stored. */
data class AppNotification(
    val key: String,
    val packageName: String,
    /** The app's name as Android gives it. */
    val app: String,
    val title: String,
    /** The whole text with its own line breaks, or empty. */
    val text: String,
    /** When it last asked for attention, in epoch milliseconds. */
    val postedAt: Long,
    val dismissible: Boolean,
    /** Whether opening it leads somewhere. */
    val opens: Boolean,
    /** Its own buttons, in the app's order. */
    val actions: List<Action> = emptyList(),
) {
    /** One of a notification's buttons; [index] is its place among all the app gave it. */
    data class Action(val index: Int, val label: String)
}

/** The notifications apps have posted on this TV, and what can be done with one. */
interface NotificationSource {
    /** Newest first. Empty while notifications are not being read. */
    val posted: StateFlow<List<AppNotification>>

    /** Opens what [key] leads to. False when its app has taken it back. */
    fun open(key: String): Boolean

    /** Presses the button at [action]. False when its app has taken it back. */
    fun press(key: String, action: Int): Boolean

    fun dismiss(keys: Collection<String>)
}

/** A TV with no notifications to show. */
object NoNotifications : NotificationSource {
    override val posted: StateFlow<List<AppNotification>> = MutableStateFlow(emptyList())
    override fun open(key: String) = false
    override fun press(key: String, action: Int) = false
    override fun dismiss(keys: Collection<String>) = Unit
}
