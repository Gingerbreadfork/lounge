package tv.lounge.platform

import android.content.ActivityNotFoundException
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.media.tv.TvContract
import android.net.Uri
import android.os.Bundle
import android.provider.Browser
import android.provider.Settings
import android.util.Log
import java.net.URISyntaxException
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.runBlocking
import tv.lounge.core.Platform
import tv.lounge.core.PlatformException
import tv.lounge.core.TvControls

/** A pop-up the home asked for, such as a ring at the door, with a picture to show by its address. */
data class HomeAlert(val title: String, val message: String?, val image: String?, val seconds: Int)

/** Commands the core asks the shell to show in its own UI rather than hand to Android. */
sealed interface UiCommand {
    data object OpenTvControls : UiCommand
    data class Toast(val message: String) : UiCommand
    /** Ask for an optional access: tv_listings, notifications, usage. */
    data class RequestPermission(val name: String) : UiCommand
    /** A remote key from the companion, as an Android key code. */
    data class Key(val keyCode: Int) : UiCommand
    /** Text from the companion for the focused field. */
    data class TypeText(val text: String) : UiCommand
    /** Open search with this query. */
    data class Search(val query: String) : UiCommand
    data object GoHome : UiCommand
    /** The update notice was pressed: install, confirm or ask for permission as the state needs. */
    data object InstallUpdate : UiCommand
    /** The All apps tile at the end of Your apps was pressed. */
    data object OpenAllApps : UiCommand
    /** Open a plugin's page, to turn it on and set it up. */
    data class OpenPlugin(val id: String) : UiCommand
    /** Open the sheet a plugin describes under [id]. */
    data class OpenPluginSheet(val plugin: String, val id: String) : UiCommand
    /** Show a plugin's still picture full screen and keep fetching it. */
    data class OpenLive(val plugin: String, val title: String, val url: String) : UiCommand
}

/**
 * The core's view of Android: launch, view, switch, tune, control. Every failure becomes a
 * [PlatformException] so the core can report it.
 */
class AndroidPlatform(private val context: Context) : Platform {
    private val prefs by lazy { Prefs(context) }
    private val _commands = MutableSharedFlow<UiCommand>(extraBufferCapacity = 8)
    val commands: SharedFlow<UiCommand> = _commands

    /** Set by the shell once the media collector exists; sends transport commands to a session. */
    @Volatile var mediaSender: ((String, String) -> Boolean)? = null

    override fun launchApp(`package`: String, activity: String) {
        if (activity.isEmpty()) {
            launchByPackage(`package`)
            return
        }
        val intent = Intent(Intent.ACTION_MAIN).apply {
            component = ComponentName(`package`, activity)
            addCategory(Intent.CATEGORY_LEANBACK_LAUNCHER)
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_RESET_TASK_IF_NEEDED)
        }
        start(intent) { launchByPackage(`package`) }
    }

    private fun launchByPackage(`package`: String) {
        val intent = context.packageManager.getLeanbackLaunchIntentForPackage(`package`)
            ?: context.packageManager.getLaunchIntentForPackage(`package`)
            ?: throw PlatformException.Failed("No launch intent for $`package`")
        intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        start(intent, null)
    }

    override fun viewUri(uri: String) {
        start(intentFor(uri), null)
    }

    /**
     * An `intent:` or `android-app:` string (what TvProvider rows carry) is an Intent serialised
     * by `toUri`; anything else is a plain link to view. They come from other apps and plugins, so
     * only a view, a search by name, or a launch of a named app gets through: no selector, clip
     * data or URI grants, no installs, and only into another app's exported activity.
     */
    private fun intentFor(uri: String): Intent {
        val intent = try {
            when {
                uri.startsWith("intent:") -> Intent.parseUri(uri, Intent.URI_INTENT_SCHEME)
                uri.startsWith("android-app:") -> Intent.parseUri(uri, Intent.URI_ANDROID_APP_SCHEME)
                else -> Intent(Intent.ACTION_VIEW, Uri.parse(uri))
            }
        } catch (e: URISyntaxException) {
            throw PlatformException.Failed(context.getString(R.string.android_platform_link_unknown))
        }
        val refused = PlatformException.Failed(context.getString(R.string.android_platform_link_refused))
        val explicit = intent.component != null || intent.`package` != null
        when (intent.action) {
            null, Intent.ACTION_VIEW -> Unit
            Intent.ACTION_MAIN -> if (!explicit) throw refused
            // The TV's own search for a title: a query and nothing else.
            Intent.ACTION_SEARCH, android.app.SearchManager.INTENT_ACTION_GLOBAL_SEARCH, android.provider.MediaStore.INTENT_ACTION_MEDIA_PLAY_FROM_SEARCH -> {
                if (explicit || intent.data != null) throw refused
                val query = intent.getStringExtra(android.app.SearchManager.QUERY) ?: throw refused
                intent.replaceExtras(android.os.Bundle().apply { putString(android.app.SearchManager.QUERY, query) })
            }
            else -> throw refused
        }
        intent.selector = null
        intent.clipData = null
        intent.flags = intent.flags and URI_GRANTS.inv()
        if (intent.type == APK_TYPE || intent.data?.scheme?.lowercase() in setOf("package", "file")) throw refused
        // A content link with no type can still be an APK by its provider's say, so the type the
        // provider gives and the activity the link would open are checked as well.
        if (intent.data?.scheme?.lowercase() == "content" && runCatching { intent.resolveType(context) }.getOrNull() == APK_TYPE) throw refused
        val target = runCatching { context.packageManager.resolveActivity(intent, 0)?.activityInfo?.packageName }.getOrNull()
        if (target != null && target == installerPackage()) throw refused
        val own = context.packageName
        if (intent.`package` == own) throw refused
        intent.component?.let { component ->
            if (component.packageName == own) throw refused
            val info = try {
                context.packageManager.getActivityInfo(component, 0)
            } catch (e: PackageManager.NameNotFoundException) {
                throw PlatformException.Failed("No Activity found for ${component.packageName}")
            }
            if (!info.exported || info.permission != null) throw refused
        }
        return intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    }

    /** The app that installs packages on this device, which no link may open. */
    private fun installerPackage(): String? = runCatching {
        val probe = Intent(Intent.ACTION_VIEW).setDataAndType(Uri.parse("content://tv.lounge.probe/update.apk"), APK_TYPE)
        context.packageManager.resolveActivity(probe, 0)?.activityInfo?.packageName?.takeIf { it != "android" }
    }.getOrNull()

    /** Home was pressed while Lounge was already in front. */
    fun goHome() {
        _commands.tryEmit(UiCommand.GoHome)
    }

    override fun switchInput(inputId: String) {
        val info = runCatching { context.getSystemService(android.media.tv.TvInputManager::class.java)?.getTvInputInfo(inputId) }.getOrNull()
        val uri = if (info != null && !info.isPassthroughInput) {
            TvContract.buildChannelsUriForInput(inputId)
        } else {
            TvContract.buildChannelUriForPassthroughInput(inputId)
        }
        val intent = Intent(Intent.ACTION_VIEW, uri).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        start(intent, null)
    }

    override fun tune(channelUri: String) {
        start(intentFor(channelUri), null)
    }

    override fun control(command: String) {
        when {
            command == "tv:open" -> _commands.tryEmit(UiCommand.OpenTvControls)
            command == "update" -> _commands.tryEmit(UiCommand.InstallUpdate)
            command == "allapps" -> _commands.tryEmit(UiCommand.OpenAllApps)
            command.startsWith("plugin:") -> _commands.tryEmit(UiCommand.OpenPlugin(command.removePrefix("plugin:")))
            command.startsWith("search:") -> _commands.tryEmit(UiCommand.Search(command.removePrefix("search:")))
            command.startsWith("alert:") -> {
                val said = runCatching { org.json.JSONObject(command.removePrefix("alert:")) }.getOrNull() ?: throw PlatformException.Failed("Unknown control: alert")
                fun text(name: String) = said.optString(name).takeIf { it.isNotBlank() && !said.isNull(name) }
                showAlert(HomeAlert(said.optString("title"), text("message"), text("image"), said.optInt("seconds", 20)))
            }
            command.startsWith("sheet:") || command.startsWith("live:") -> {
                val screen = pluginScreen(command) ?: throw PlatformException.Failed("Unknown control: $command")
                // A press that would open one of Lounge's own screens behind another app is told so.
                if (!inFront()) throw PlatformException.Failed(context.getString(R.string.android_platform_not_in_front))
                _commands.tryEmit(screen)
            }
            else -> throw PlatformException.Failed("Unknown control: $command")
        }
    }

    /** Set by the shell: shows a pop-up, or throws a sentence saying why it cannot. */
    @Volatile var showAlert: (HomeAlert) -> Unit = { throw PlatformException.Failed("Unknown control: alert") }

    /** Set by the shell: whether Lounge's own window is in front. */
    @Volatile var inFront: () -> Boolean = { true }

    /** Set by the shell: whether Lounge's window is on the screen, in front or under a dialog. */
    @Volatile var onScreen: () -> Boolean = { true }

    /**
     * Whether Android will let Lounge bring another app forward now. From behind another app it
     * does so only for the home screen and for an app allowed to draw over others. Any other
     * start it drops without a word.
     */
    private fun mayOpen(): Boolean =
        android.os.Build.VERSION.SDK_INT < 29 || onScreen() || isHomeScreen() || Settings.canDrawOverlays(context)

    /** What the phone is told when Android would drop a start made from behind another app. */
    private fun openedFromBehind() = PlatformException.Failed(context.getString(R.string.android_platform_opened_from_behind))

    private fun isHomeScreen(): Boolean = runCatching {
        val home = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_HOME)
        context.packageManager.resolveActivity(home, PackageManager.MATCH_DEFAULT_ONLY)?.activityInfo?.packageName == context.packageName
    }.getOrDefault(true)

    /** Set by the home screen: whether one of Lounge's text fields is open to take text. */
    @Volatile var fieldOpen: () -> Boolean = { false }

    /** Set by the shell: whether Power Setup can press keys for another app in front. */
    @Volatile var keysForOtherApps: () -> Boolean = { false }

    /** Set by the shell: the TV's power and input controls, as the phone lists them. */
    @Volatile var listTvControls: () -> TvControls = { TvControls(note = null, controls = emptyList()) }

    /** Set by the shell: runs one of those rows by its id, or throws with a sentence saying why not. */
    @Volatile var pressTvControl: (String) -> Unit = { throw PlatformException.Failed(context.getString(R.string.android_platform_still_starting)) }

    override fun tvControls(): TvControls = listTvControls()

    override fun runTvControl(id: String) = pressTvControl(id)

    /**
     * A key from the phone. Home, volume and media keys always arrive; the d-pad, Back and Menu
     * reach Lounge while it is in front and another app only through Power Setup, and a key that
     * can reach nothing is refused with the reason.
     */
    override fun sendKey(key: String) {
        val audio = context.getSystemService(Context.AUDIO_SERVICE) as android.media.AudioManager
        when (key) {
            "volume_up" -> audio.adjustStreamVolume(android.media.AudioManager.STREAM_MUSIC, android.media.AudioManager.ADJUST_RAISE, android.media.AudioManager.FLAG_SHOW_UI)
            "volume_down" -> audio.adjustStreamVolume(android.media.AudioManager.STREAM_MUSIC, android.media.AudioManager.ADJUST_LOWER, android.media.AudioManager.FLAG_SHOW_UI)
            "mute" -> audio.adjustStreamVolume(android.media.AudioManager.STREAM_MUSIC, android.media.AudioManager.ADJUST_TOGGLE_MUTE, android.media.AudioManager.FLAG_SHOW_UI)
            "home" -> {
                _commands.tryEmit(UiCommand.GoHome)
                // With another app in front, Home brings Lounge up; the home screen may start from
                // the background.
                runCatching {
                    context.startActivity(Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_HOME).setPackage(context.packageName).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                }
            }
            "play_pause", "rewind", "fast_forward", "play", "pause", "next", "previous", "stop" -> {
                // Media keys go to whichever app is playing, not to our window.
                val code = when (key) {
                    "play_pause" -> android.view.KeyEvent.KEYCODE_MEDIA_PLAY_PAUSE
                    "rewind" -> android.view.KeyEvent.KEYCODE_MEDIA_REWIND
                    "fast_forward" -> android.view.KeyEvent.KEYCODE_MEDIA_FAST_FORWARD
                    "play" -> android.view.KeyEvent.KEYCODE_MEDIA_PLAY
                    "pause" -> android.view.KeyEvent.KEYCODE_MEDIA_PAUSE
                    "next" -> android.view.KeyEvent.KEYCODE_MEDIA_NEXT
                    "previous" -> android.view.KeyEvent.KEYCODE_MEDIA_PREVIOUS
                    else -> android.view.KeyEvent.KEYCODE_MEDIA_STOP
                }
                audio.dispatchMediaKeyEvent(android.view.KeyEvent(android.view.KeyEvent.ACTION_DOWN, code))
                audio.dispatchMediaKeyEvent(android.view.KeyEvent(android.view.KeyEvent.ACTION_UP, code))
            }
            else -> {
                val code = when (key) {
                    "up" -> android.view.KeyEvent.KEYCODE_DPAD_UP
                    "down" -> android.view.KeyEvent.KEYCODE_DPAD_DOWN
                    "left" -> android.view.KeyEvent.KEYCODE_DPAD_LEFT
                    "right" -> android.view.KeyEvent.KEYCODE_DPAD_RIGHT
                    "center", "ok", "select" -> android.view.KeyEvent.KEYCODE_DPAD_CENTER
                    "back" -> android.view.KeyEvent.KEYCODE_BACK
                    "menu" -> android.view.KeyEvent.KEYCODE_MENU
                    else -> throw PlatformException.Failed(context.getString(R.string.android_platform_unknown_key))
                }
                if (!inFront() && !keysForOtherApps()) {
                    throw PlatformException.Failed(context.getString(R.string.android_platform_keys_need_power_setup))
                }
                if (!_commands.tryEmit(UiCommand.Key(code))) throw PlatformException.Failed(context.getString(R.string.android_platform_busy))
            }
        }
    }

    /** Text from the phone lands only in a field open in Lounge, so it needs Lounge in front with a field open. */
    override fun typeText(text: String) {
        if (!inFront()) throw PlatformException.Failed(context.getString(R.string.android_platform_typing_needs_lounge))
        if (!fieldOpen()) throw PlatformException.Failed(context.getString(R.string.android_platform_no_field))
        if (!_commands.tryEmit(UiCommand.TypeText(text))) throw PlatformException.Failed(context.getString(R.string.android_platform_busy))
    }

    override fun mediaCommand(`package`: String, command: String) {
        val sender = mediaSender ?: throw PlatformException.Failed(context.getString(R.string.android_platform_needs_notification_access))
        if (!sender(`package`, command)) throw PlatformException.Failed(context.getString(R.string.android_platform_not_playing))
    }

    override fun requestPermission(name: String) {
        if (!_commands.tryEmit(UiCommand.RequestPermission(name))) throw PlatformException.Failed("busy")
    }

    override fun playStream(url: String, title: String, startMs: Long?, headers: Map<String, String>, audio: Boolean) {
        if (!mayOpen()) throw openedFromBehind()
        // Sound is offered to audio players first, and to a video player on a TV that has none.
        for (type in if (audio) listOf("audio/*", "video/*") else listOf("video/*")) {
            val intent = Intent(Intent.ACTION_VIEW).apply {
                setDataAndType(Uri.parse(url), type)
                putExtra(Intent.EXTRA_TITLE, title)
                putExtra("title", title)
                addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            }
            if (headers.isNotEmpty()) {
                // Players take request headers one of two ways: names and values in turn, as MX
                // Player defined it, or Android's own bundle for a view. Whether one sends them is its choice.
                intent.putExtra("headers", headers.flatMap { listOf(it.key, it.value) }.toTypedArray())
                intent.putExtra(Browser.EXTRA_HEADERS, Bundle().apply { headers.forEach { (name, value) -> putString(name, value) } })
            }
            if (startMs != null && startMs > 0) {
                // Players take the start as "position": MX Player and Just Player read an int, VLC a long.
                val player = runCatching { context.packageManager.resolveActivity(intent, 0)?.activityInfo?.packageName }.getOrNull().orEmpty()
                if (player.startsWith("com.mxtech.videoplayer") || player == "com.brouken.player") {
                    intent.putExtra("position", startMs.coerceAtMost(Int.MAX_VALUE.toLong()).toInt())
                } else {
                    intent.putExtra("position", startMs)
                }
            }
            try {
                context.startActivity(intent)
                return
            } catch (e: ActivityNotFoundException) {
                continue
            } catch (e: SecurityException) {
                throw PlatformException.Failed(e.message ?: "not allowed")
            }
        }
        throw PlatformException.Failed(context.getString(if (audio) R.string.android_platform_no_player else R.string.android_platform_no_video_player))
    }

    /** Both are called off the main thread, by the phone's backup and restore. */
    override fun preferences(): Map<String, String> = runBlocking { prefs.exported() }

    override fun restorePreferences(preferences: Map<String, String>) {
        try {
            runBlocking { prefs.restore(preferences) }
        } catch (e: java.io.IOException) {
            throw PlatformException.Failed(context.getString(R.string.android_platform_settings_not_saved))
        }
    }

    override fun openAppInfo(`package`: String) {
        val intent = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:$`package`"))
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        start(intent, null)
    }

    private fun start(intent: Intent, fallback: (() -> Unit)?) {
        if (!mayOpen()) throw openedFromBehind()
        try {
            context.startActivity(intent)
        } catch (e: ActivityNotFoundException) {
            if (fallback != null) {
                try {
                    fallback()
                    return
                } catch (inner: Exception) {
                    Log.w(TAG, "fallback launch failed", inner)
                    throw PlatformException.Failed(inner.message ?: "launch failed")
                }
            }
            throw PlatformException.Failed(e.message ?: "nothing can open this")
        } catch (e: SecurityException) {
            throw PlatformException.Failed(e.message ?: "not allowed")
        }
    }

    private companion object {
        const val TAG = "LoungePlatform"
        const val APK_TYPE = "application/vnd.android.package-archive"
        const val URI_GRANTS = Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION or
            Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION or Intent.FLAG_GRANT_PREFIX_URI_PERMISSION
    }
}

/**
 * The screen a plugin's control opens: `sheet:<plugin>:<sheet id>`, or `live:<plugin>` with the
 * title and the picture's address on the lines after.
 */
internal fun pluginScreen(command: String): UiCommand? {
    if (command.startsWith("sheet:")) {
        val rest = command.removePrefix("sheet:")
        val plugin = rest.substringBefore(':')
        val id = rest.substringAfter(':', "")
        return if (plugin.isEmpty() || id.isEmpty()) null else UiCommand.OpenPluginSheet(plugin, id)
    }
    if (command.startsWith("live:")) {
        val lines = command.removePrefix("live:").split('\n')
        val plugin = lines.getOrNull(0).orEmpty()
        val url = lines.getOrNull(2).orEmpty()
        return if (plugin.isEmpty() || url.isEmpty()) null else UiCommand.OpenLive(plugin, lines.getOrNull(1).orEmpty(), url)
    }
    return null
}
