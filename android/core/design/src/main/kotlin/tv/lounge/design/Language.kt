package tv.lounge.design

import android.content.ContextWrapper
import android.content.res.Configuration
import android.content.res.Resources
import android.os.LocaleList
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.unit.LayoutDirection
import java.util.Locale

/** The languages Lounge is written in, and the one the person chose over the TV's own. */
object Language {
    /** A language Lounge ships: its tag, and its name in its own words. */
    data class Shipped(val tag: String, val name: String)

    /** English first, then by name. Each has string resources here and catalogs in the core. */
    val shipped = listOf(
        Shipped("en", "English"),
        Shipped("de", "Deutsch"),
        Shipped("es", "Español"),
        Shipped("fr", "Français"),
        Shipped("it", "Italiano"),
        Shipped("nl", "Nederlands"),
        Shipped("pt-BR", "Português (Brasil)"),
        Shipped("pt-PT", "Português (Portugal)"),
    )

    /** The tag the person chose, or null while Lounge follows the TV. */
    @Volatile
    var chosen: String? = null
        private set

    /** Puts [tag] in use everywhere text is read; null, or a language Lounge does not ship, follows the TV. */
    fun choose(tag: String?) {
        val known = tag?.takeIf { wanted -> shipped.any { it.tag == wanted } }
        if (known == chosen) return
        chosen = known
        Locale.setDefault(known?.let(Locale::forLanguageTag) ?: systemLocales()[0])
    }

    /** [system] with the languages text is read in: the chosen one, or the TV's own. */
    fun configuration(system: Configuration): Configuration =
        Configuration(system).apply { setLocales(chosen?.let { LocaleList(Locale.forLanguageTag(it)) } ?: systemLocales()) }

    private fun systemLocales(): LocaleList = Resources.getSystem().configuration.locales
}

/**
 * A context's resources in the chosen language. An activity, service or application returns
 * [over] from `getResources`, so every string it reads follows the choice.
 */
class SpokenResources(private val context: ContextWrapper) {
    @Volatile private var made: Pair<String, Resources>? = null

    /** [base] while Lounge follows the TV, else resources in the chosen language. */
    fun over(base: Resources): Resources {
        val tag = Language.chosen ?: return base
        made?.let { (madeFor, resources) -> if (madeFor == tag) return resources }
        val chosen = Configuration().apply { setLocale(Locale.forLanguageTag(tag)) }
        return context.baseContext.createConfigurationContext(chosen).resources.also { made = tag to it }
    }
}

/**
 * Shows [content] in the language [tag] names, or the TV's own for null. Text already on screen
 * follows a change at once. The host activity must read its resources through [SpokenResources].
 */
@Composable
fun InLanguage(tag: String?, content: @Composable () -> Unit) {
    val system = LocalConfiguration.current
    val spoken = remember(system, tag) {
        Language.choose(tag)
        Language.configuration(system)
    }
    val direction = if (spoken.layoutDirection == android.view.View.LAYOUT_DIRECTION_RTL) LayoutDirection.Rtl else LayoutDirection.Ltr
    CompositionLocalProvider(LocalConfiguration provides spoken, LocalLayoutDirection provides direction, content = content)
}
