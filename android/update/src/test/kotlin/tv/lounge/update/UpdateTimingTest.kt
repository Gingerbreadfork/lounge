package tv.lounge.update

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** When the background pass looks for a release, and when it tries again after a failure. */
class UpdateTimingTest {
    private val hour = 3_600_000L
    private val now = 100 * hour

    @Test
    fun aDueCheckWaitsForANetworkAndIsStillDueWhenThereIsOne() {
        assertTrue(UpdateManager.checkDue(now, now - 13 * hour, online = true))
        assertFalse(UpdateManager.checkDue(now, now - 11 * hour, online = true))
        // A sleeping TV has no network: nothing is tried, so nothing is recorded as failed.
        assertFalse(UpdateManager.checkDue(now, now - 13 * hour, online = false))
        assertFalse(UpdateManager.checkDue(now, 0, online = false))
        assertTrue(UpdateManager.checkDue(now, 0, online = true))
    }

    @Test
    fun aFailureForWantOfANetworkIsTriedAgainAsSoonAsThereIsOne() {
        val justNow = now - 60_000L
        assertTrue(UpdateManager.retryDue(now, justNow, online = true, failedOffline = true))
        assertFalse(UpdateManager.retryDue(now, justNow, online = false, failedOffline = true))
        // Any other failure waits its hour, so a server that is refusing is not asked again and again.
        assertFalse(UpdateManager.retryDue(now, justNow, online = true, failedOffline = false))
        assertTrue(UpdateManager.retryDue(now, now - 2 * hour, online = true, failedOffline = false))
        assertFalse(UpdateManager.retryDue(now, now - 2 * hour, online = false, failedOffline = false))
    }
}
