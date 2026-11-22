package tv.lounge.feature.home

import android.content.res.Resources
import androidx.annotation.StringRes
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import kotlinx.coroutines.launch
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.core.graphics.drawable.toBitmap
import androidx.tv.material3.Icon
import androidx.tv.material3.Text
import coil3.compose.AsyncImage
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import tv.lounge.design.Hairline
import tv.lounge.design.LocalSwapSlotActive
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme
import tv.lounge.design.R as DesignR
import tv.lounge.design.Words
import tv.lounge.design.sounds
import tv.lounge.platform.AppNotification
import tv.lounge.platform.Prefs

/** The notifications the home shows, newest first, and how many arrived since the panel was last looked at. */
@Immutable
data class NotificationsShown(val list: List<AppNotification> = emptyList(), val fresh: Int = 0)

/** What is [posted], less the apps the person hid; nothing at all while notifications are turned off. */
internal fun shownNotifications(posted: List<AppNotification>, prefs: Prefs.NotificationPrefs): NotificationsShown {
    if (!prefs.shown) return NotificationsShown()
    val list = posted.filter { it.packageName !in prefs.hiddenApps }
    return NotificationsShown(list, list.count { it.postedAt > prefs.seenAt })
}

/** The words the notifications panel and its chip use (DESIGN.md §10). */
internal object NotificationWords {
    private val runs = Regex("\\s+")

    /** The chip's label: "1 notification", "3 notifications". */
    fun count(resources: Resources, n: Int): String = resources.getQuantityString(R.plurals.notifications_count, n, n)

    /** The chip as a screen reader says it, with how many are new. */
    fun spoken(resources: Resources, n: Int, fresh: Int): String {
        if (fresh <= 0) return count(resources, n)
        return resources.getQuantityString(R.plurals.notifications_count_spoken, n, n, resources.getQuantityString(R.plurals.notifications_new, fresh, fresh))
    }

    /** A notification's text run together, for the two lines the list shows of it. */
    fun preview(text: String): String = text.trim().replace(runs, " ")

    /** Who says it and when: "Play Store, 5 minutes ago". The app is left out where the title is already its name. */
    fun byline(resources: Resources, n: AppNotification, now: Long): String {
        val age = now - n.postedAt
        return if (n.title == n.app) agoAlone(resources, age) else resources.getString(R.string.notifications_byline, n.app, Words.ago(resources, age))
    }

    /** How long ago something [ms] old came, worded to start a line: "Just now", "5 minutes ago". */
    private fun agoAlone(resources: Resources, ms: Long): String {
        val minutes = (ms / 60_000).coerceAtLeast(0)
        fun counted(plural: Int, n: Long) = resources.getQuantityString(plural, n.toInt(), n.toInt())
        return when {
            minutes < 1 -> resources.getString(R.string.notifications_when_just_now)
            minutes < 60 -> counted(R.plurals.notifications_when_minutes_ago, minutes)
            minutes < 24 * 60 -> counted(R.plurals.notifications_when_hours_ago, minutes / 60)
            else -> counted(R.plurals.notifications_when_days_ago, minutes / (24 * 60))
        }
    }

    /** Words that are capitals by right, and stay so in a label that is otherwise lowered. */
    private val initials = setOf("OK", "TV", "USB", "HDMI", "VPN", "PIN", "DVR", "HD", "UHD", "HDR", "APK", "SD", "PC", "ID", "URL", "DNS", "IP")

    /** A button's label as its app wrote it, in sentence case where the app wrote capitals alone. */
    fun button(label: String): String {
        val letters = label.filter { it.isLetter() }
        if (letters.length < 2 || letters.any { !it.isUpperCase() }) return label
        var first = true
        return label.split(' ').joinToString(" ") { word ->
            val leads = first && word.isNotEmpty()
            if (word.isNotEmpty()) first = false
            when {
                word.trim { !it.isLetterOrDigit() } in initials -> word
                leads -> word.lowercase().replaceFirstChar { it.uppercase() }
                else -> word.lowercase()
            }
        }
    }

    /** What Dismiss all says of those that cannot be dismissed. */
    fun staying(resources: Resources, n: Int): String? = if (n == 0) null else resources.getQuantityString(R.plurals.notifications_staying, n, n)

    @StringRes val TAKEN_BACK = R.string.notifications_taken_back

    /** Said once on a TV whose notification access was given before it covered notifications. */
    @StringRes val OFFERED = R.string.notifications_offered

    /** Why a notification has no Dismiss. */
    @StringRes val STAYS = R.string.notifications_stays
}

/** The time now, read again every half minute while on screen, so "3 minutes ago" keeps up. */
@Composable
private fun rememberNow(): Long {
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    WhileStarted {
        while (true) {
            now = System.currentTimeMillis()
            delay(30_000)
        }
    }
    return now
}

/**
 * An app's icon at sheet-row size: the home's own copy where it has one, else the icon Android
 * holds for the package, and the bell for an app that has none.
 */
@Composable
internal fun AppIcon(packageName: String, iconPath: String?, size: Dp = 28.dp) {
    val shape = RoundedCornerShape(size / 4)
    if (iconPath != null) {
        AsyncImage(model = iconPath, contentDescription = null, contentScale = ContentScale.Fit, modifier = Modifier.size(size).clip(shape))
        return
    }
    val context = LocalContext.current
    // Unset while it is being read, so the bell does not flash before an icon that is on its way.
    val icon by produceState<Result<ImageBitmap>?>(null, packageName) {
        value = withContext(Dispatchers.IO) { runCatching { context.packageManager.getApplicationIcon(packageName).toBitmap(96, 96).asImageBitmap() } }
    }
    Box(Modifier.size(size)) {
        val read = icon ?: return@Box
        val bitmap = read.getOrNull()
        if (bitmap != null) {
            Image(bitmap, contentDescription = null, contentScale = ContentScale.Fit, modifier = Modifier.size(size).clip(shape))
        } else {
            Icon(LoungeIcons.Bell, contentDescription = null, tint = LoungeTheme.colors.dust, modifier = Modifier.size(size))
        }
    }
}

/**
 * The notifications panel (DESIGN.md §5.4): what apps on the TV have posted, newest first. A row
 * is the title, up to two lines of the text, and who says it and when; OK opens it in full.
 * [returnTo] is the key of the one Back has just left and [lastIndex] its place, so focus lands
 * on it again, or on what took its place.
 */
@Composable
fun NotificationsSheet(
    shown: NotificationsShown,
    iconPath: (String) -> String?,
    returnTo: String?,
    lastIndex: Int,
    onRead: (Int, AppNotification) -> Unit,
    onDismissAll: () -> Unit,
    firstFocus: FocusRequester,
) {
    val colors = LoungeTheme.colors
    val sounds = LoungeTheme.sounds
    val resources = LocalResources.current
    val now = rememberNow()
    val list = shown.list
    val focusAt = when {
        returnTo == null -> 0
        else -> list.indexOfFirst { it.key == returnTo }.takeIf { it >= 0 } ?: lastIndex.coerceIn(0, (list.size - 1).coerceAtLeast(0))
    }
    val scope = rememberCoroutineScope()
    val rowFocus = remember { HashMap<String, FocusRequester>() }
    // The focused row and its place, so one that its app takes back hands focus to its neighbour.
    var focused by remember { mutableStateOf<Pair<String, Int>?>(null) }
    val keys = list.map { it.key }
    LaunchedEffect(keys) {
        rowFocus.keys.retainAll(keys.toSet())
        val (key, at) = focused ?: return@LaunchedEffect
        if (key !in keys && keys.isNotEmpty()) rowFocus[keys[at.coerceIn(0, keys.lastIndex)]]?.requestWhenReady()
    }
    SheetHeading(stringResource(DesignR.string.common_notifications))
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        list.forEachIndexed { i, n ->
            key(n.key) {
                val preview = NotificationWords.preview(n.text)
                // Cut at a word, since the whole text is a press away.
                val text = rememberFittedLines(preview, LoungeTheme.typography.meta, SheetRowWordsWidth, 2)
                SheetRow(
                    icon = null,
                    // Level with the title, as the one who says it.
                    leading = { Box(Modifier.align(Alignment.Top).padding(top = 15.dp)) { AppIcon(n.packageName, iconPath(n.packageName)) } },
                    title = n.title,
                    lines = {
                        if (text.isNotEmpty()) Text(text, style = LoungeTheme.typography.meta, color = colors.dust, maxLines = 2, overflow = TextOverflow.Ellipsis)
                        Text(NotificationWords.byline(resources, n, now), style = LoungeTheme.typography.caption, color = colors.dust, modifier = Modifier.padding(top = 4.dp))
                    },
                    onClick = { onRead(i, n) },
                    focusRequester = firstFocus.takeIf { i == focusAt },
                    modifier = Modifier.focusRequester(rowFocus.getOrPut(n.key) { FocusRequester() }).onFocusChanged { if (it.isFocused) focused = n.key to i },
                )
            }
        }
        val dismissible = list.count { it.dismissible }
        if (dismissible >= 2) {
            Spacer(Modifier.height(16.dp))
            Hairline(Modifier.fillMaxWidth())
            Spacer(Modifier.height(8.dp))
            SheetRow(
                icon = SettingsGlyphs.Dismiss,
                title = stringResource(R.string.notifications_dismiss_all),
                subtitle = NotificationWords.staying(resources, list.size - dismissible),
                onClick = {
                    sounds?.tick()
                    onDismissAll()
                    // This row leaves with them; what stays takes focus.
                    focused = null
                    scope.launch {
                        withFrameNanos { }
                        firstFocus.requestWhenReady()
                    }
                },
            )
        }
    }
}

/**
 * One notification in full: its title, who says it and when, the whole text, then a row for each
 * thing to do with it. Focus lands on the first of them, so OK twice from the list opens it.
 */
@Composable
fun NotificationDetailSheet(
    notification: AppNotification?,
    onOpen: (AppNotification) -> Unit,
    onPress: (AppNotification, AppNotification.Action) -> Unit,
    onDismiss: (AppNotification) -> Unit,
    onHideApp: (AppNotification) -> Unit,
    firstFocus: FocusRequester,
) {
    // One that has just gone leaves the page empty for the moment it takes the panel to move on.
    val n = notification ?: return
    val colors = LoungeTheme.colors
    val sounds = LoungeTheme.sounds
    val now = rememberNow()
    var sent by remember(n.key) { mutableStateOf<Int?>(null) }
    LaunchedEffect(sent) {
        if (sent != null) {
            delay(2_500)
            sent = null
        }
    }
    val live = LocalSwapSlotActive.current
    LaunchedEffect(n.key) { if (live.value) firstFocus.requestWhenReady() }
    val firstAction = n.actions.firstOrNull()?.takeIf { !n.opens }

    SheetHeading(n.title, NotificationWords.byline(LocalResources.current, n, now))
    if (n.text.isNotEmpty()) {
        Text(n.text, style = LoungeTheme.typography.body, color = colors.limelight, maxLines = 14, overflow = TextOverflow.Ellipsis)
        Spacer(Modifier.height(28.dp))
    }
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        if (n.opens) {
            SheetRow(icon = SettingsGlyphs.Open, title = stringResource(DesignR.string.common_open), onClick = { sounds?.open(); onOpen(n) }, focusRequester = firstFocus)
        }
        n.actions.forEach { action ->
            SheetRow(
                icon = SettingsGlyphs.Act,
                title = NotificationWords.button(action.label),
                value = if (sent == action.index) stringResource(R.string.notifications_sent) else null,
                onClick = {
                    sounds?.tick()
                    sent = action.index
                    onPress(n, action)
                },
                focusRequester = firstFocus.takeIf { action == firstAction },
            )
        }
        if (n.dismissible) {
            SheetRow(
                icon = SettingsGlyphs.Dismiss,
                title = stringResource(R.string.notifications_dismiss),
                onClick = { sounds?.tick(); onDismiss(n) },
                focusRequester = firstFocus.takeIf { !n.opens && n.actions.isEmpty() },
            )
        }
        Spacer(Modifier.height(16.dp))
        Hairline(Modifier.fillMaxWidth())
        Spacer(Modifier.height(8.dp))
        SheetRow(
            icon = LoungeIcons.Hide,
            title = stringResource(R.string.notifications_hide_app, n.app),
            subtitle = stringResource(R.string.notifications_hide_app_subtitle),
            onClick = { sounds?.tick(); onHideApp(n) },
            focusRequester = firstFocus.takeIf { !n.opens && n.actions.isEmpty() && !n.dismissible },
        )
    }
    if (!n.dismissible) SheetFootnote(stringResource(NotificationWords.STAYS))
}

/** An app whose notifications are left out, as Settings lists it. */
@Immutable
data class HiddenApp(val packageName: String, val name: String)

/**
 * Settings, Notifications: the switch, then the apps left out with a Show row each. Without
 * notification access the switch is the one step to take and OK asks for it.
 */
@Composable
fun NotificationSettingsSheet(
    on: Boolean,
    allowed: Boolean,
    hidden: List<HiddenApp>,
    iconPath: (String) -> String?,
    onSet: (Boolean) -> Unit,
    onAllow: () -> Unit,
    onShow: (HiddenApp) -> Unit,
    firstFocus: FocusRequester,
) {
    val colors = LoungeTheme.colors
    val sounds = LoungeTheme.sounds
    val scope = rememberCoroutineScope()
    val rowFocus = remember { HashMap<String, FocusRequester>() }
    SheetHeading(stringResource(DesignR.string.common_notifications), stringResource(R.string.notifications_settings_subtitle))
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        // One row for both states, so it keeps focus when the access is given while it has it.
        val flip = { sounds?.tick(); onSet(!on) }
        SheetRow(
            icon = LoungeIcons.Bell,
            title = stringResource(DesignR.string.common_notifications),
            subtitle = if (allowed) null else stringResource(R.string.notifications_needs_access),
            subtitleLines = WHOLE,
            value = stringResource(if (!allowed) R.string.notifications_not_allowed else if (on) DesignR.string.common_on else DesignR.string.common_off),
            accent = if (allowed && on) colors.signal else colors.dust,
            onClick = if (allowed) flip else onAllow,
            onCycle = if (allowed) { _ -> flip() } else null,
            focusRequester = firstFocus,
        )
        if (hidden.isNotEmpty()) {
            Spacer(Modifier.height(16.dp))
            Hairline(Modifier.fillMaxWidth())
            SheetGroup(stringResource(R.string.notifications_hidden_apps))
            hidden.forEachIndexed { i, app ->
                key(app.packageName) {
                    SheetRow(
                        icon = null,
                        leading = { AppIcon(app.packageName, iconPath(app.packageName)) },
                        title = app.name,
                        value = stringResource(DesignR.string.common_show),
                        focusRequester = rowFocus.getOrPut(app.packageName) { FocusRequester() },
                        onClick = {
                            sounds?.tick()
                            // The row goes at once, so focus moves to its neighbour, or back up to the switch.
                            val next = hidden.getOrNull(i + 1) ?: hidden.getOrNull(i - 1)
                            onShow(app)
                            scope.launch {
                                withFrameNanos { }
                                (next?.let { rowFocus[it.packageName] } ?: firstFocus).requestWhenReady()
                            }
                        },
                    )
                }
            }
        }
    }
    SheetFootnote(
        stringResource(R.string.notifications_footnote_privacy),
        stringResource(R.string.notifications_footnote_left_out),
    )
}
