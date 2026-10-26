package tv.lounge.platform

import android.content.Context
import androidx.datastore.core.DataStore
import androidx.datastore.core.handlers.ReplaceFileCorruptionHandler
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.emptyPreferences
import androidx.datastore.preferences.core.longPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.map
import java.io.IOException

// A corrupt file starts over with the defaults instead of failing every read.
private val Context.dataStore: DataStore<Preferences> by preferencesDataStore(
    name = "lounge",
    corruptionHandler = ReplaceFileCorruptionHandler { emptyPreferences() },
)

/** UI-only preferences. Durable data lives in the core. */
class Prefs(private val context: Context) {
    /** The stored preferences, or the defaults while the file cannot be read. */
    private val data: Flow<Preferences> = context.dataStore.data.catch { e ->
        if (e is IOException) {
            android.util.Log.w("Lounge", "preferences unreadable, using the defaults", e)
            emit(emptyPreferences())
        } else {
            throw e
        }
    }

    enum class Theme { Dusk, House, Hearth, Opera, Marquee, GreenRoom, Moonlight, Graphite, Blackout, HighContrast }

    /** Every UI preference at once, so the screen recomposes once per change rather than once per key. */
    data class UiPrefs(
        val theme: Theme = Theme.Dusk,
        val sounds: Boolean = true,
        val liteMode: Boolean? = null,
        val reducedMotion: Boolean? = null,
        val showMobileApps: Boolean = false,
        val onboardingDone: Boolean = false,
        val companionEnabled: Boolean = false,
        /** The home's background: "still" for the light field, or the id of an abstract scene drawn live. */
        val backdrop: String = STILL_BACKDROP,
        /** False only for the placeholder used before the store has been read. */
        val loaded: Boolean = false,
    )

    val all: Flow<UiPrefs> = data.map { p ->
        UiPrefs(
            theme = runCatching { Theme.valueOf(p[THEME] ?: Theme.Dusk.name) }.getOrDefault(Theme.Dusk),
            sounds = p[SOUNDS] ?: true,
            liteMode = p[LITE_MODE],
            reducedMotion = p[REDUCED_MOTION],
            showMobileApps = p[SHOW_MOBILE_APPS] ?: false,
            onboardingDone = p[ONBOARDING_DONE] ?: false,
            companionEnabled = p[COMPANION_ENABLED] ?: false,
            backdrop = p[BACKDROP] ?: STILL_BACKDROP,
            loaded = true,
        )
    }

    val theme: Flow<Theme> = data.map { p ->
        runCatching { Theme.valueOf(p[THEME] ?: Theme.Dusk.name) }.getOrDefault(Theme.Dusk)
    }
    val sounds: Flow<Boolean> = data.map { it[SOUNDS] ?: true }
    val liteMode: Flow<Boolean?> = data.map { it[LITE_MODE] }
    val onboardingDone: Flow<Boolean> = data.map { it[ONBOARDING_DONE] ?: false }
    val reducedMotionOverride: Flow<Boolean?> = data.map { it[REDUCED_MOTION] }
    val showMobileApps: Flow<Boolean> = data.map { it[SHOW_MOBILE_APPS] ?: false }

    val companionEnabled: Flow<Boolean> = data.map { it[COMPANION_ENABLED] ?: false }

    /** The system screensaver component that was set before Lounge took over, to give back. */
    val previousScreensaver: Flow<String?> = data.map { it[PREVIOUS_SCREENSAVER] }

    /** The screensaver's style: "pictures", or the id of an abstract scene. */
    val ambientStyle: Flow<String> = data.map { it[AMBIENT_STYLE] ?: "pictures" }
    suspend fun setAmbientStyle(id: String) { context.dataStore.edit { it[AMBIENT_STYLE] = id } }
    suspend fun setBackdrop(id: String) { context.dataStore.edit { it[BACKDROP] = id } }
    suspend fun setPreviousScreensaver(component: String?) {
        context.dataStore.edit { if (component.isNullOrBlank()) it.remove(PREVIOUS_SCREENSAVER) else it[PREVIOUS_SCREENSAVER] = component }
    }

    /** The screensaver's enabled flag before Lounge took over: "1", "0", "null" for unset, or null when unknown. */
    val previousScreensaverEnabled: Flow<String?> = data.map { it[PREVIOUS_SCREENSAVER_ENABLED] }
    suspend fun setPreviousScreensaverEnabled(value: String?) {
        context.dataStore.edit { if (value == null) it.remove(PREVIOUS_SCREENSAVER_ENABLED) else it[PREVIOUS_SCREENSAVER_ENABLED] = value }
    }
    suspend fun setCompanionEnabled(on: Boolean) { context.dataStore.edit { it[COMPANION_ENABLED] = on } }

    /** The APK update time the bundled plugins were last installed from; they re-install after an update. */
    val bundledPluginsStamp: Flow<Long> = data.map { it[BUNDLED_PLUGINS_STAMP] ?: 0L }
    suspend fun setBundledPluginsStamp(stamp: Long) { context.dataStore.edit { it[BUNDLED_PLUGINS_STAMP] = stamp } }

    suspend fun setShowMobileApps(on: Boolean) { context.dataStore.edit { it[SHOW_MOBILE_APPS] = on } }
    suspend fun setTheme(theme: Theme) { context.dataStore.edit { it[THEME] = theme.name } }
    suspend fun setSounds(on: Boolean) { context.dataStore.edit { it[SOUNDS] = on } }
    suspend fun setLiteMode(on: Boolean?) { context.dataStore.edit { if (on == null) it.remove(LITE_MODE) else it[LITE_MODE] = on } }
    suspend fun setOnboardingDone(done: Boolean) { context.dataStore.edit { it[ONBOARDING_DONE] = done } }
    suspend fun setReducedMotion(on: Boolean?) { context.dataStore.edit { if (on == null) it.remove(REDUCED_MOTION) else it[REDUCED_MOTION] = on } }

    companion object {
        const val STILL_BACKDROP = "still"
        private val THEME = stringPreferencesKey("theme")
        private val SOUNDS = booleanPreferencesKey("sounds")
        private val LITE_MODE = booleanPreferencesKey("lite_mode")
        private val ONBOARDING_DONE = booleanPreferencesKey("onboarding_done")
        private val REDUCED_MOTION = booleanPreferencesKey("reduced_motion")
        private val SHOW_MOBILE_APPS = booleanPreferencesKey("show_mobile_apps")
        private val BUNDLED_PLUGINS_STAMP = longPreferencesKey("bundled_plugins_stamp")
        private val COMPANION_ENABLED = booleanPreferencesKey("companion_enabled")
        private val PREVIOUS_SCREENSAVER = stringPreferencesKey("previous_screensaver")
        private val PREVIOUS_SCREENSAVER_ENABLED = stringPreferencesKey("previous_screensaver_enabled")
        private val AMBIENT_STYLE = stringPreferencesKey("ambient_style")
        private val BACKDROP = stringPreferencesKey("backdrop")
    }
}
