package tv.lounge.platform

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Whether notifications show before anyone has chosen, and what an older install is settled to. */
class NotificationsDefaultTest {
    @Test
    fun aChoiceStandsAndWithoutOneTheyShowOnlyUntilSetupIsFinished() {
        assertTrue(notificationsShown(chosen = true, setupDone = true))
        assertFalse(notificationsShown(chosen = false, setupDone = false))
        // A TV in setup is about to be told what the access covers.
        assertTrue(notificationsShown(chosen = null, setupDone = false))
        // One set up before that reads nothing until it is settled.
        assertFalse(notificationsShown(chosen = null, setupDone = true))
    }

    @Test
    fun anOlderInstallIsOffWhereAccessWasAlreadyGivenAndOnWhereItIsStillToBeAskedFor() {
        assertEquals(false, settledNotifications(chosen = null, setupDone = true, accessGiven = true))
        assertEquals(true, settledNotifications(chosen = null, setupDone = true, accessGiven = false))
        // Nothing is settled twice, nor for a TV still in setup.
        assertNull(settledNotifications(chosen = false, setupDone = true, accessGiven = true))
        assertNull(settledNotifications(chosen = true, setupDone = true, accessGiven = true))
        assertNull(settledNotifications(chosen = null, setupDone = false, accessGiven = true))
    }
}
