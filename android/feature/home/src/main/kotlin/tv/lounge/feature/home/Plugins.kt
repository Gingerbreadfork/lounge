package tv.lounge.feature.home

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.isImeVisible
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.foundation.focusable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.OffsetMapping
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.input.TransformedText
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.tv.material3.Text
import coil3.compose.AsyncImage
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import tv.lounge.core.CoreClient
import tv.lounge.core.PluginAsk
import tv.lounge.core.PluginInfo
import tv.lounge.core.PluginSettingInfo
import tv.lounge.design.Hairline
import tv.lounge.design.LocalSwapSlotActive
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme
import tv.lounge.design.Words
import tv.lounge.design.focusRim
import tv.lounge.design.lightGlow

/** Installed plugins by name, each with its state. Selecting one opens its detail sheet. */
@Composable
fun PluginsSheet(core: CoreClient, onOpen: (String) -> Unit, firstFocus: FocusRequester, cameFrom: Overlay? = null) {
    val colors = LoungeTheme.colors
    val snapshot by core.snapshots.collectAsStateWithLifecycle()
    var plugins by remember { mutableStateOf<List<PluginInfo>?>(null) }
    // Each read waits on the core's lock, which a big refresh can hold, so the reads run off the main thread.
    WhileStarted {
        while (true) {
            plugins = withContext(Dispatchers.IO) { core.plugins() }.sortedWith(compareBy(String.CASE_INSENSITIVE_ORDER) { it.name })
            delay(2000)
        }
    }
    // A plugin for an app on the TV wears that app's icon.
    val appIcons = remember(snapshot.allApps) { snapshot.allApps.mapNotNull { t -> t.iconPath?.let { t.id.removePrefix("app:") to it } }.toMap() }
    // The rows land after the sheet first asks for focus; a copy being swapped out leaves focus alone.
    val live = LocalSwapSlotActive.current
    LaunchedEffect(plugins != null) { if (plugins != null && live.value) firstFocus.requestWhenReady() }
    val list = plugins
    // Back from a plugin's page puts focus on that plugin, if it is still there.
    val returnTo = (cameFrom as? Overlay.PluginDetail)?.id?.takeIf { id -> list.orEmpty().any { it.id == id } }
    SheetHeading("Plugins", "They add rows, channels and guide data from your services.")
    if (list == null) {
        Text("Loading…", style = LoungeTheme.typography.meta, color = colors.dust)
    } else if (list.isEmpty()) {
        Text("No plugins installed. They install from the phone companion: on your phone, open Plugins.", style = LoungeTheme.typography.meta, color = colors.dust)
    } else {
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            list.forEachIndexed { i, p ->
                val appIcon = p.forApps.firstNotNullOfOrNull { appIcons[it] }
                SheetRow(
                    icon = if (appIcon == null) pluginGlyph(p) else null,
                    leading = appIcon?.let { path ->
                        { AsyncImage(model = path, contentDescription = null, contentScale = ContentScale.Fit, modifier = Modifier.size(28.dp).clip(RoundedCornerShape(7.dp))) }
                    },
                    title = p.name,
                    value = statusWord(p),
                    accent = statusColour(p),
                    onClick = { onOpen(p.id) },
                    focusRequester = if (p.id == returnTo || (returnTo == null && i == 0)) firstFocus else null,
                )
            }
        }
    }
    SheetFootnote("Each plugin can reach only the places it names, and sees only its own settings.", "More plugins install from the phone companion.")
}

/** A glyph for what a plugin brings, for one with no app of its own on the TV. */
private fun pluginGlyph(p: PluginInfo): ImageVector = when {
    "epg" in p.kinds -> LoungeIcons.Guide
    "content" in p.kinds -> LoungeIcons.Play
    "screensaver" in p.kinds || "wallpaper" in p.kinds -> LoungeIcons.Frame
    "search" in p.kinds -> LoungeIcons.Search
    else -> SettingsGlyphs.Plugin
}

private fun statusWord(p: PluginInfo): String = when {
    // A plugin waiting on the person is not a problem: a sign-in when there is a page or a code.
    p.enabled && p.ask != null -> if (p.ask?.link != null || p.ask?.code != null) "Sign in" else "Needs setup"
    p.lastError != null && p.enabled -> "Problem"
    p.enabled -> "On"
    p.updateAsksToReach.isNotEmpty() -> "Turned off by its update"
    !p.configured -> "Needs setup"
    else -> "Off"
}

@Composable
private fun statusColour(p: PluginInfo) = with(LoungeTheme.colors) {
    when {
        p.enabled && p.ask != null -> houselight
        p.enabled && p.lastError == null -> signal
        p.enabled -> ember
        p.updateAsksToReach.isNotEmpty() -> houselight
        else -> dust
    }
}

/**
 * One plugin: what it is, whether it runs, its settings as editable fields, its log. Built-in
 * plugins can be turned off but not removed. One that still needs setting up opens on its first
 * empty required field, and offers Turn on after the last one.
 */
@Composable
fun PluginDetailSheet(core: CoreClient, pluginId: String, onRemoved: () -> Unit, firstFocus: FocusRequester) {
    val colors = LoungeTheme.colors
    var info by remember { mutableStateOf<PluginInfo?>(null) }
    var loaded by remember { mutableStateOf(false) }
    var showLog by remember { mutableStateOf(false) }
    var log by remember { mutableStateOf<List<String>?>(null) }
    var refreshAsked by remember { mutableStateOf<Long?>(null) }
    val scope = rememberCoroutineScope()
    // A switch waits for any setting still being written, so it sees what was just typed.
    val writes = remember { Mutex() }
    val fieldFocus = remember(pluginId) { HashMap<String, FocusRequester>() }
    fun focusOf(key: String) = fieldFocus.getOrPut(key) { FocusRequester() }
    WhileStarted(pluginId) {
        while (true) {
            info = withContext(Dispatchers.IO) { core.plugins().firstOrNull { it.id == pluginId } }
            loaded = true
            if (showLog) log = withContext(Dispatchers.IO) { core.pluginLog(pluginId) }
            delay(1500)
        }
    }
    val live = LocalSwapSlotActive.current
    LaunchedEffect(loaded) { if (loaded && live.value) firstFocus.requestWhenReady() }
    if (!loaded) {
        Text("Loading…", style = LoungeTheme.typography.meta, color = colors.dust)
        return
    }
    val p = info ?: run {
        SheetHeading("Plugin", "This plugin is no longer installed.")
        SheetRow(icon = SettingsGlyphs.Plugin, title = "Back to Plugins", onClick = onRemoved, focusRequester = firstFocus)
        return
    }

    val held = !p.enabled && p.updateAsksToReach.isNotEmpty()
    val missing = p.settings.firstOrNull { it.required && !it.hasValue }
    val startAtField = !p.enabled && !held && !p.configured && missing != null
    /** Turns it on once every required setting is in; until then, takes focus to the one still empty. */
    fun switch(on: Boolean) {
        scope.launch {
            val now = writes.withLock {
                withContext(Dispatchers.IO) {
                    val fresh = core.plugins().firstOrNull { it.id == pluginId }
                    if (fresh != null && (!on || fresh.configured)) core.setPluginEnabled(fresh.id, on)
                    core.plugins().firstOrNull { it.id == pluginId }
                }
            }
            info = now
            if (on && now != null && !now.enabled) now.settings.firstOrNull { it.required && !it.hasValue }?.let { focusOf(it.key).requestWhenReady() }
        }
    }

    SheetHeading(p.name, "Version ${p.version}" + if (p.publisherName.isNotEmpty()) ", by ${p.publisherName}" else "")
    if (p.description.isNotEmpty()) {
        Text(p.description, style = LoungeTheme.typography.meta, color = colors.dust)
        Spacer(Modifier.height(20.dp))
    }
    val ask = p.ask?.takeIf { p.enabled }
    when {
        held -> Text("Turned off by its update, which also asks to reach ${joined(p.updateAsksToReach)}.", style = LoungeTheme.typography.meta, color = colors.houselight)
        ask != null -> AskCard(ask)
        else -> StatusLine(p)
    }
    Spacer(Modifier.height(20.dp))

    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        if (held) {
            SheetRow(
                icon = LoungeIcons.Power,
                title = "Allow and turn on",
                subtitle = "It can then reach ${joined(p.updateAsksToReach)} as well",
                subtitleLines = WHOLE,
                accent = colors.houselight,
                onClick = { switch(true) },
                focusRequester = firstFocus,
            )
        } else {
            SheetRow(
                icon = LoungeIcons.Power,
                title = p.name,
                value = if (p.enabled) "On" else "Off",
                accent = if (p.enabled) colors.signal else colors.dust,
                onClick = { switch(!p.enabled) },
                focusRequester = if (startAtField) null else firstFocus,
            )
        }
        if (p.settings.isNotEmpty()) {
            Spacer(Modifier.height(8.dp))
            Text("Settings", style = LoungeTheme.typography.meta, color = colors.dust)
            p.settings.forEach { s ->
                SettingField(
                    setting = s,
                    focusRequester = focusOf(s.key),
                    // The page opens on the first field still to fill in.
                    alsoFocus = firstFocus.takeIf { startAtField && s.key == missing?.key },
                    onCommit = { value -> scope.launch { writes.withLock { withContext(Dispatchers.IO) { core.setPluginSetting(p.id, s.key, value) } } } },
                    onToggle = { on -> scope.launch { writes.withLock { withContext(Dispatchers.IO) { core.setPluginSetting(p.id, s.key, if (on) "true" else "false") } } } },
                )
            }
            if (!p.enabled && !held) {
                SheetRow(
                    icon = LoungeIcons.Power,
                    title = "Turn on ${p.name}",
                    subtitle = missing?.let { "Fill in ${it.label} first" },
                    accent = colors.houselight,
                    onClick = { switch(true) },
                )
            }
            Spacer(Modifier.height(8.dp))
        }
        if (p.enabled) {
            val refreshing = refreshAsked?.let { asked -> (p.lastRefreshMs ?: 0L) < asked && System.currentTimeMillis() - asked < 120_000 } == true
            SheetRow(
                icon = LoungeIcons.Refresh,
                title = "Refresh now",
                value = if (refreshing) "Refreshing…" else null,
                accent = if (refreshing) colors.houselight else colors.dust,
                onClick = { refreshAsked = System.currentTimeMillis(); core.refreshPlugin(p.id) },
            )
        }
        SheetRow(
            icon = LoungeIcons.Info,
            title = "Log",
            value = if (showLog) "Hide" else "Show",
            onClick = {
                showLog = !showLog
                if (showLog) scope.launch { log = withContext(Dispatchers.IO) { core.pluginLog(pluginId) } }
            },
        )
        if (showLog) {
            Column(Modifier.padding(start = 20.dp, top = 4.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                val lines = log
                when {
                    lines == null -> Text("Loading…", style = LoungeTheme.typography.caption, color = colors.dust)
                    lines.isEmpty() -> Text("Nothing logged yet.", style = LoungeTheme.typography.caption, color = colors.dust)
                    else -> lines.takeLast(12).forEach { line ->
                        Text(line.substringAfter(' '), style = LoungeTheme.typography.caption, color = colors.dust, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    }
                }
            }
        }
        if (!p.builtIn) {
            Spacer(Modifier.height(8.dp))
            Hairline(Modifier.fillMaxWidth())
            Spacer(Modifier.height(8.dp))
            // Removing forgets the plugin's servers and passwords too, so it asks once more first;
            // moving away takes the question back. It deletes files, so it runs off the main thread.
            var confirming by remember(p.id) { mutableStateOf(false) }
            SheetRow(
                icon = LoungeIcons.Trash,
                title = if (confirming) "Yes, remove it and its settings" else "Remove",
                titleColor = if (confirming) colors.ember else colors.limelight,
                accent = colors.ember,
                modifier = Modifier.onFocusChanged { if (!it.isFocused) confirming = false },
                onClick = {
                    if (!confirming) {
                        confirming = true
                    } else {
                        scope.launch { withContext(Dispatchers.IO) { core.removePlugin(p.id) }; onRemoved() }
                    }
                },
            )
        }
    }
    SheetFootnote(
        "It can reach ${reachText(p)}.",
        "It refreshes every ${Words.counted(p.refreshMinutes.toLong(), "minute", "minutes")} while the TV is on.",
        "Updates are taken only from its publisher, whose key starts ${p.publisher.take(8).chunked(4).joinToString(" ")}.",
    )
}

/** Items in a sentence: "a", "a and b", "a, b and c". */
private fun joined(parts: List<String>): String = when (parts.size) {
    0 -> "nothing"
    1 -> parts[0]
    else -> parts.dropLast(1).joinToString(", ") + " and " + parts.last()
}

/** Where a plugin may connect, in words; a host it names is shown as it is. */
private fun reachText(p: PluginInfo): String {
    val parts = LinkedHashSet<String>()
    for (h in p.fetchHosts) {
        parts += when {
            h.startsWith("$") -> p.settings.firstOrNull { it.key == h.drop(1) }?.let { "what you enter in ${it.label}" } ?: continue
            h == "*" -> "any public site"
            h == "local-network" -> "devices on your home network"
            h == "localhost" || h == "127.0.0.1" || h == "::1" -> "this TV"
            else -> h
        }
    }
    val list = parts.toList()
    val reach = joined(list)
    return if (p.selfSignedServers && list.isNotEmpty()) "$reach, accepting a home server's own certificate" else reach
}

/**
 * What the plugin waits on the person for. A sign-in shows a QR code of its page when it has one,
 * where to go and the code to enter, large enough to read from the sofa; any other step is said
 * in words.
 */
@Composable
private fun AskCard(ask: PluginAsk) {
    val colors = LoungeTheme.colors
    val link = ask.link
    if (link == null && ask.code == null) {
        Text(ask.say, style = LoungeTheme.typography.meta, color = colors.houselight)
        return
    }
    Row(horizontalArrangement = Arrangement.spacedBy(24.dp), verticalAlignment = Alignment.CenterVertically) {
        ask.qrPngPath?.let { path ->
            AsyncImage(model = path, contentDescription = null, modifier = Modifier.size(200.dp).clip(RoundedCornerShape(12.dp)))
        }
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(if (link != null) "Sign in on your phone" else "Sign in", style = LoungeTheme.typography.title, color = colors.houselight)
            // With a page to open, the code follows it; without one, the plugin says where the code goes.
            val where = when {
                link == null -> ask.after
                ask.linkText != null -> "Scan this with its camera, or open ${ask.linkText}" + if (ask.code != null) " and enter" else ""
                else -> "Scan this with its camera, or tap the link in the phone companion" + if (ask.code != null) " and enter" else ""
            }
            where?.let { Text(it, style = LoungeTheme.typography.meta, color = colors.dust) }
            ask.code?.let { Text(it, style = LoungeTheme.typography.displayL, color = colors.limelight, maxLines = 1) }
            if (link != null) ask.after?.let { Text(it, style = LoungeTheme.typography.caption, color = colors.dust) }
        }
    }
}

@Composable
private fun StatusLine(p: PluginInfo) {
    val colors = LoungeTheme.colors
    val (text, color) = when {
        p.enabled && p.lastError != null -> p.lastError!! to colors.ember
        p.enabled && p.lastRefreshMs != null -> "${if (p.items == 1u) "1 item" else "${p.items} items"}, refreshed ${Words.ago(System.currentTimeMillis() - p.lastRefreshMs!!)}" to colors.signal
        p.enabled -> "Fetching for the first time…" to colors.dust
        !p.configured -> "Fill in the settings below, then turn it on." to colors.houselight
        // Off and ready: the switch row below says so.
        else -> return
    }
    Text(text, style = LoungeTheme.typography.meta, color = color)
}

/** How long after the keyboard goes away a Back still belongs to the press that sent it away. */
private const val KEYBOARD_BACK_MS = 300L

/** About three lines of `caption` in the sheet: help this long reads whole. */
private const val SHORT_HELP = 160

/** A setting's help as the TV shows it: whole when short, else its first sentence and a pointer to the phone. */
internal fun shortHelp(text: String): String {
    val trimmed = text.trim()
    if (trimmed.length <= SHORT_HELP) return trimmed
    val end = Regex("""[.!?](?=\s+["“‘(]?[A-Z0-9])""").find(trimmed)?.range?.last ?: return trimmed
    return trimmed.substring(0, end + 1) + " Your phone shows the full steps."
}

/**
 * A plugin setting as a TV text field: label above, value in `title`. Focus rests on the field as
 * a row, and OK opens the keyboard on it; Done, Back or leaving saves it. Passwords are masked;
 * toggles are a row. Text typed on the phone goes into the focused field.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun SettingField(setting: PluginSettingInfo, focusRequester: FocusRequester, alsoFocus: FocusRequester?, onCommit: (String?) -> Unit, onToggle: (Boolean) -> Unit) {
    val colors = LoungeTheme.colors
    val visuals = LoungeTheme.visuals
    val also = if (alsoFocus != null) Modifier.focusRequester(alsoFocus) else Modifier
    val label = setting.label + if (setting.required) "" else " (optional)"
    if (setting.kind == "toggle") {
        val on = (setting.value ?: setting.defaultValue) == "true"
        SheetRow(
            icon = null,
            title = label,
            subtitle = setting.help?.let { shortHelp(it) },
            subtitleLines = WHOLE,
            value = if (on) "On" else "Off",
            accent = if (on) colors.signal else colors.dust,
            onClick = { onToggle(!on) },
            focusRequester = focusRequester,
            modifier = also,
        )
        return
    }
    // A secret is never shown back; a password is also masked while it is typed.
    val secret = setting.kind == "password" || setting.kind == "secret_url"
    val masked = setting.kind == "password"
    // A list pasted from the phone has an entry a line; the TV's one line shows them side by side.
    val onOneLine = remember { VisualTransformation { text -> TransformedText(AnnotatedString(text.text.replace('\n', ' ')), OffsetMapping.Identity) } }
    var field by remember(setting.key) { mutableStateOf(TextFieldValue(setting.value ?: "")) }
    var committed by remember(setting.key) { mutableStateOf(setting.value ?: "") }
    var rowFocused by remember { mutableStateOf(false) }
    var editing by remember { mutableStateOf(false) }
    // Only a press that began on the row opens the keyboard, not the release of the one that opened the page.
    var pressed by remember { mutableStateOf(false) }
    val focused = rowFocused || editing
    val keyboard = LocalSoftwareKeyboardController.current
    val editor = remember { FocusRequester() }
    val scope = rememberCoroutineScope()
    // Read when an edit ends, which may be a later composition than the one it began in.
    val rowFocus = rememberUpdatedState(focusRequester)
    val save = rememberUpdatedState(onCommit)
    PhoneTextInput(active = focused) { typed ->
        val next = field.text + typed
        field = TextFieldValue(next, TextRange(next.length))
    }
    fun commit() {
        val text = field.text
        if (text != committed) {
            committed = text
            save.value(text.ifBlank { null })
        }
    }
    fun stopEditing() {
        commit()
        editing = false
        keyboard?.hide()
        // The row may be on its way out with its sheet, and then has no focus to take.
        runCatching { rowFocus.value.requestFocus() }
    }
    // Back with the keyboard up goes to the keyboard, which only puts itself away. Its going away
    // ends the edit, so that one press also returns to the row.
    val keyboardUp = rememberUpdatedState(WindowInsets.isImeVisible)
    var keyboardLeft by remember { mutableStateOf(false) }
    LaunchedEffect(editing) {
        if (!editing) return@LaunchedEffect
        // A keyboard never seen to come up cannot be seen to leave, and Back then works as below.
        snapshotFlow { keyboardUp.value }.first { it }
        snapshotFlow { keyboardUp.value }.first { !it }
        keyboardLeft = true
        stopEditing()
    }
    LaunchedEffect(keyboardLeft) {
        if (keyboardLeft) {
            delay(KEYBOARD_BACK_MS)
            keyboardLeft = false
        }
    }
    // While the field is open with the keyboard already away, Back returns to the row rather than
    // closing the sheet. The press that put the keyboard away is not passed on to the sheet either.
    val sheetBack = LocalSheetBack.current
    DisposableEffect(editing, keyboardLeft) {
        val handler = { if (editing) stopEditing(); true }
        if (editing || keyboardLeft) sheetBack.handler = handler
        onDispose { if (sheetBack.handler === handler) sheetBack.handler = null }
    }
    val placeholder = when {
        secret && setting.hasValue -> "•••••••• (saved)"
        setting.defaultValue != null -> setting.defaultValue!!
        else -> "Not set"
    }
    // What TalkBack says after the label while the field is empty.
    val state = when {
        field.text.isNotEmpty() -> null
        secret && setting.hasValue -> "Saved"
        setting.defaultValue != null -> "Not set, uses ${setting.defaultValue}"
        else -> "Not set"
    }
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text(label, style = LoungeTheme.typography.caption, color = colors.dust)
        Box(
            Modifier
                .fillMaxWidth()
                .height(56.dp)
                .lightGlow(colors.houselight, if (focused) visuals.glowAlpha * 0.8f else 0f, cornerRadius = 12.dp)
                .clip(RoundedCornerShape(12.dp))
                .background(if (focused) colors.curtainEdge else colors.velvet)
                .focusRim(colors, { if (focused) 1f else 0f }, 12.dp)
                .focusRequester(focusRequester)
                .then(also)
                .onFocusChanged {
                    if (rowFocused && !it.isFocused && !editing) commit()
                    rowFocused = it.isFocused
                }
                .onPreviewKeyEvent { e ->
                    val ok = e.key == Key.DirectionCenter || e.key == Key.Enter || e.key == Key.NumPadEnter
                    if (editing || !ok) return@onPreviewKeyEvent false
                    when (e.type) {
                        KeyEventType.KeyDown -> pressed = true
                        KeyEventType.KeyUp -> if (pressed) {
                            pressed = false
                            editing = true
                            scope.launch { if (editor.requestWhenReady()) keyboard?.show() }
                        }
                    }
                    true
                }
                .focusable()
                .semantics {
                    contentDescription = label
                    state?.let { stateDescription = it }
                }
                .padding(horizontal = 16.dp),
            contentAlignment = Alignment.CenterStart,
        ) {
            if (field.text.isEmpty()) Text(placeholder, style = LoungeTheme.typography.title, color = colors.dust, maxLines = 1, overflow = TextOverflow.Ellipsis)
            BasicTextField(
                value = field,
                onValueChange = { field = it },
                singleLine = true,
                textStyle = LoungeTheme.typography.title.copy(color = colors.limelight),
                cursorBrush = SolidColor(colors.houselight),
                visualTransformation = when {
                    masked -> PasswordVisualTransformation()
                    setting.kind == "list" -> onOneLine
                    else -> VisualTransformation.None
                },
                keyboardOptions = KeyboardOptions(
                    imeAction = ImeAction.Done,
                    keyboardType = when (setting.kind) {
                        "url", "secret_url" -> KeyboardType.Uri
                        "number" -> KeyboardType.Number
                        "password" -> KeyboardType.Password
                        else -> KeyboardType.Text
                    },
                    showKeyboardOnFocus = false,
                ),
                keyboardActions = KeyboardActions(onDone = { stopEditing() }),
                modifier = Modifier
                    .fillMaxWidth()
                    .focusRequester(editor)
                    // Reached only through OK on the row, so walking down a page never raises the keyboard.
                    .focusProperties { canFocus = editing }
                    // The field would drop focus on Back's press; the sheet's release then returns to the row.
                    .onPreviewKeyEvent { e -> editing && e.key == Key.Back && e.type == KeyEventType.KeyDown }
                    .onFocusChanged {
                        if (editing && !it.isFocused) {
                            commit()
                            editing = false
                        }
                    }
                    .semantics { contentDescription = label },
            )
        }
        setting.help?.let { Text(shortHelp(it), style = LoungeTheme.typography.caption, color = colors.dust) }
    }
}
