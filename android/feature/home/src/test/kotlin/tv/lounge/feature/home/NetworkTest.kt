package tv.lounge.feature.home

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import androidx.compose.runtime.State
import androidx.compose.ui.test.junit4.createComposeRule
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.shadows.ShadowNetwork
import org.robolectric.shadows.ShadowNetworkCapabilities

/** The top bar says the TV has no network only once it has been without one for a while. */
@RunWith(RobolectricTestRunner::class)
class NetworkTest {
    @get:Rule val rule = createComposeRule()

    private val context: Context = RuntimeEnvironment.getApplication()
    private val manager = context.getSystemService(ConnectivityManager::class.java)
    private val network = manager.activeNetwork!!
    private val internet: NetworkCapabilities = ShadowNetworkCapabilities.newInstance().also {
        shadowOf(it).addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
    }
    private lateinit var offline: State<Boolean>

    private fun show() {
        rule.mainClock.autoAdvance = false
        rule.setContent { offline = rememberOffline() }
        rule.mainClock.advanceTimeByFrame()
    }

    @Before
    fun startOnline() {
        shadowOf(manager).setNetworkCapabilities(network, internet)
    }

    /** A network becomes the one in use, as Android reports it. */
    private fun use(now: Network) {
        shadowOf(manager).networkCallbacks.forEach { it.onCapabilitiesChanged(now, internet) }
        rule.mainClock.advanceTimeByFrame()
    }

    private fun lose(gone: Network = network) {
        shadowOf(manager).networkCallbacks.forEach { it.onLost(gone) }
        rule.mainClock.advanceTimeByFrame()
    }

    private fun regain() = use(network)

    @Test
    fun aLostNetworkIsSaidAfterTheWaitAndForgottenAtOnce() {
        show()
        use(network)
        assertEquals(false, offline.value)
        lose()
        rule.mainClock.advanceTimeBy(OFFLINE_GRACE_MS - 1_000)
        assertEquals(false, offline.value)
        rule.mainClock.advanceTimeBy(2_000)
        assertEquals(true, offline.value)
        regain()
        assertEquals(false, offline.value)
    }

    @Test
    fun aNetworkThatComesBackWithinTheWaitIsNeverSaid() {
        show()
        use(network)
        lose()
        rule.mainClock.advanceTimeBy(OFFLINE_GRACE_MS / 2)
        regain()
        rule.mainClock.advanceTimeBy(OFFLINE_GRACE_MS)
        assertEquals(false, offline.value)
    }

    @Test
    fun theNetworkLeftBehindForAnotherIsNotMissed() {
        show()
        use(network)
        use(ShadowNetwork.newInstance(7))
        lose(network)
        rule.mainClock.advanceTimeBy(OFFLINE_GRACE_MS + 1_000)
        assertEquals(false, offline.value)
    }

    @Test
    fun aTvThatStartsWithNoNetworkSaysSoAfterTheWait() {
        shadowOf(manager).setActiveNetworkInfo(null)
        show()
        assertEquals(false, offline.value)
        rule.mainClock.advanceTimeBy(OFFLINE_GRACE_MS + 1_000)
        assertEquals(true, offline.value)
    }

    private fun reach(vararg transports: Int, internet: Boolean = true): NetworkCapabilities = ShadowNetworkCapabilities.newInstance().also {
        if (internet) shadowOf(it).addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
        transports.forEach { t -> shadowOf(it).addTransportType(t) }
    }

    @Test
    fun theLinkIsNamedByHowTheTvIsConnected() {
        assertEquals(Link.Wifi, linkOf(reach(NetworkCapabilities.TRANSPORT_WIFI)))
        assertEquals(Link.Ethernet, linkOf(reach(NetworkCapabilities.TRANSPORT_ETHERNET)))
        assertEquals(Link.Wifi, linkOf(reach(NetworkCapabilities.TRANSPORT_VPN, NetworkCapabilities.TRANSPORT_WIFI)))
        assertEquals(Link.Other, linkOf(reach(NetworkCapabilities.TRANSPORT_BLUETOOTH)))
        assertEquals(Link.None, linkOf(reach(NetworkCapabilities.TRANSPORT_WIFI, internet = false)))
        assertEquals(Link.None, linkOf(null))
    }
}
