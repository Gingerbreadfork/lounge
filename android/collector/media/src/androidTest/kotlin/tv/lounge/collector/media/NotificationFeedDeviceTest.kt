package tv.lounge.collector.media

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.ParcelFileDescriptor
import android.os.Process
import androidx.test.filters.SdkSuppress
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import tv.lounge.platform.AppNotification
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * The listener against a real Android: it is given access, reads what is posted, opens it and
 * dismisses it. It posts through the shell, since Lounge leaves its own notifications out. It
 * shows nothing and wakes nothing, and takes its access and its notifications away afterwards.
 */
@SdkSuppress(minSdkVersion = 33)
class NotificationFeedDeviceTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context: Context get() = instrumentation.targetContext
    private val listener get() = "${context.packageName}/${NowPlayingListener::class.java.name}"

    /** Runs [command] as the shell. Its words are split at spaces and quotes mean nothing, so no argument may hold a space. */
    private fun shell(command: String): String =
        ParcelFileDescriptor.AutoCloseInputStream(instrumentation.uiAutomation.executeShellCommand(command)).use { it.readBytes().decodeToString() }

    private fun <T : Any> await(what: String, seconds: Int = 15, read: () -> T?): T {
        repeat(seconds * 10) {
            read()?.let { return it }
            Thread.sleep(100)
        }
        throw AssertionError("$what did not happen within $seconds seconds")
    }

    private fun onMain(block: () -> Unit) = instrumentation.runOnMainSync(block)

    private fun posted(title: String): AppNotification? = NotificationFeed.posted.value.firstOrNull { it.title == title }

    /** Nothing is posted before the listener is connected, since only the listener can take a notification away again. */
    @Before
    fun allow() {
        shell("cmd notification allow_listener $listener")
        onMain { NotificationFeed.reading = true }
        await("Android binding the listener") { if (NotificationFeed.connected) Unit else null }
    }

    @After
    fun tidy() {
        onMain {
            if (NotificationFeed.connected) {
                NotificationFeed.reading = true
                NotificationFeed.dismiss(NotificationFeed.posted.value.filter { it.packageName == SHELL && "Lounge" in it.title }.map { it.key })
            }
            NotificationFeed.reading = false
        }
        shell("cmd notification disallow_listener $listener")
    }

    @Test
    fun aNotificationIsOpenedItsButtonIsPressedAndItIsTakenAway() {
        // The shell here may not give a notification an intent, so this one is the test's own. The
        // list leaves it out, as it does Lounge's, but the listener holds it and can act on it.
        shell("pm grant ${context.packageName} android.permission.POST_NOTIFICATIONS")
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel(CHANNEL, "Listener test", NotificationManager.IMPORTANCE_LOW))
        val opened = CountDownLatch(1)
        val pressed = CountDownLatch(1)
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) = (if (intent.action == OPENED) opened else pressed).countDown()
        }
        context.registerReceiver(receiver, IntentFilter(OPENED).apply { addAction(PRESSED) }, Context.RECEIVER_NOT_EXPORTED)
        try {
            fun broadcast(code: Int, action: String) = PendingIntent.getBroadcast(context, code, Intent(action).setPackage(context.packageName), PendingIntent.FLAG_IMMUTABLE)
            val notification = Notification.Builder(context, CHANNEL)
                .setSmallIcon(android.R.drawable.stat_notify_more)
                .setContentTitle(TITLE)
                .setContentText(TEXT)
                .setContentIntent(broadcast(1, OPENED))
                .setAutoCancel(true)
                .addAction(Notification.Action.Builder(null, "Eject", broadcast(2, PRESSED)).build())
                .build()
            manager.notify(ID, notification)
            val key = "${Process.myUserHandle().hashCode()}|${context.packageName}|$ID|null|${Process.myUid()}"

            await("the listener holding it and its button's press being sent") {
                var sent = false
                onMain { sent = NotificationFeed.press(key, 0) }
                if (sent) Unit else null
            }
            assertTrue("its app heard the button", pressed.await(10, TimeUnit.SECONDS))

            var sent = false
            onMain { sent = NotificationFeed.open(key) }
            assertTrue("its intent was sent", sent)
            assertTrue("its app heard the press", opened.await(10, TimeUnit.SECONDS))
            // It asked to be taken away once opened.
            await("Android letting go of it") { if (manager.activeNotifications.none { it.id == ID }) Unit else null }
            assertNull("Lounge's own are left out of the list", posted(TITLE))
        } finally {
            context.unregisterReceiver(receiver)
            manager.cancel(ID)
        }
    }

    @Test
    fun whatIsOnTheTvIsReadAndNothingIsHeldOnceReadingStops() {
        shell("cmd notification post -t $TITLE lounge_test $TEXT")
        await("the notification reaching the list") { posted(TITLE) }
        val all = NotificationFeed.posted.value
        // Whatever else is on the device was read without a fault, each with something to say.
        assertTrue(all.all { it.title.isNotBlank() && it.app.isNotBlank() && it.key.isNotBlank() })
        assertEquals(all.sortedByDescending { it.postedAt }, all)
        assertNull("Lounge's own are left out", all.firstOrNull { it.packageName == context.packageName })

        onMain { NotificationFeed.reading = false }
        assertEquals(emptyList<AppNotification>(), NotificationFeed.posted.value)
        onMain { NotificationFeed.reading = true }
        val again = await("reading again") { posted(TITLE) }
        assertEquals(SHELL, again.packageName)
        assertEquals(TEXT, again.text)
        assertTrue("it can be dismissed", again.dismissible)

        onMain { NotificationFeed.dismiss(listOf(again.key)) }
        assertNull("it leaves the list at once", posted(TITLE))
        await("Android letting go of it") { if ("lounge_test" !in shell("cmd notification list")) Unit else null }
    }

    private companion object {
        const val SHELL = "com.android.shell"
        const val TITLE = "LoungeListenerTest"
        const val TEXT = "Free-up-space"
        const val CHANNEL = "listener-test"
        const val ID = 4242
        const val OPENED = "tv.lounge.collector.media.test.OPENED"
        const val PRESSED = "tv.lounge.collector.media.test.PRESSED"
    }
}
