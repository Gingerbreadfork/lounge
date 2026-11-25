package tv.lounge.feature.home

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.provider.Settings

/** Some TVs' list of the accessories connected to them, with the way to pair another. */
private const val ACTION_CONNECTED_ACCESSORIES = "com.android.tv.settings.ACCESSORIES_CONNECTED_DEVICES"

/** Android TV's screen that looks for a remote, a controller or headphones to pair. */
private const val ACTION_PAIR_ACCESSORY = "com.google.android.intent.action.CONNECT_INPUT"

/**
 * A screen of the TV's own settings, or null on a TV that has none Lounge may open. Some TVs
 * keep a screen to apps that hold a permission, and starting it without one would fail.
 */
private fun Context.settingsScreen(action: String): Intent? {
    val intent = Intent(action).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    val screen = runCatching { packageManager.resolveActivity(intent, PackageManager.MATCH_DEFAULT_ONLY)?.activityInfo }.getOrNull() ?: return null
    // Android's own chooser stands in when several screens answer, and it opens for anyone.
    val chooser = screen.packageName == "android"
    val allowed = screen.exported && (screen.permission == null || checkSelfPermission(screen.permission) == PackageManager.PERMISSION_GRANTED)
    return intent.takeIf { chooser || allowed }
}

/** The TV's own settings, or null on a TV with no such screen. */
internal fun tvSettingsIntent(context: Context): Intent? = context.settingsScreen(Settings.ACTION_SETTINGS)

/** The TV's network screen, or null on a TV with no such screen. */
internal fun networkSettingsIntent(context: Context): Intent? = context.settingsScreen(Settings.ACTION_WIFI_SETTINGS)

/** Where the TV pairs accessories. [direct] is false when that is only somewhere in its settings. */
internal class AccessoryScreen(val intent: Intent, val direct: Boolean)

/**
 * The TV's screen for remotes, controllers and headphones: its list of them, or the one that
 * pairs a new one, or all of its settings where Lounge may open neither. Null on a TV with none.
 */
internal fun accessoryScreen(context: Context): AccessoryScreen? {
    val own = context.settingsScreen(ACTION_CONNECTED_ACCESSORIES) ?: context.settingsScreen(ACTION_PAIR_ACCESSORY)
    return own?.let { AccessoryScreen(it, direct = true) } ?: tvSettingsIntent(context)?.let { AccessoryScreen(it, direct = false) }
}
