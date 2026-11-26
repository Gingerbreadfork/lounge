package tv.lounge.feature.home

import android.content.Context
import androidx.annotation.StringRes
import kotlinx.coroutines.flow.first
import org.json.JSONArray
import org.json.JSONObject
import tv.lounge.core.CoreClient
import tv.lounge.core.LayoutCommand
import tv.lounge.core.PlatformException
import tv.lounge.design.R as DesignR
import tv.lounge.platform.AppNotification
import tv.lounge.platform.NotificationSource
import tv.lounge.platform.Prefs

/**
 * One of the TV's settings as a paired phone shows it: its words, what it is set to and what it
 * can be set to. [switch] is one that is simply on or off.
 */
internal data class PhoneSetting(val key: String, val title: String, val about: String?, val value: String, val choices: List<Pair<String, String>>, val switch: Boolean = false)

/**
 * The TV's settings for a paired phone, in the groups Settings on the TV has them, and setting
 * one. The words come from [context], so they are in the TV's language. What the core keeps is
 * reached through [appsOnly], [setAppsOnly] and [scenes].
 */
class PhoneSettings(
    private val context: Context,
    private val prefs: Prefs,
    private val appsOnly: () -> Boolean,
    private val setAppsOnly: (Boolean) -> Unit,
    private val scenes: () -> List<ShaderStyle>,
    private val clock: () -> Long = android.os.SystemClock::elapsedRealtime,
) {
    constructor(context: Context, prefs: Prefs, core: CoreClient) :
        this(context, prefs, { core.appsOnly() }, { core.command(LayoutCommand.SetAppsOnly(it)) }, { ShaderStyles.all(context, core) })

    // The core takes a change in its own time, so what was just asked for stands in until it has.
    @Volatile private var asked: Pair<Boolean, Long>? = null

    private fun appsOnlyNow(): Boolean {
        val kept = appsOnly()
        val wanted = asked ?: return kept
        if (kept == wanted.first || clock() - wanted.second > CORE_CATCHES_UP_MS) asked = null
        return if (asked == null) kept else wanted.first
    }

    private fun text(@StringRes id: Int) = context.getString(id)
    private val onOff get() = listOf("true" to text(DesignR.string.common_on), "false" to text(DesignR.string.common_off))

    /** The groups and their settings as they stand now. Reads the store and the core, so call it off the main thread. */
    internal suspend fun groups(): List<Pair<String, List<PhoneSetting>>> {
        val now = prefs.exported()
        fun switch(key: String, @StringRes title: Int, @StringRes about: Int?) = PhoneSetting(key, text(title), about?.let(::text), now[key] ?: "false", onOff, switch = true)
        fun choice(key: String, @StringRes title: Int, @StringRes about: Int?, choices: List<Pair<String, String>>) =
            PhoneSetting(key, text(title), about?.let(::text), now[key].orEmpty(), choices)
        val themes = Prefs.Theme.entries.map { it.name to text(it.title) }
        val backgrounds = listOf(Prefs.STILL_BACKDROP to text(R.string.sheets_background_still)) + scenes().map { it.id to it.nameIn(context.resources) }
        val sizes = listOf("Normal" to text(R.string.sheets_size_normal), "Large" to text(R.string.sheets_size_large), "Largest" to text(R.string.sheets_size_largest))
        val holds = listOf("Short" to text(R.string.sheets_hold_time_short), "Normal" to text(R.string.sheets_hold_time_normal), "Long" to text(R.string.sheets_hold_time_long))
        fun following(@StringRes follows: Int) = listOf(Prefs.AUTOMATIC to text(follows)) + onOff
        return listOf(
            text(R.string.sheets_group_home) to listOf(
                switch(Prefs.KEY_SHOW_STAGE, R.string.sheets_stage_title, R.string.sheets_stage_subtitle),
                PhoneSetting(APPS_ONLY, text(R.string.sheets_apps_only_title), text(R.string.sheets_apps_only_subtitle), appsOnlyNow().toString(), onOff, switch = true),
                switch(Prefs.KEY_SHOW_MOBILE_APPS, R.string.sheets_phone_apps_title, R.string.sheets_phone_apps_subtitle),
                switch(Prefs.KEY_NOTIFICATIONS, DesignR.string.common_notifications, R.string.sheets_notifications_subtitle),
            ),
            text(R.string.sheets_section_look) to listOf(
                choice(Prefs.KEY_THEME, R.string.sheets_theme_title, R.string.sheets_theme_subtitle, themes),
                choice(Prefs.KEY_BACKDROP, R.string.sheets_background_title, null, backgrounds),
                choice(Prefs.KEY_TEXT_SIZE, R.string.sheets_text_size_title, R.string.sheets_text_size_subtitle, sizes),
                choice(Prefs.KEY_LARGE_TILES, R.string.sheets_tile_size_title, R.string.sheets_tile_size_subtitle, listOf("false" to sizes[0].second, "true" to sizes[1].second)),
                switch(Prefs.KEY_SHOW_CLOCK, R.string.sheets_clock_title, R.string.sheets_clock_subtitle),
                choice(Prefs.KEY_LITE_MODE, R.string.sheets_lite_title, R.string.sheets_lite_on, following(R.string.sheets_automatic)),
                choice(Prefs.KEY_REDUCED_MOTION, R.string.sheets_reduce_motion_title, R.string.sheets_reduce_motion_subtitle, following(R.string.sheets_follow_system)),
            ),
            text(R.string.sheets_section_remote) to listOf(
                choice(Prefs.KEY_HOLD_TIME, R.string.sheets_hold_time_title, R.string.sheets_hold_time_subtitle, holds),
                switch(Prefs.KEY_HOLD_HINTS, R.string.sheets_hold_hint_title, R.string.sheets_hold_hint_subtitle),
                switch(Prefs.KEY_SOUNDS, R.string.sheets_sounds_title, R.string.sheets_sounds_subtitle),
            ),
        )
    }

    /** The groups as JSON for the phone. */
    suspend fun json(): String = JSONObject().put(
        "groups",
        JSONArray(
            groups().map { (title, settings) ->
                JSONObject().put("title", title).put(
                    "settings",
                    JSONArray(
                        settings.map { s ->
                            JSONObject().put("key", s.key).put("title", s.title).put("about", s.about ?: JSONObject.NULL).put("value", s.value).put("switch", s.switch)
                                .put("choices", JSONArray(s.choices.map { (value, label) -> JSONObject().put("value", value).put("label", label) }))
                        },
                    ),
                )
            },
        ),
    ).toString()

    /** Sets [key] to [value] when both are among what [groups] offers. Anything else is refused in a sentence. */
    suspend fun set(key: String, value: String) {
        val setting = groups().flatMap { it.second }.firstOrNull { it.key == key }
        if (setting == null || setting.choices.none { it.first == value }) throw PlatformException.Failed(text(R.string.phone_views_no_setting))
        when (key) {
            APPS_ONLY -> {
                asked = (value == "true") to clock()
                setAppsOnly(value == "true")
            }
            // Turned on again, the hint is owed in full, as it is from the TV.
            Prefs.KEY_HOLD_HINTS -> prefs.setHoldHints(value == "true")
            else -> prefs.restore(mapOf(key to value))
        }
    }

    private companion object {
        /** Kept by the core with the layout, and not with the settings a backup names. */
        const val APPS_ONLY = "apps_only"

        /** How long a change asked of the core may take to show in what it reports. */
        const val CORE_CATCHES_UP_MS = 5_000L
    }
}

/**
 * The TV's notifications for a paired phone, as the TV's own panel has them, and dismissing them.
 * While the TV may not read notifications, or has them turned off, the phone is shown none.
 */
class PhoneNotifications(
    private val context: Context,
    private val prefs: Prefs,
    private val source: NotificationSource,
    private val allowed: () -> Boolean = { hasNotificationAccess(context) },
) {
    private suspend fun shown(): List<AppNotification> = if (allowed()) shownNotifications(source.posted.value, prefs.notifications.first()).list else emptyList()

    /** Whether the TV reads notifications at all, and the ones it shows, newest first. */
    suspend fun json(): String = JSONObject()
        .put("on", allowed() && prefs.notifications.first().shown)
        .put(
            "list",
            JSONArray(
                shown().map { n ->
                    JSONObject().put("key", n.key).put("app", n.app).put("title", n.title).put("text", n.text).put("postedAt", n.postedAt).put("dismissible", n.dismissible)
                },
            ),
        ).toString()

    /** Dismisses the one with [key], or with none every one that can be. One the TV does not show is left alone. */
    suspend fun dismiss(key: String?) {
        val keys = shown().filter { it.dismissible && (key == null || it.key == key) }.map { it.key }
        if (key != null && keys.isEmpty()) throw PlatformException.Failed(context.getString(R.string.phone_views_no_notification))
        if (keys.isNotEmpty()) source.dismiss(keys)
    }
}
