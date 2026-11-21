package tv.lounge.feature.home

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** The About page's words, where its code leads and how the code is laid out. */
class AboutTest {
    @Test
    fun theCodeLeadsToTheRepositoryFirstAndRoundToItAgain() {
        assertEquals("https://github.com/Gingerbreadfork/lounge", AboutLinks.first().code)
        // What is read under the code is the address without its scheme, and the address really leads there.
        for (link in AboutLinks) assertEquals(link.code, "https://${link.shown}")
        assertEquals(AboutLinks.size, AboutLinks.map { it.name }.toSet().size)
        assertEquals("https://x.com/gingerbreadfork", AboutLinks.single { it.name == "Gingerbreadfork on X" }.code)

        assertEquals(1, choiceAfter(0, 1, 4))
        assertEquals(0, choiceAfter(3, 1, 4))
        assertEquals(3, choiceAfter(0, -1, 4))
        assertEquals(0, choiceAfter(0, 4, 4))
    }

    @Test
    fun theVersionIsSaidPlainlyAndAnUnknownOneOwnedUpTo() {
        assertEquals("Version 0.9.0", aboutVersion("0.9.0"))
        assertEquals("Version unknown", aboutVersion(" "))
        assertEquals("plugin", AboutFact(1, "plugin", "plugins").label)
        assertEquals("plugins", AboutFact(27, "plugin", "plugins").label)
    }

    @Test
    fun aCodesModulesAreWholePixelsAndCentred() {
        // 344 px for 33 modules: ten each, with the seven left over shared between the sides.
        assertEquals(10f to 7f, qrLayout(344f, 33))
        assertEquals(13f to 9f, qrLayout(344f, 25))
        // A side too small for the code still draws something a pixel wide.
        assertEquals(1f, qrLayout(20f, 33).first)
    }

    @Test
    fun theThreeEyesSitInThreeCorners() {
        val n = 29
        assertTrue(inQrEye(0, 0, n) && inQrEye(6, 6, n) && inQrEye(n - 1, 0, n) && inQrEye(0, n - 1, n) && inQrEye(n - 7, 6, n))
        assertFalse(inQrEye(7, 0, n) || inQrEye(0, 7, n) || inQrEye(n - 1, n - 1, n) || inQrEye(n - 8, 0, n) || inQrEye(14, 14, n))
    }
}
