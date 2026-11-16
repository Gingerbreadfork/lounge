package tv.lounge.platform

import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

/** The settings a backup carries, and what a restore will and will not take from one. */
@RunWith(RobolectricTestRunner::class)
class PrefsBackupTest {
    @Test
    fun a_backup_names_how_the_home_looks_and_a_restore_sets_it_again() = runBlocking {
        val prefs = Prefs(RuntimeEnvironment.getApplication())
        val platform = AndroidPlatform(RuntimeEnvironment.getApplication())

        // A TV nobody has changed says each setting's default, with the two it follows the TV on.
        assertEquals(
            mapOf(
                "theme" to "Dusk", "sounds" to "true", "lite_mode" to "auto", "reduced_motion" to "auto",
                "show_mobile_apps" to "false", "backdrop" to "still", "ambient_style" to "pictures", "all_apps_sort" to "az",
                "notifications" to "true", "notifications_hidden" to "",
            ),
            platform.preferences(),
        )

        // A TV in setup has nothing to settle. One that finished setup on an older Lounge, with
        // notification access already given, has them off from then on and is told once.
        assertEquals(false, prefs.settleNotifications(accessGiven = true))
        prefs.setOnboardingDone(true)
        assertEquals(false, prefs.notifications.first().shown)
        assertEquals(true, prefs.settleNotifications(accessGiven = true))
        assertEquals(false, prefs.settleNotifications(accessGiven = true))
        assertEquals("false", platform.preferences()["notifications"])
        // Finishing setup again leaves a choice already made alone.
        prefs.finishSetup()
        assertEquals(false, prefs.notifications.first().shown)
        prefs.setCompanionEnabled(true)
        val backup = mapOf(
            "theme" to "Opera", "sounds" to "false", "lite_mode" to "true", "reduced_motion" to "false",
            "show_mobile_apps" to "true", "backdrop" to "silk", "ambient_style" to "plugin:tv.example.scenes:smoke", "all_apps_sort" to "opened",
            "notifications" to "false", "notifications_hidden" to "android,tv.example.shop",
        )
        // What belongs to this TV alone is not a backup's to change, whatever the file says.
        platform.restorePreferences(backup + mapOf("onboarding_done" to "false", "companion_enabled" to "false", "made_up" to "x"))
        val restored = prefs.all.first()
        assertEquals(Prefs.Theme.Opera, restored.theme)
        assertEquals(false, restored.sounds)
        assertEquals(true, restored.liteMode)
        assertEquals(false, restored.reducedMotion)
        assertEquals(true, restored.showMobileApps)
        assertEquals("silk", restored.backdrop)
        assertEquals(Prefs.SORT_OPENED, restored.allAppsSort)
        assertEquals("plugin:tv.example.scenes:smoke", prefs.ambientStyle.first())
        assertEquals(Prefs.NotificationPrefs(shown = false, hiddenApps = setOf("android", "tv.example.shop")), prefs.notifications.first())
        assertTrue(restored.onboardingDone && restored.companionEnabled)
        assertEquals(backup, platform.preferences())

        // A value that is not one of a setting's own is passed over, and so is a setting left out.
        platform.restorePreferences(mapOf("theme" to "Neon", "sounds" to "loud", "lite_mode" to "sometimes", "backdrop" to " ", "all_apps_sort" to "size", "notifications_hidden" to "tv.a, not a package"))
        assertEquals(backup, platform.preferences())

        // Following the TV again is a value like any other.
        platform.restorePreferences(mapOf("lite_mode" to "auto", "reduced_motion" to "auto"))
        assertNull(prefs.all.first().liteMode)
        assertNull(prefs.all.first().reducedMotion)
        assertEquals("Opera", platform.preferences()["theme"])
    }
}
