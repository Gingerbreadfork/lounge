package tv.lounge.feature.home

import android.content.Context
import android.content.Intent
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.provider.Settings
import androidx.compose.runtime.Composable
import androidx.compose.runtime.State
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.repeatOnLifecycle
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map

/** How long the TV must be without a network before the top bar says so. Wi-Fi takes a moment to come back after boot or sleep. */
internal const val OFFLINE_GRACE_MS = 10_000L

/** How the TV reaches the internet. */
internal enum class Link { None, Wifi, Ethernet, Other }

/**
 * The link a network gives. Android's own test of the internet is not asked for, since a home
 * network may block it.
 */
internal fun linkOf(capabilities: NetworkCapabilities?): Link = when {
    capabilities == null || !capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) -> Link.None
    capabilities.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) -> Link.Ethernet
    capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) -> Link.Wifi
    else -> Link.Other
}

/** The link in use now. A TV that cannot be asked is taken to have one. */
private fun Context.link(): Link = runCatching {
    val manager = getSystemService(ConnectivityManager::class.java) ?: return@runCatching Link.Other
    linkOf(manager.getNetworkCapabilities(manager.activeNetwork))
}.getOrDefault(Link.Other)

/** [link] now, and again each time it changes. */
private fun Context.links(): Flow<Link> = callbackFlow {
    val manager = getSystemService(ConnectivityManager::class.java)
    val callback = object : ConnectivityManager.NetworkCallback() {
        private var current: Network? = null

        override fun onCapabilitiesChanged(network: Network, capabilities: NetworkCapabilities) {
            current = network
            trySend(linkOf(capabilities))
        }

        // Only the network in use counts: one left behind for another may be reported lost later.
        override fun onLost(network: Network) {
            if (network != current) return
            current = null
            trySend(Link.None)
        }
    }
    // The callback says nothing while there is no network, so the first answer is read here.
    trySend(link())
    val registered = manager != null && runCatching { manager.registerDefaultNetworkCallback(callback) }.isSuccess
    awaitClose { if (registered) runCatching { manager?.unregisterNetworkCallback(callback) } }
}.distinctUntilChanged()

/** The TV's link now, followed while Lounge is on screen. */
@Composable
internal fun rememberLink(): State<Link> {
    val context = LocalContext.current.applicationContext
    return remember(context) { context.links() }.collectAsStateWithLifecycle(initialValue = remember(context) { context.link() })
}

/**
 * True once the TV has been without a network for [OFFLINE_GRACE_MS] with Lounge on screen, and
 * false as soon as one is back.
 */
@Composable
internal fun rememberOffline(): State<Boolean> {
    val context = LocalContext.current.applicationContext
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    return produceState(false, context, lifecycle) {
        lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
            context.links().map { it != Link.None }.distinctUntilChanged().collectLatest { online ->
                if (online) {
                    value = false
                } else if (!value) {
                    delay(OFFLINE_GRACE_MS)
                    value = true
                }
            }
        }
    }
}

private fun Context.settingsScreen(action: String): Intent? =
    Intent(action).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK).takeIf { runCatching { it.resolveActivity(packageManager) != null }.getOrDefault(false) }

/** The TV's own settings, or null on a TV with no such screen. */
internal fun tvSettingsIntent(context: Context): Intent? = context.settingsScreen(Settings.ACTION_SETTINGS)

/** The TV's network screen, or null on a TV with no such screen. */
internal fun networkSettingsIntent(context: Context): Intent? = context.settingsScreen(Settings.ACTION_WIFI_SETTINGS)
