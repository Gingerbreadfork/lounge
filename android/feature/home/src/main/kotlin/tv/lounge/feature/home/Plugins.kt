package tv.lounge.feature.home

import androidx.compose.foundation.background
import androidx.compose.foundation.focusable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.tv.material3.Text
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import tv.lounge.core.CoreClient
import tv.lounge.core.PluginInfo
import tv.lounge.core.PluginSettingInfo
import tv.lounge.design.Hairline
import tv.lounge.design.LocalSwapSlotActive
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme
import tv.lounge.design.focusRing
import tv.lounge.design.lightGlow

/** Installed plugins: one row each with its state. Selecting one opens its detail sheet. */
@Composable
fun PluginsSheet(core: CoreClient, onOpen: (String) -> Unit, firstFocus: FocusRequester, cameFrom: Overlay? = null) {
    val colors = LoungeTheme.colors
    var plugins by remember { mutableStateOf<List<PluginInfo>?>(null) }
    // Each read waits on the core's lock, which a big refresh can hold, so the reads run off the main thread.
    WhileStarted {
        while (true) {
            plugins = withContext(Dispatchers.IO) { core.plugins() }
            delay(2000)
        }
    }
    // The rows land after the sheet first asks for focus; a copy being swapped out leaves focus alone.
    val live = LocalSwapSlotActive.current
    LaunchedEffect(plugins != null) { if (plugins != null && live.value) firstFocus.requestWhenReady() }
    val list = plugins
    // Back from a plugin's page puts focus on that plugin, if it is still there.
    val returnTo = (cameFrom as? Overlay.PluginDetail)?.id?.takeIf { id -> list.orEmpty().any { it.id == id } }
    SheetHeading("Plugins", "Sources that add rows, channels and guide data. Each runs in its own sandbox.")
    if (list == null) {
        Text("Loading…", style = LoungeTheme.typography.meta, color = colors.dust)
    } else if (list.isEmpty()) {
        Text("No plugins installed.", style = LoungeTheme.typography.meta, color = colors.dust)
    } else {
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            list.forEachIndexed { i, p ->
                SheetRow(
                    icon = LoungeIcons.Apps,
                    title = p.name,
                    value = statusWord(p),
                    accent = if (p.enabled && p.lastError == null) colors.signal else if (p.lastError != null) colors.ember else colors.dust,
                    onClick = { onOpen(p.id) },
                    focusRequester = if (p.id == returnTo || (returnTo == null && i == 0)) firstFocus else null,
                )
            }
        }
    }
    SheetFootnote("Plugins can only reach the hosts they declare, and only see their own settings and storage.", "More plugins install from the companion app on your phone.")
}

private fun statusWord(p: PluginInfo): String = when {
    p.lastError != null && p.enabled -> "Problem"
    p.enabled -> "On"
    !p.configured -> "Needs setup"
    else -> "Off"
}

/**
 * One plugin: what it is, whether it runs, its settings as editable fields, its log. Built-in
 * plugins can be turned off but not removed.
 */
@Composable
fun PluginDetailSheet(core: CoreClient, pluginId: String, onRemoved: () -> Unit, firstFocus: FocusRequester) {
    val colors = LoungeTheme.colors
    var info by remember { mutableStateOf<PluginInfo?>(null) }
    var loaded by remember { mutableStateOf(false) }
    var showLog by remember { mutableStateOf(false) }
    var log by remember { mutableStateOf<List<String>?>(null) }
    val scope = rememberCoroutineScope()
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
        SheetRow(icon = LoungeIcons.Info, title = "Back", onClick = onRemoved, focusRequester = firstFocus)
        return
    }

    SheetHeading(p.name, "v${p.version}" + if (p.publisherName.isNotEmpty()) " · by ${p.publisherName}" else "")
    if (p.description.isNotEmpty()) {
        Text(p.description, style = LoungeTheme.typography.meta, color = colors.dust)
        Spacer(Modifier.height(20.dp))
    }
    StatusLine(p)
    Spacer(Modifier.height(20.dp))

    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        SheetRow(
            icon = LoungeIcons.Power,
            title = if (p.enabled) "On" else "Off",
            value = when {
                p.enabled -> "Turn off"
                p.configured -> "Turn on"
                else -> "Set up first"
            },
            accent = if (p.enabled) colors.signal else colors.dust,
            onClick = {
                scope.launch {
                    info = withContext(Dispatchers.IO) {
                        if (p.enabled || p.configured) core.setPluginEnabled(p.id, !p.enabled)
                        core.plugins().firstOrNull { it.id == pluginId }
                    }
                }
            },
            focusRequester = firstFocus,
        )
        if (p.settings.isNotEmpty()) {
            Spacer(Modifier.height(8.dp))
            Text("Settings", style = LoungeTheme.typography.meta, color = colors.dust)
            p.settings.forEach { s ->
                SettingField(
                    setting = s,
                    onCommit = { value -> scope.launch { withContext(Dispatchers.IO) { core.setPluginSetting(p.id, s.key, value) } } },
                    onToggle = { on -> scope.launch { withContext(Dispatchers.IO) { core.setPluginSetting(p.id, s.key, if (on) "true" else "false") } } },
                )
            }
            Spacer(Modifier.height(8.dp))
        }
        if (p.enabled) {
            SheetRow(icon = LoungeIcons.Move, title = "Refresh now", onClick = { core.refreshPlugin(p.id) })
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
        "Can connect to: ${reachText(p)}.",
        "Signed by ${p.publisher.take(16)}… Refreshes every ${p.refreshMinutes} min while the TV is on.",
    )
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
    return when (list.size) {
        0 -> "nothing"
        1 -> list[0]
        else -> list.dropLast(1).joinToString(", ") + " and " + list.last()
    }
}

@Composable
private fun StatusLine(p: PluginInfo) {
    val colors = LoungeTheme.colors
    val (text, color) = when {
        p.enabled && p.lastError != null -> p.lastError!! to colors.ember
        p.enabled && p.lastRefreshMs != null -> "${p.items} items · refreshed ${agoText(p.lastRefreshMs!!)}" to colors.signal
        p.enabled -> "Fetching for the first time…" to colors.dust
        !p.configured -> "Fill in the settings below, then turn it on." to colors.houselight
        else -> "Off" to colors.dust
    }
    Text(text, style = LoungeTheme.typography.meta, color = color, maxLines = 3, overflow = TextOverflow.Ellipsis)
}

private fun agoText(ms: Long): String {
    val minutes = (System.currentTimeMillis() - ms) / 60_000
    return when {
        minutes < 1 -> "just now"
        minutes < 60 -> "$minutes min ago"
        minutes < 1440 -> "${minutes / 60} h ago"
        else -> "${minutes / 1440} d ago"
    }
}

/**
 * A plugin setting as a TV text field: label above, value in `title`, saved when the keyboard's
 * Done is pressed or focus leaves. Passwords are masked; toggles are a row. Text typed on the phone
 * goes into the focused field.
 */
@Composable
private fun SettingField(setting: PluginSettingInfo, onCommit: (String?) -> Unit, onToggle: (Boolean) -> Unit) {
    val colors = LoungeTheme.colors
    val visuals = LoungeTheme.visuals
    if (setting.kind == "toggle") {
        val on = (setting.value ?: setting.defaultValue) == "true"
        SheetRow(icon = null, title = setting.label, value = if (on) "On" else "Off", onClick = { onToggle(!on) })
        return
    }
    // A secret is never shown back; a password is also masked while it is typed.
    val secret = setting.kind == "password" || setting.kind == "secret_url"
    val masked = setting.kind == "password"
    var field by remember(setting.key) { mutableStateOf(TextFieldValue(setting.value ?: "")) }
    var committed by remember(setting.key) { mutableStateOf(setting.value ?: "") }
    var focused by remember { mutableStateOf(false) }
    val keyboard = LocalSoftwareKeyboardController.current
    PhoneTextInput(active = focused) { typed ->
        val next = field.text + typed
        field = TextFieldValue(next, TextRange(next.length))
    }
    fun commit() {
        val text = field.text
        if (text != committed) {
            committed = text
            onCommit(text.ifBlank { null })
        }
    }
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text(
            setting.label + if (setting.required) "" else " (optional)",
            style = LoungeTheme.typography.caption,
            color = colors.dust,
        )
        Box(
            Modifier
                .fillMaxWidth()
                .height(56.dp)
                .lightGlow(colors.houselight, if (focused) visuals.glowAlpha * 0.8f else 0f, cornerRadius = 12.dp)
                .clip(RoundedCornerShape(12.dp))
                .background(if (focused) colors.curtainEdge else colors.velvet)
                .focusRing(colors, { focused }, 12.dp)
                .padding(horizontal = 16.dp),
            contentAlignment = Alignment.CenterStart,
        ) {
            val placeholder = when {
                secret && setting.hasValue -> "•••••••• (saved)"
                setting.defaultValue != null -> setting.defaultValue!!
                else -> "Not set"
            }
            if (field.text.isEmpty()) Text(placeholder, style = LoungeTheme.typography.title, color = colors.dust, maxLines = 1, overflow = TextOverflow.Ellipsis)
            BasicTextField(
                value = field,
                onValueChange = { field = it },
                singleLine = true,
                textStyle = LoungeTheme.typography.title.copy(color = colors.limelight),
                cursorBrush = SolidColor(colors.houselight),
                visualTransformation = if (masked) PasswordVisualTransformation() else VisualTransformation.None,
                keyboardOptions = KeyboardOptions(
                    imeAction = ImeAction.Done,
                    keyboardType = when (setting.kind) {
                        "url", "secret_url" -> KeyboardType.Uri
                        "number" -> KeyboardType.Number
                        "password" -> KeyboardType.Password
                        else -> KeyboardType.Text
                    },
                ),
                keyboardActions = KeyboardActions(onDone = { commit(); keyboard?.hide() }),
                modifier = Modifier
                    .fillMaxWidth()
                    .onFocusChanged { if (focused && !it.isFocused) commit(); focused = it.isFocused },
            )
        }
        setting.help?.let { Text(it, style = LoungeTheme.typography.caption, color = colors.dust) }
    }
}
