package tv.lounge.feature.home

import androidx.annotation.StringRes
import tv.lounge.design.LoungeColors
import tv.lounge.design.LoungeThemeVariant
import tv.lounge.design.colors
import tv.lounge.platform.Prefs

/** The rooms (DESIGN.md §3.1), as Settings and onboarding name them. */
@get:StringRes
val Prefs.Theme.title: Int
    get() = when (this) {
        Prefs.Theme.Dusk -> R.string.themes_dusk_title
        Prefs.Theme.House -> R.string.themes_house_title
        Prefs.Theme.Hearth -> R.string.themes_hearth_title
        Prefs.Theme.Opera -> R.string.themes_opera_title
        Prefs.Theme.Marquee -> R.string.themes_marquee_title
        Prefs.Theme.GreenRoom -> R.string.themes_green_room_title
        Prefs.Theme.Moonlight -> R.string.themes_moonlight_title
        Prefs.Theme.Graphite -> R.string.themes_graphite_title
        Prefs.Theme.Cabaret -> R.string.themes_cabaret_title
        Prefs.Theme.Aquarium -> R.string.themes_aquarium_title
        Prefs.Theme.Absinthe -> R.string.themes_absinthe_title
        Prefs.Theme.Ultraviolet -> R.string.themes_ultraviolet_title
        Prefs.Theme.Blackout -> R.string.themes_blackout_title
        Prefs.Theme.HighContrast -> R.string.themes_high_contrast_title
    }

@get:StringRes
val Prefs.Theme.tagline: Int
    get() = when (this) {
        Prefs.Theme.Dusk -> R.string.themes_dusk_tagline
        Prefs.Theme.House -> R.string.themes_house_tagline
        Prefs.Theme.Hearth -> R.string.themes_hearth_tagline
        Prefs.Theme.Opera -> R.string.themes_opera_tagline
        Prefs.Theme.Marquee -> R.string.themes_marquee_tagline
        Prefs.Theme.GreenRoom -> R.string.themes_green_room_tagline
        Prefs.Theme.Moonlight -> R.string.themes_moonlight_tagline
        Prefs.Theme.Graphite -> R.string.themes_graphite_tagline
        Prefs.Theme.Cabaret -> R.string.themes_cabaret_tagline
        Prefs.Theme.Aquarium -> R.string.themes_aquarium_tagline
        Prefs.Theme.Absinthe -> R.string.themes_absinthe_tagline
        Prefs.Theme.Ultraviolet -> R.string.themes_ultraviolet_tagline
        Prefs.Theme.Blackout -> R.string.themes_blackout_tagline
        Prefs.Theme.HighContrast -> R.string.themes_high_contrast_tagline
    }

fun Prefs.Theme.variant(): LoungeThemeVariant = LoungeThemeVariant.valueOf(name)

fun Prefs.Theme.palette(): LoungeColors = variant().colors()
