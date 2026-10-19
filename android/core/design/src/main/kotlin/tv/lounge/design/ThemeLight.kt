package tv.lounge.design

import androidx.compose.runtime.Composable
import androidx.compose.runtime.staticCompositionLocalOf

/** Convenience accessor so features never construct a second [LightState]. */
val LoungeTheme.lightState: LightState
    @Composable get() = LocalLightState.current

/** The UI sound player, provided by the host activity; null when sounds are unavailable. */
val LocalUiSounds = staticCompositionLocalOf<UiSounds?> { null }

val LoungeTheme.sounds: UiSounds?
    @Composable get() = LocalUiSounds.current
