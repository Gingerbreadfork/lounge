package tv.lounge.design

import android.content.ContextWrapper
import java.util.Locale
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

/** A language chosen over the TV's own is the one every resource is read in, until the TV is followed again. */
@RunWith(RobolectricTestRunner::class)
class LanguageTest {
    private val app get() = RuntimeEnvironment.getApplication()

    @After
    fun followTheTv() = Language.choose(null)

    @Test
    fun aChosenLanguageIsReadThroughTheResources() {
        val spoken = SpokenResources(ContextWrapper(app))
        fun tag() = spoken.over(app.resources).getString(R.string.language_tag)
        assertSame(app.resources, spoken.over(app.resources))
        assertEquals("en", tag())

        Language.choose("de")
        assertEquals("de", tag())
        assertEquals("An", spoken.over(app.resources).getString(R.string.common_on))
        assertEquals("vor 3 Minuten", Words.ago(spoken.over(app.resources), 3 * 60_000L))
        assertEquals(Locale.GERMAN, Locale.getDefault())
        assertEquals("de", Language.configuration(app.resources.configuration).locales[0].toLanguageTag())

        // Portugal's Portuguese and Brazil's are two languages here.
        Language.choose("pt-PT")
        assertEquals("pt-PT", tag())
        Language.choose("pt-BR")
        assertEquals("pt-BR", tag())

        Language.choose(null)
        assertSame(app.resources, spoken.over(app.resources))
        assertEquals("en", Language.configuration(app.resources.configuration).locales[0].language)
    }

    @Test
    fun aLanguageLoungeIsNotWrittenInFollowsTheTv() {
        Language.choose("de")
        Language.choose("ja")
        assertNull(Language.chosen)
        assertSame(app.resources, SpokenResources(ContextWrapper(app)).over(app.resources))
    }
}
