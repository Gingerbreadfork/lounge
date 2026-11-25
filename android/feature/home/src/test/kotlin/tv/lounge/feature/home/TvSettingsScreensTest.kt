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

    private fun opened(): Pair<String?, Boolean>? = accessoryScreen(context)?.let { it.intent.action to it.direct }

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
        assertEquals(pairing to true, opened())
        tvHas(list)
        assertEquals(list to true, opened())
    }

    @Test
    fun androidsChooserBetweenSeveralScreensStillOpens() {
        // What Android answers with when more than one screen takes the action: its own, unexported.
        tvHas(Settings.ACTION_SETTINGS, exported = false, owner = "android")
        assertEquals(Settings.ACTION_SETTINGS, tvSettingsIntent(context)?.action)
    }

    @Test
    fun aScreenLoungeMayNotOpenGivesWayToTheTvsSettings() {
        tvHas(Settings.ACTION_SETTINGS)
        tvHas(pairing, permission = android.Manifest.permission.BLUETOOTH_CONNECT)
        tvHas(list, exported = false)
        assertEquals(Settings.ACTION_SETTINGS to false, opened())
        shadowOf(RuntimeEnvironment.getApplication()).grantPermissions(android.Manifest.permission.BLUETOOTH_CONNECT)
        assertEquals(pairing to true, opened())
    }
}
