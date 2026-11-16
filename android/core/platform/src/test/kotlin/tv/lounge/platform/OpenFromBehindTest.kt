package tv.lounge.platform

import android.content.Context
import android.content.ContextWrapper
import android.content.Intent
import android.content.pm.ActivityInfo
import android.content.pm.ResolveInfo
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import tv.lounge.core.PlatformException

/**
 * Android drops a start made from behind another app without a word, so Lounge refuses one it
 * can see will be dropped and says why, for the phone that asked.
 */
@RunWith(RobolectricTestRunner::class)
class OpenFromBehindTest {
    private class Tv(base: Context) : ContextWrapper(base) {
        val opened = mutableListOf<Intent>()

        override fun startActivity(intent: Intent) {
            opened += intent
        }
    }

    private val tv = Tv(RuntimeEnvironment.getApplication())
    private val platform = AndroidPlatform(tv)

    private fun openEachWay() {
        platform.viewUri("https://example.org/watch")
        platform.playStream("http://radio/live", "A station", null, emptyMap(), audio = true)
        platform.openAppInfo("tv.example.app")
    }

    @Test
    fun onScreenAndJustAfterLeavingItEverythingOpens() {
        openEachWay()
        platform.offScreenFor = { 2_000L }
        openEachWay()
        assertEquals(6, tv.opened.size)
    }

    @Test
    fun fromBehindAnotherAppThePhoneIsToldWhyNothingOpened() {
        platform.offScreenFor = { 60_000L }
        for (open in listOf<() -> Unit>(
            { platform.viewUri("https://example.org/watch") },
            { platform.playStream("http://radio/live", "A station", null, emptyMap(), audio = true) },
            { platform.openAppInfo("tv.example.app") },
        )) {
            val refused = assertThrows(PlatformException.Failed::class.java) { open() }
            assertEquals(OPENED_FROM_BEHIND, refused.detail)
        }
        assertTrue(tv.opened.isEmpty())
    }

    @Test
    fun theHomeScreenOpensThingsFromBehindAnotherApp() {
        val home = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_HOME)
        val lounge = ResolveInfo().apply { activityInfo = ActivityInfo().apply { packageName = tv.packageName; name = "Home" } }
        @Suppress("DEPRECATION")
        shadowOf(tv.packageManager).addResolveInfoForIntent(home, lounge)
        platform.offScreenFor = { 60_000L }
        openEachWay()
        assertEquals(3, tv.opened.size)
    }
}
