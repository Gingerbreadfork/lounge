package tv.lounge.platform

import android.content.Context
import androidx.datastore.core.DataStore
import androidx.datastore.core.handlers.ReplaceFileCorruptionHandler
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.emptyPreferences
import androidx.datastore.preferences.core.intPreferencesKey
import androidx.datastore.preferences.core.longPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.core.stringSetPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map
import java.io.IOException

// A corrupt file starts over with the defaults instead of failing every read.
private val Context.dataStore: DataStore<Preferences> by preferencesDataStore(
    name = "lounge",
    corruptionHandler = ReplaceFileCorruptionHandler { emptyPreferences() },
)

/** Whether notifications show. Until it is chosen they show only on a TV still in setup, which is about to be asked. */
internal fun notificationsShown(chosen: Boolean?, setupDone: Boolean): Boolean = chosen ?: !setupDone

/**
 * What to store for a TV that finished setup with nothing chosen: off where notification
 * access was already given, on where it is still to be asked for. Null where there is nothing
 * to settle.
 */
internal fun settledNotifications(chosen: Boolean?, setupDone: Boolean, accessGiven: Boolean): Boolean? =
    if (chosen == null && setupDone) !accessGiven else null

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

    enum class Theme { Dusk, House, Hearth, Opera, Marquee, GreenRoom, Moonlight, Graphite, Cabaret, Aquarium, Absinthe, Ultraviolet, Blackout, HighContrast }

    /** Every UI preference at once, so the screen recomposes once per change rather than once per key. */
    data class UiPrefs(
        val theme: Theme = Theme.Dusk,
        val sounds: Boolean = true,
        val liteMode: Boolean? = null,
        val reducedMotion: Boolean? = null,
        val showMobileApps: Boolean = false,
        val onboardingDone: Boolean = false,
        /** The setup step reached, so setup left unfinished comes back where it was. */
        val onboardingStep: Int = 0,
        val companionEnabled: Boolean = false,
        /** The home's background: "still" for the light field, or the id of an abstract scene drawn live. */
        val backdrop: String = STILL_BACKDROP,
        /** How the All apps screen is sorted: "az", "opened" or "installed". */
        val allAppsSort: String = SORT_AZ,
        /** Whether apps' notifications are read and shown in the top bar. */
        val notifications: Boolean = true,
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
            onboardingStep = p[ONBOARDING_STEP] ?: 0,
            companionEnabled = p[COMPANION_ENABLED] ?: false,
            backdrop = p[BACKDROP] ?: STILL_BACKDROP,
            allAppsSort = p[ALL_APPS_SORT] ?: SORT_AZ,
            notifications = notificationsShown(p[NOTIFICATIONS], p[ONBOARDING_DONE] ?: false),
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

    /** How the home treats apps' notifications. */
    data class NotificationPrefs(
        val shown: Boolean = true,
        /** Packages whose notifications are left out. */
        val hiddenApps: Set<String> = emptySet(),
        /** When the panel was last looked at; a notification posted later is new. */
        val seenAt: Long = 0L,
    )

    val notifications: Flow<NotificationPrefs> = data.map { p ->
        NotificationPrefs(notificationsShown(p[NOTIFICATIONS], p[ONBOARDING_DONE] ?: false), p[NOTIFICATIONS_HIDDEN] ?: emptySet(), p[NOTIFICATIONS_SEEN_AT] ?: 0L)
    }
    suspend fun setNotifications(on: Boolean) { context.dataStore.edit { it[NOTIFICATIONS] = on } }

    /**
     * Settles notifications on a TV that finished setup before Lounge could show them, once.
     * With notification access already given, it was given for Now playing alone, so they stay
     * off until the person turns them on. Returns whether that is so, for the person to be told.
     */
    suspend fun settleNotifications(accessGiven: Boolean): Boolean {
        var toTell = false
        context.dataStore.edit { p ->
            val settled = settledNotifications(p[NOTIFICATIONS], p[ONBOARDING_DONE] ?: false, accessGiven) ?: return@edit
            p[NOTIFICATIONS] = settled
            toTell = !settled
        }
        return toTell
    }

    /** Marks setup finished. Its steps name what notification access covers, so notifications are on from here unless already chosen. */
    suspend fun finishSetup() {
        context.dataStore.edit { p ->
            p[ONBOARDING_DONE] = true
            if (p[NOTIFICATIONS] == null) p[NOTIFICATIONS] = true
        }
    }
    suspend fun setNotificationsHidden(packageName: String, hidden: Boolean) {
        context.dataStore.edit { p ->
            val now = p[NOTIFICATIONS_HIDDEN] ?: emptySet()
            p[NOTIFICATIONS_HIDDEN] = if (hidden) now + packageName else now - packageName
        }
    }
    suspend fun setNotificationsSeenAt(at: Long) { context.dataStore.edit { it[NOTIFICATIONS_SEEN_AT] = at } }

    /** The system screensaver component that was set before Lounge took over, to give back. */
    val previousScreensaver: Flow<String?> = data.map { it[PREVIOUS_SCREENSAVER] }

    /** The screensaver's style: "pictures", or the id of an abstract scene. */
    val ambientStyle: Flow<String> = data.map { it[AMBIENT_STYLE] ?: PICTURES }
    suspend fun setAmbientStyle(id: String) { context.dataStore.edit { it[AMBIENT_STYLE] = id } }
    suspend fun setBackdrop(id: String) { context.dataStore.edit { it[BACKDROP] = id } }
    suspend fun setAllAppsSort(sort: String) { context.dataStore.edit { it[ALL_APPS_SORT] = sort } }
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
    suspend fun setOnboardingStep(step: Int) { context.dataStore.edit { it[ONBOARDING_STEP] = step } }
    suspend fun setReducedMotion(on: Boolean?) { context.dataStore.edit { if (on == null) it.remove(REDUCED_MOTION) else it[REDUCED_MOTION] = on } }

    /**
     * The settings a backup carries, by name: how the home looks, sounds and is laid out. What
     * belongs to this TV alone stays behind, such as its setup, its phone and its screensaver.
     */
    suspend fun exported(): Map<String, String> {
        val p = data.first()
        return mapOf(
            THEME.name to runCatching { Theme.valueOf(p[THEME] ?: Theme.Dusk.name) }.getOrDefault(Theme.Dusk).name,
            SOUNDS.name to (p[SOUNDS] ?: true).toString(),
            LITE_MODE.name to (p[LITE_MODE]?.toString() ?: AUTOMATIC),
            REDUCED_MOTION.name to (p[REDUCED_MOTION]?.toString() ?: AUTOMATIC),
            SHOW_MOBILE_APPS.name to (p[SHOW_MOBILE_APPS] ?: false).toString(),
            BACKDROP.name to (p[BACKDROP] ?: STILL_BACKDROP),
            AMBIENT_STYLE.name to (p[AMBIENT_STYLE] ?: PICTURES),
            ALL_APPS_SORT.name to (p[ALL_APPS_SORT] ?: SORT_AZ),
            NOTIFICATIONS.name to notificationsShown(p[NOTIFICATIONS], p[ONBOARDING_DONE] ?: false).toString(),
            NOTIFICATIONS_HIDDEN.name to (p[NOTIFICATIONS_HIDDEN] ?: emptySet()).sorted().joinToString(","),
        )
    }

    /**
     * Sets each setting in [values] that [exported] names and whose value reads as one of its
     * own. Anything else in [values] is passed over, and a setting it leaves out stays as it is.
     */
    suspend fun restore(values: Map<String, String>) {
        context.dataStore.edit { p ->
            values[THEME.name]?.takeIf { name -> Theme.entries.any { it.name == name } }?.let { p[THEME] = it }
            values[SOUNDS.name]?.toBooleanStrictOrNull()?.let { p[SOUNDS] = it }
            values[SHOW_MOBILE_APPS.name]?.toBooleanStrictOrNull()?.let { p[SHOW_MOBILE_APPS] = it }
            values[NOTIFICATIONS.name]?.toBooleanStrictOrNull()?.let { p[NOTIFICATIONS] = it }
            values[NOTIFICATIONS_HIDDEN.name]?.let { list ->
                val apps = list.split(',').map { it.trim() }.filter { it.isNotEmpty() }
                if (apps.size <= MAX_HIDDEN_APPS && apps.all { PACKAGE_NAME.matches(it) }) p[NOTIFICATIONS_HIDDEN] = apps.toSet()
            }
            for (key in listOf(LITE_MODE, REDUCED_MOTION)) {
                val value = values[key.name] ?: continue
                if (value == AUTOMATIC) p.remove(key) else value.toBooleanStrictOrNull()?.let { p[key] = it }
            }
            for (key in listOf(BACKDROP, AMBIENT_STYLE)) {
                values[key.name]?.takeIf { it.isNotBlank() && it.length <= MAX_STYLE_ID }?.let { p[key] = it }
            }
            values[ALL_APPS_SORT.name]?.takeIf { it in setOf(SORT_AZ, SORT_OPENED, SORT_INSTALLED) }?.let { p[ALL_APPS_SORT] = it }
        }
    }

    companion object {
        const val STILL_BACKDROP = "still"
        const val SORT_AZ = "az"
        const val SORT_OPENED = "opened"
        const val SORT_INSTALLED = "installed"
        /** How a backup writes a setting that follows the TV: Lite mode and reduced motion left to it. */
        const val AUTOMATIC = "auto"
        private const val PICTURES = "pictures"
        /** The longest id a background or screensaver style has. */
        private const val MAX_STYLE_ID = 200
        private const val MAX_HIDDEN_APPS = 200
        private val PACKAGE_NAME = Regex("[A-Za-z][A-Za-z0-9_]*(\\.[A-Za-z0-9_]+)*")
        private val THEME = stringPreferencesKey("theme")
        private val SOUNDS = booleanPreferencesKey("sounds")
        private val LITE_MODE = booleanPreferencesKey("lite_mode")
        private val ONBOARDING_DONE = booleanPreferencesKey("onboarding_done")
        private val ONBOARDING_STEP = intPreferencesKey("onboarding_step")
        private val REDUCED_MOTION = booleanPreferencesKey("reduced_motion")
        private val SHOW_MOBILE_APPS = booleanPreferencesKey("show_mobile_apps")
        private val BUNDLED_PLUGINS_STAMP = longPreferencesKey("bundled_plugins_stamp")
        private val COMPANION_ENABLED = booleanPreferencesKey("companion_enabled")
        private val PREVIOUS_SCREENSAVER = stringPreferencesKey("previous_screensaver")
        private val PREVIOUS_SCREENSAVER_ENABLED = stringPreferencesKey("previous_screensaver_enabled")
        private val AMBIENT_STYLE = stringPreferencesKey("ambient_style")
        private val BACKDROP = stringPreferencesKey("backdrop")
        private val ALL_APPS_SORT = stringPreferencesKey("all_apps_sort")
        private val NOTIFICATIONS = booleanPreferencesKey("notifications")
        private val NOTIFICATIONS_HIDDEN = stringSetPreferencesKey("notifications_hidden")
        private val NOTIFICATIONS_SEEN_AT = longPreferencesKey("notifications_seen_at")
    }
}
