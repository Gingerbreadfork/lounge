package tv.lounge.collector.media

import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.RemoteInput
import android.content.Intent
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import tv.lounge.platform.AppNotification

/** Which notifications the home lists, and what it reads from one. */
@RunWith(RobolectricTestRunner::class)
class NotificationListingTest {
    private fun posted(
        key: String,
        at: Long = 0,
        packageName: String = "tv.shop",
        title: String = "Title",
        text: String = "",
        group: String? = null,
        summary: Boolean = false,
        ongoing: Boolean = false,
        media: Boolean = false,
        importance: Int? = NotificationManager.IMPORTANCE_DEFAULT,
        suspended: Boolean = false,
        says: Boolean = true,
    ) = Posted(packageName, group, summary, ongoing, media, importance, suspended, says, AppNotification(key, packageName, "Shop", title, text, at, dismissible = true, opens = true))

    private fun keys(vararg posted: Posted) = listed(posted.toList(), own = "tv.lounge.launcher").map { it.key }

    @Test
    fun messages_are_listed_newest_first() {
        assertEquals(listOf("late", "early"), keys(posted("early", at = 1), posted("late", at = 2)))
    }

    @Test
    fun states_and_what_is_playing_and_lounge_s_own_are_left_out() {
        assertEquals(
            listOf("message"),
            keys(
                posted("message"),
                posted("download", ongoing = true),
                posted("song", media = true),
                posted("own", packageName = "tv.lounge.launcher"),
                posted("asleep", suspended = true),
                // Android's marker for a group says nothing itself, though it is shown under its app's name.
                posted("marker", title = "Shop", says = false),
            ),
        )
    }

    @Test
    fun what_android_keeps_out_of_sight_is_left_out() {
        assertEquals(
            listOf("quiet", "unranked"),
            keys(
                posted("unranked", at = 1, importance = null),
                posted("quiet", at = 2, importance = NotificationManager.IMPORTANCE_LOW),
                posted("hidden", importance = NotificationManager.IMPORTANCE_MIN),
                posted("blocked", importance = NotificationManager.IMPORTANCE_NONE),
            ),
        )
    }

    @Test
    fun a_group_shows_its_members_and_a_summary_only_when_it_stands_alone() {
        assertEquals(
            listOf("two", "one", "lone"),
            keys(
                posted("summary", group = "g", summary = true),
                posted("one", at = 2, group = "g"),
                posted("two", at = 3, group = "g"),
                posted("lone", at = 1, group = "h", summary = true),
                // A member that is itself left out does not stand in for its summary.
                posted("going", group = "h", ongoing = true),
            ),
        )
    }

    private val context = RuntimeEnvironment.getApplication()
    private fun intent(action: String) = PendingIntent.getBroadcast(context, 0, Intent(action), PendingIntent.FLAG_IMMUTABLE)
    private fun builder() = Notification.Builder(context, "updates")

    @Test
    fun the_whole_text_is_read_and_one_that_says_nothing_reads_as_empty() {
        val plain = builder().setContentTitle(" 3 apps updated ").setContentText("Plex and two more").build()
        assertEquals("3 apps updated" to "Plex and two more", wordsOf(plain))
        val long = builder().setContentTitle("Storage").setContentText("Short").setStyle(Notification.BigTextStyle().bigText("Some system functions may not work.\nFree up space.")).build()
        assertEquals("Storage" to "Some system functions may not work.\nFree up space.", wordsOf(long))
        val lines = builder().setStyle(Notification.InboxStyle().addLine("First").addLine("Second")).build()
        assertEquals("" to "First\nSecond", wordsOf(lines))
        assertEquals("" to "", wordsOf(builder().setGroup("g").setGroupSummary(true).build()))
    }

    @Test
    fun only_buttons_a_remote_can_press_are_offered() {
        val reply = Notification.Action.Builder(null, "Reply", intent("reply")).addRemoteInput(RemoteInput.Builder("text").build()).build()
        val n = builder()
            .addAction(Notification.Action.Builder(null, "Explore", intent("explore")).build())
            .addAction(reply)
            .addAction(Notification.Action.Builder(null, " EJECT ", intent("eject")).build())
            .build()
        assertEquals(listOf(AppNotification.Action(0, "Explore"), AppNotification.Action(2, "EJECT")), buttonsOf(n))
        assertEquals(emptyList<AppNotification.Action>(), buttonsOf(builder().build()))
    }

    /** Before Android 15 the TV's part of a notification is read by hand, and an app may have put anything there. */
    @Test
    @org.robolectric.annotation.Config(sdk = [34])
    fun what_an_app_puts_where_the_tv_s_intent_goes_is_never_trusted_to_be_one() {
        fun withTvPart(under: android.os.Parcelable) = builder().build().apply {
            extras.putBundle("android.tv.EXTENSIONS", android.os.Bundle().apply { putParcelable("content_intent", under) })
        }
        assertNotNull(opensWith(withTvPart(intent("tv"))))
        assertNull(opensWith(withTvPart(Intent("not.a.pending.intent"))))
        assertNull(opensWith(withTvPart(android.graphics.Rect(0, 0, 1, 1))))
    }

    @Test
    fun a_notification_made_for_tv_opens_with_the_intent_it_carries_there() {
        assertNull(opensWith(builder().build()))
        assertNotNull(opensWith(builder().setContentIntent(intent("open")).build()))
        assertNotNull(opensWith(builder().extend(Notification.TvExtender().setChannelId("tv").setContentIntent(intent("tv"))).build()))
    }
}
