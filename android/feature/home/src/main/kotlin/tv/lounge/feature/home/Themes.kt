package tv.lounge.feature.home

import tv.lounge.design.LoungeColors
import tv.lounge.design.LoungeThemeVariant
import tv.lounge.design.colors
import tv.lounge.platform.Prefs

/** The ten rooms (DESIGN.md §3.1), as Settings and onboarding name them. */
val Prefs.Theme.title: String
    get() = when (this) {
        Prefs.Theme.Dusk -> "Dusk"
        Prefs.Theme.House -> "House"
        Prefs.Theme.Hearth -> "Hearth"
        Prefs.Theme.Opera -> "Opera"
        Prefs.Theme.Marquee -> "Marquee"
        Prefs.Theme.GreenRoom -> "Green room"
        Prefs.Theme.Moonlight -> "Moonlight"
        Prefs.Theme.Graphite -> "Graphite"
        Prefs.Theme.Blackout -> "Blackout"
        Prefs.Theme.HighContrast -> "High contrast"
    }

val Prefs.Theme.tagline: String
    get() = when (this) {
        Prefs.Theme.Dusk -> "Indigo night, coral lamp"
        Prefs.Theme.House -> "Warm plum, tungsten light"
        Prefs.Theme.Hearth -> "Umber walls, firelight"
        Prefs.Theme.Opera -> "Red velvet and gilt"
        Prefs.Theme.Marquee -> "Navy, marquee bulbs"
        Prefs.Theme.GreenRoom -> "Deep green, brass lamp"
        Prefs.Theme.Moonlight -> "Cool slate, moonlight"
        Prefs.Theme.Graphite -> "No colour, warm white"
        Prefs.Theme.Blackout -> "True black for OLED panels"
        Prefs.Theme.HighContrast -> "Bolder text and edges"
    }

fun Prefs.Theme.variant(): LoungeThemeVariant = LoungeThemeVariant.valueOf(name)

fun Prefs.Theme.palette(): LoungeColors = variant().colors()
