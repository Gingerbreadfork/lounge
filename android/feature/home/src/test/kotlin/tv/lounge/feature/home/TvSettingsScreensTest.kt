package tv.lounge.feature.home

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.ActivityInfo
import android.provider.Settings
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf

/** Which of the TV's own screens the rows in Settings open. */
@RunWith(RobolectricTestRunner::class)
class TvSettingsScreensTest {
    private val context: Context = RuntimeEnvironment.getApplication()
    private val list = "com.android.tv.settings.ACCESSORIES_CONNECTED_DEVICES"
    private val pairing = "com.google.android.intent.action.CONNECT_INPUT"

    private fun tvHas(action: String, exported: Boolean = true, permission: String? = null, owner: String = "com.android.tv.settings") {
        val screen = ActivityInfo().apply {
            packageName = owner
            name = "$owner.$action"
            this.exported = exported
            this.permission = permission
        }
        shadowOf(context.packageManager).addOrUpdateActivity(screen)
        shadowOf(context.packageManager).addIntentFilterForActivity(
            ComponentName(screen.packageName, screen.name),
            IntentFilter(action).apply { addCategory(Intent.CATEGORY_DEFAULT) },
        )
    }

    /** The actions of the screen that opens now, the one that wants Nearby devices first, and the settings to fall back on. */
    private fun found(): List<String?>? = accessoryScreen(context)?.let { listOf(it.open?.action, it.locked?.action, it.settings?.action) }

    @Test
    fun aTvWithNoScreenHasNoRow() {
        assertNull(accessoryScreen(context))
        assertNull(tvSettingsIntent(context))
        assertNull(networkSettingsIntent(context))
    }

    @Test
    fun theListOfAccessoriesComesBeforeThePairingScreen() {
        tvHas(Settings.ACTION_SETTINGS)
        tvHas(pairing)
        assertEquals(listOf(pairing, null, Settings.ACTION_SETTINGS), found())
        tvHas(list)
        assertEquals(listOf(list, null, Settings.ACTION_SETTINGS), found())
    }

    @Test
    fun androidsChooserBetweenSeveralScreensStillOpens() {
        // What Android answers with when more than one screen takes the action: its own, unexported.
        tvHas(Settings.ACTION_SETTINGS, exported = false, owner = "android")
        assertEquals(Settings.ACTION_SETTINGS, tvSettingsIntent(context)?.action)
    }

    @Test
    fun aPairingScreenKeptToNearbyDevicesOpensOnceThatIsAllowed() {
        tvHas(Settings.ACTION_SETTINGS)
        tvHas(pairing, permission = NEARBY_DEVICES)
        tvHas(list, exported = false)
        assertEquals(listOf(null, pairing, Settings.ACTION_SETTINGS), found())
        shadowOf(RuntimeEnvironment.getApplication()).grantPermissions(NEARBY_DEVICES)
        assertEquals(listOf(pairing, null, Settings.ACTION_SETTINGS), found())
    }

    @Test
    fun aScreenKeptToAPermissionLoungeNeverAsksForIsLeftAlone() {
        tvHas(Settings.ACTION_SETTINGS)
        tvHas(pairing, permission = "android.permission.BLUETOOTH_PRIVILEGED")
        assertEquals(listOf(null, null, Settings.ACTION_SETTINGS), found())
    }
}
