package tv.lounge.feature.home

import androidx.test.core.app.ApplicationProvider
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import tv.lounge.core.PlatformException
import tv.lounge.platform.AppNotification
import tv.lounge.platform.NotificationSource
import tv.lounge.platform.Prefs

/** What a paired phone is told of the TV's notifications, and what it may dismiss. */
@RunWith(RobolectricTestRunner::class)
class PhoneViewsTest {
    private val context = ApplicationProvider.getApplicationContext<android.content.Context>()
    private val prefs = Prefs(context)

    private class Posted(list: List<AppNotification>) : NotificationSource {
        override val posted: StateFlow<List<AppNotification>> = MutableStateFlow(list)
        val dismissed = mutableListOf<String>()
        override fun open(key: String) = false
        override fun press(key: String, action: Int) = false
        override fun dismiss(keys: Collection<String>) {
            dismissed += keys
        }
    }

    private fun notice(key: String, dismissible: Boolean = true) =
        AppNotification(key = key, packageName = "tv.example", app = "Example", title = "Update ready", text = "Version 2", postedAt = 1_000, dismissible = dismissible, opens = false)

    @Test
    fun thePhoneReadsWhatTheTvShowsAndDismissesOnlyThat() = runBlocking {
        prefs.setNotifications(true)
        val source = Posted(listOf(notice("a"), notice("b", dismissible = false)))
        val phone = PhoneNotifications(context, prefs, source, allowed = { true })
        val said = JSONObject(phone.json())
        assertEquals(true, said.getBoolean("on"))
        assertEquals(listOf("a", "b"), (0 until said.getJSONArray("list").length()).map { said.getJSONArray("list").getJSONObject(it).getString("key") })
        assertEquals("Example" to "Update ready", said.getJSONArray("list").getJSONObject(0).let { it.getString("app") to it.getString("title") })

        phone.dismiss("a")
        assertEquals(listOf("a"), source.dismissed)
        // One that cannot be dismissed, and one the TV does not have, are refused in a sentence.
        for (key in listOf("b", "gone")) {
            val refused = assertThrows(PlatformException.Failed::class.java) { runBlocking { phone.dismiss(key) } }
            assertEquals("The TV no longer shows that notification.", refused.detail)
        }
        phone.dismiss(null)
        assertEquals(listOf("a", "a"), source.dismissed)
    }

    @Test
    fun aTvThatMayNotReadNotificationsOrHasThemOffShowsThePhoneNone() = runBlocking {
        val source = Posted(listOf(notice("a")))
        prefs.setNotifications(true)
        val unread = JSONObject(PhoneNotifications(context, prefs, source, allowed = { false }).json())
        assertEquals(false to 0, unread.getBoolean("on") to unread.getJSONArray("list").length())
        prefs.setNotifications(false)
        try {
            val off = JSONObject(PhoneNotifications(context, prefs, source, allowed = { true }).json())
            assertEquals(false to 0, off.getBoolean("on") to off.getJSONArray("list").length())
        } finally {
            // The store is shared by every test here, so it is put back to showing them.
            prefs.setNotifications(true)
        }
    }

    private fun settings(appsOnly: () -> Boolean = { false }, setAppsOnly: (Boolean) -> Unit = {}, clock: () -> Long = { 0L }) = PhoneSettings(
        context,
        prefs,
        appsOnly,
        setAppsOnly,
        scenes = { listOf(ShaderStyle(id = "lounge:silk", name = "Silk", description = null, source = "", values = emptyList(), origin = "lounge")) },
        clock = clock,
    )

    @Test
    fun everyChoiceThePhoneIsOfferedIsOneTheTvTakes() = runBlocking {
        val phone = settings()
        val before = prefs.exported()
        try {
            for (setting in phone.groups().flatMap { it.second }.filter { it.key != "apps_only" }) {
                for ((value, _) in setting.choices) {
                    phone.set(setting.key, value)
                    assertEquals("${setting.key} set to $value", value, phone.groups().flatMap { it.second }.first { it.key == setting.key }.value)
                }
                // A switch is on or off and nothing else, and every setting says what it is set to now.
                if (setting.switch) assertEquals(listOf("true", "false"), setting.choices.map { it.first })
            }
        } finally {
            prefs.restore(before)
        }
    }

    @Test
    fun whatIsNotOnTheListCannotBeSetFromThePhone() = runBlocking {
        val phone = settings()
        for ((key, value) in listOf("onboarding_done" to "false", "companion_enabled" to "false", "theme" to "Neon", "show_clock" to "maybe", "start_action" to "open:app:x")) {
            val refused = assertThrows(PlatformException.Failed::class.java) { runBlocking { phone.set(key, value) } }
            assertEquals("The TV doesn't have that setting.", refused.detail)
        }
    }

    @Test
    fun appsOnlyReadsAsAskedUntilTheCoreHasCaughtUp() = runBlocking {
        var kept = false
        var now = 0L
        val asked = mutableListOf<Boolean>()
        val phone = settings(appsOnly = { kept }, setAppsOnly = { asked += it }, clock = { now })
        fun shown() = runBlocking { phone.groups().flatMap { it.second }.first { it.key == "apps_only" }.value }
        phone.set("apps_only", "true")
        assertEquals(listOf(true), asked)
        // The core has not applied it yet, and the phone is not told the old value.
        assertEquals("true", shown())
        kept = true
        assertEquals("true", shown())
        // Changed back on the TV later, the phone follows the core again.
        kept = false
        assertEquals("false", shown())
        // A change the core never takes stops standing in after a while.
        phone.set("apps_only", "true")
        now += 10_000
        assertEquals("false", shown())
    }

    @Test
    fun theHintTurnedOnFromThePhoneIsOwedInFull() = runBlocking {
        val phone = settings()
        prefs.useHoldHint(learned = true)
        phone.set("hold_hints", "false")
        phone.set("hold_hints", "true")
        assertEquals(true to Prefs.HOLD_HINTS, prefs.all.first().let { it.holdHints to it.holdHintsLeft })
    }
}
