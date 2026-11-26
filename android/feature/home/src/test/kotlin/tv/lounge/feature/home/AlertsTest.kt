package tv.lounge.feature.home

import android.content.Context
import android.os.PowerManager
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.widget.FrameLayout
import android.widget.TextView
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.shadows.ShadowLooper
import org.robolectric.shadows.ShadowSettings
import org.robolectric.shadows.ShadowWindowManagerImpl
import tv.lounge.core.PlatformException
import tv.lounge.design.LoungeColors
import tv.lounge.platform.HomeAlert
import java.util.concurrent.TimeUnit
import kotlin.concurrent.thread

/** Where a pop-up from the home is drawn, what it says and when it goes. */
@RunWith(RobolectricTestRunner::class)
class AlertsTest {
    private val context: Context = RuntimeEnvironment.getApplication()
    private val popups = AlertPopups(context) { LoungeColors.Dusk }
    private val home = FrameLayout(context)
    private val ring = HomeAlert("Someone is at the door", "Front door", null, 20)

    private fun settle() = ShadowLooper.idleMainLooper()

    private fun words(view: View): List<String> = when (view) {
        is TextView -> listOf(view.text.toString())
        is ViewGroup -> (0 until view.childCount).flatMap { words(view.getChildAt(it)) }
        else -> emptyList()
    }

    private fun overApps(): List<View> = (shadowOf(context.getSystemService(WindowManager::class.java)) as ShadowWindowManagerImpl).views

    @Test
    fun withNowhereToDrawTheCallerIsToldWhatToRun() {
        ShadowSettings.setCanDrawOverlays(false)
        val refused = assertThrows(PlatformException.Failed::class.java) { popups.show(ring) }
        assertEquals(context.getString(R.string.alerts_not_over_apps), refused.detail)
    }

    @Test
    fun withoutLeaveToDrawOverAppsItShowsInTheHomesWindow() {
        ShadowSettings.setCanDrawOverlays(false)
        popups.homeWindow = { home }
        popups.show(ring)
        settle()
        assertEquals(1, home.childCount)
        assertEquals(listOf("Someone is at the door", "Front door"), words(home))
        assertTrue(overApps().isEmpty())
    }

    @Test
    fun withLeaveItIsDrawnOverWhateverIsOnScreenAndTakesNoFocus() {
        ShadowSettings.setCanDrawOverlays(true)
        popups.homeWindow = { home }
        popups.show(ring)
        settle()
        assertEquals(0, home.childCount)
        val card = overApps().single()
        assertEquals(listOf("Someone is at the door", "Front door"), words(card))
        val place = card.layoutParams as WindowManager.LayoutParams
        assertEquals(WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY, place.type)
        assertTrue(place.flags and WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE != 0)
        // One that lets touches through is drawn part see-through by Android.
        assertEquals(0, place.flags and WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE)
        card.performClick()
        assertTrue(overApps().isEmpty())
    }

    @Test
    fun aNewPopUpTakesThePlaceOfTheLastAndEachGoesWhenItsTimeIsUp() {
        ShadowSettings.setCanDrawOverlays(false)
        popups.homeWindow = { home }
        popups.show(ring)
        settle()
        popups.show(HomeAlert("Post has come", null, null, 5))
        settle()
        assertEquals(listOf("Post has come"), words(home))
        ShadowLooper.idleMainLooper(4, TimeUnit.SECONDS)
        assertEquals(1, home.childCount)
        ShadowLooper.idleMainLooper(2, TimeUnit.SECONDS)
        assertEquals(0, home.childCount)
    }

    @Test
    fun itIsTakenDownWhenTheHomeGoesAway() {
        ShadowSettings.setCanDrawOverlays(false)
        popups.homeWindow = { home }
        popups.show(ring)
        settle()
        popups.homeGone()
        settle()
        assertEquals(0, home.childCount)
    }

    @Test
    fun oneOverAnotherAppStaysWhenTheHomeGoesAway() {
        ShadowSettings.setCanDrawOverlays(true)
        popups.show(ring)
        settle()
        popups.homeGone()
        settle()
        assertEquals(1, overApps().size)
    }

    @Test
    fun theRemotesFocusCannotLandOnIt() {
        ShadowSettings.setCanDrawOverlays(false)
        popups.homeWindow = { home }
        popups.show(ring)
        settle()
        assertTrue(!home.getChildAt(0).isFocusable)
    }

    @Test
    fun withTheScreenOffNothingIsShownAndNothingIsRefused() {
        ShadowSettings.setCanDrawOverlays(false)
        shadowOf(context.getSystemService(PowerManager::class.java)).turnScreenOn(false)
        popups.show(ring)
        settle()
        assertEquals(0, home.childCount)
    }

    @Test
    fun aCallFromAnotherThreadHearsThatThereIsNowhereToShowIt() {
        ShadowSettings.setCanDrawOverlays(false)
        popups.homeWindow = { null }
        var refused: Throwable? = null
        val caller = thread { refused = runCatching { popups.show(ring) }.exceptionOrNull() }
        while (caller.isAlive) {
            settle()
            Thread.sleep(10)
        }
        assertEquals(context.getString(R.string.alerts_not_over_apps), (refused as PlatformException.Failed).detail)
    }
}
