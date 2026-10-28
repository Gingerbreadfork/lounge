package tv.lounge.platform

import android.content.ActivityNotFoundException
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.media.tv.TvContract
import android.net.Uri
import android.provider.Settings
import android.util.Log
import java.net.URISyntaxException
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import tv.lounge.core.Platform
import tv.lounge.core.PlatformException

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
}

/**
 * The core's view of Android: launch, view, switch, tune, control. Every failure becomes a
 * [PlatformException] so the core can report it.
 */
class AndroidPlatform(private val context: Context) : Platform {
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
            throw PlatformException.Failed("That link is not one Lounge can open")
        }
        val refused = PlatformException.Failed("Lounge won't open that link")
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
            command.startsWith("search:") -> _commands.tryEmit(UiCommand.Search(command.removePrefix("search:")))
            else -> throw PlatformException.Failed("Unknown control: $command")
        }
    }

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
                    else -> throw PlatformException.Failed("Unknown key: $key")
                }
                if (!_commands.tryEmit(UiCommand.Key(code))) throw PlatformException.Failed("busy")
            }
        }
    }

    override fun typeText(text: String) {
        if (!_commands.tryEmit(UiCommand.TypeText(text))) throw PlatformException.Failed("busy")
    }

    override fun mediaCommand(`package`: String, command: String) {
        val sender = mediaSender ?: throw PlatformException.Failed("Now playing needs notification access")
        if (!sender(`package`, command)) throw PlatformException.Failed("That app is not playing anything")
    }

    override fun requestPermission(name: String) {
        if (!_commands.tryEmit(UiCommand.RequestPermission(name))) throw PlatformException.Failed("busy")
    }

    override fun playStream(url: String, title: String, startMs: Long?) {
        val intent = Intent(Intent.ACTION_VIEW).apply {
            setDataAndType(Uri.parse(url), "video/*")
            putExtra(Intent.EXTRA_TITLE, title)
            putExtra("title", title)
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
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
        } catch (e: ActivityNotFoundException) {
            throw PlatformException.Failed("No video player can open this stream. Install VLC or another player.")
        } catch (e: SecurityException) {
            throw PlatformException.Failed(e.message ?: "not allowed")
        }
    }

    override fun openAppInfo(`package`: String) {
        val intent = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:$`package`"))
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        start(intent, null)
    }

    private fun start(intent: Intent, fallback: (() -> Unit)?) {
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
