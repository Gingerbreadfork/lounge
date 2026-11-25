package tv.lounge.feature.home

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.provider.Settings

/** Some TVs' list of the accessories connected to them, with the way to pair another. */
private const val ACTION_CONNECTED_ACCESSORIES = "com.android.tv.settings.ACCESSORIES_CONNECTED_DEVICES"

/** Android TV's screen that looks for a remote, a controller or headphones to pair. */
private const val ACTION_PAIR_ACCESSORY = "com.google.android.intent.action.CONNECT_INPUT"

/** What some TVs want an app to hold before it may open their pairing screen. Android calls it Nearby devices. */
internal const val NEARBY_DEVICES = android.Manifest.permission.BLUETOOTH_CONNECT

/**
 * A screen of the TV's own settings, with the permission Lounge still lacks to open it, if any.
 * Null on a TV with no such screen, or one no other app may open.
 */
private fun Context.findScreen(action: String): Pair<Intent, String?>? {
    val intent = Intent(action).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    val screen = runCatching { packageManager.resolveActivity(intent, PackageManager.MATCH_DEFAULT_ONLY)?.activityInfo }.getOrNull() ?: return null
    // Android's own chooser stands in when several screens answer, and it opens for anyone.
    if (screen.packageName == "android") return intent to null
    if (!screen.exported) return null
    val held = screen.permission == null || checkSelfPermission(screen.permission) == PackageManager.PERMISSION_GRANTED
    return intent to screen.permission.takeIf { !held }
}

/** A screen of the TV's own settings that Lounge can open now, or null. */
private fun Context.settingsScreen(action: String): Intent? = findScreen(action)?.takeIf { it.second == null }?.first

/** The TV's own settings, or null on a TV with no such screen. */
internal fun tvSettingsIntent(context: Context): Intent? = context.settingsScreen(Settings.ACTION_SETTINGS)

/** The TV's network screen, or null on a TV with no such screen. */
internal fun networkSettingsIntent(context: Context): Intent? = context.settingsScreen(Settings.ACTION_WIFI_SETTINGS)

/**
 * Where the TV pairs remotes, controllers and headphones. [open] is its own screen for them, when
 * Lounge can open it now. [locked] is that screen when it first wants [NEARBY_DEVICES], and
 * [settings] is all of the TV's settings, to fall back on.
 */
internal class AccessoryScreen(val open: Intent?, val locked: Intent?, val settings: Intent?)

/**
 * The TV's screen for accessories: its list of them before the one that pairs a new one. Null on
 * a TV with neither and no settings to open in their place.
 */
internal fun accessoryScreen(context: Context): AccessoryScreen? {
    val own = listOf(ACTION_CONNECTED_ACCESSORIES, ACTION_PAIR_ACCESSORY).mapNotNull { context.findScreen(it) }
    val open = own.firstOrNull { it.second == null }?.first
    val locked = own.firstOrNull { it.second == NEARBY_DEVICES }?.first.takeIf { open == null }
    val settings = tvSettingsIntent(context)
    return AccessoryScreen(open, locked, settings).takeIf { open != null || locked != null || settings != null }
}
