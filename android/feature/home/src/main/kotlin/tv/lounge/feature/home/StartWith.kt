package tv.lounge.feature.home

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.SystemClock
import android.provider.Settings
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.runtime.Composable
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import tv.lounge.core.HomeSnapshot
import tv.lounge.core.Tile
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme
import tv.lounge.design.sounds

/** The row the TV's inputs are tiles in. */
private const val INPUTS_ROW = "tvcontrols"

/** The TV's inputs as the home has them, shown or not. A control in their row is not one. */
internal fun inputsOf(snapshot: HomeSnapshot): List<Tile> =
    (snapshot.rows + snapshot.hiddenRows).firstOrNull { it.id == INPUTS_ROW }?.tiles.orEmpty().filter { !it.action.startsWith("control:") }

/** Whether [action] is still one of the TV's inputs or apps. An input that is unplugged is not, until it is back. */
internal fun canStartWith(snapshot: HomeSnapshot, action: String): Boolean =
    inputsOf(snapshot).any { it.action == action } || snapshot.allApps.any { it.action == action }

/**
 * Each time the TV starts or wakes: once for the first run since it was switched on, then once
 * for every time the screen comes back on. Leaving the screensaver is not one.
 */
internal fun Context.starts(): Flow<Unit> = callbackFlow {
    if (firstRunSinceBoot()) trySend(Unit)
    val woke = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent?) {
            trySend(Unit)
        }
    }
    val registered = runCatching { ContextCompat.registerReceiver(this@starts, woke, IntentFilter(Intent.ACTION_SCREEN_ON), ContextCompat.RECEIVER_NOT_EXPORTED) }.isSuccess
    awaitClose { if (registered) runCatching { unregisterReceiver(woke) } }
}

/**
 * Whether the TV has just been switched on and this is the first time since that it is asked.
 * Lounge first opened long after, by the Home button, is not a start. A TV that keeps no count says no.
 */
private fun Context.firstRunSinceBoot(): Boolean {
    val boots = runCatching { Settings.Global.getInt(contentResolver, Settings.Global.BOOT_COUNT) }.getOrNull() ?: return false
    val seen = getSharedPreferences("start_with", Context.MODE_PRIVATE)
    if (seen.getInt("boot", -1) == boots) return false
    seen.edit().putInt("boot", boots).apply()
    return SystemClock.elapsedRealtime() < JUST_SWITCHED_ON_MS
}

/** How long after switching on the TV still counts as starting. */
private const val JUST_SWITCHED_ON_MS = 180_000L

/**
 * What opens when the TV starts: Lounge's home, one of the TV's inputs or an app. The one in
 * use carries the word for it, and choosing another goes back to Settings.
 */
@Composable
fun StartWithSheet(snapshot: HomeSnapshot, chosen: String?, chosenTitle: String?, onChoose: (action: String?, title: String?) -> Unit, firstFocus: FocusRequester) {
    val colors = LoungeTheme.colors
    val sounds = LoungeTheme.sounds
    val inUse = stringResource(R.string.sheets_language_in_use)
    val inputs = inputsOf(snapshot)
    val apps = snapshot.allApps
    // A choice the TV does not have now is listed by the name it was chosen under, so it can be seen and changed.
    val known = chosen?.takeIf { canStartWith(snapshot, it) }
    val missing = chosenTitle?.takeIf { chosen != null && known == null }
    SheetHeading(stringResource(R.string.sheets_start_title), stringResource(R.string.start_with_about))
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        SheetRow(
            icon = SettingsGlyphs.Home,
            title = stringResource(R.string.start_with_home),
            subtitle = stringResource(R.string.start_with_home_about),
            value = inUse.takeIf { chosen == null },
            accent = colors.signal,
            onClick = { sounds?.tick(); onChoose(null, null) },
            focusRequester = firstFocus.takeIf { chosen == null },
        )
        missing?.let { title ->
            SheetRow(
                icon = LoungeIcons.Info,
                title = title,
                subtitle = stringResource(R.string.start_with_missing),
                value = inUse,
                accent = colors.signal,
                onClick = {},
                focusRequester = firstFocus,
            )
        }
        @Composable
        fun choices(label: Int, tiles: List<Tile>, icon: androidx.compose.ui.graphics.vector.ImageVector) {
            if (tiles.isEmpty()) return
            SheetGroup(stringResource(label))
            tiles.forEach { tile ->
                SheetRow(
                    icon = icon,
                    title = tile.title,
                    value = inUse.takeIf { chosen == tile.action },
                    accent = colors.signal,
                    onClick = { sounds?.tick(); onChoose(tile.action, tile.title) },
                    focusRequester = firstFocus.takeIf { known == tile.action },
                )
            }
        }
        choices(R.string.start_with_inputs, inputs, LoungeIcons.Hdmi)
        choices(R.string.start_with_apps, apps, LoungeIcons.Apps)
    }
}
