package tv.lounge.feature.home

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.tv.material3.Icon
import androidx.tv.material3.Text
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import tv.lounge.core.CoreClient
import tv.lounge.core.PluginSheet
import tv.lounge.core.PluginSheetRow
import tv.lounge.core.PluginSheetRowKind
import tv.lounge.core.reason
import tv.lounge.design.LoungeGlyphs
import tv.lounge.design.LoungeTheme

/** How often an open sheet is read again: often while it is in use, seldom once it is left alone. */
internal const val SHEET_READ_BUSY_MS = 3_000L
internal const val SHEET_READ_QUIET_MS = 15_000L
internal const val SHEET_BUSY_FOR_MS = 60_000L

/** How long a row's question waits for its second press. */
internal const val CONFIRM_WAITS_MS = 8_000L

/** Whether focus can rest on [row]: a row to read and a group's label take none. */
internal fun PluginSheetRow.takesFocus(): Boolean = kind != PluginSheetRowKind.Text && kind != PluginSheetRowKind.Group

/**
 * The row that takes focus when the focused one has left the sheet: the one now in its place, or
 * the last. Null while the focused row is still there, or when nothing can take focus.
 */
internal fun focusAfter(rows: List<String>, focused: String?, at: Int): String? {
    if (focused == null || focused in rows || rows.isEmpty()) return null
    return rows[at.coerceIn(0, rows.lastIndex)]
}

/**
 * The row focus opens on. Coming back to a sheet, it is the row focus was left on, else the one
 * that leads to the sheet Back came from; a sheet opened afresh starts on its first row.
 */
internal fun firstFocusRow(rows: List<PluginSheetRow>, cameFrom: String?, leftOn: String? = null): String? {
    val focusable = rows.filter { it.takesFocus() }
    val left = focusable.firstOrNull { it.id == leftOn }
    val back = focusable.firstOrNull { (it.kind as? PluginSheetRowKind.Open)?.sheet == cameFrom && cameFrom != null }
    return (left ?: back ?: focusable.firstOrNull())?.id
}

/**
 * The one row that has asked its question and waits for a second press. A row whose action has
 * changed since it asked is no longer waiting, so the second press never does something else.
 */
@Stable
internal class ConfirmArm {
    private var armed by mutableStateOf<Pair<String, String>?>(null)

    val waiting: Boolean get() = armed != null

    fun armedFor(row: String, action: String): Boolean = armed == row to action

    /** True when this press is the second, and the row should act. */
    fun press(row: String, action: String): Boolean {
        if (armedFor(row, action)) {
            armed = null
            return true
        }
        armed = row to action
        return false
    }

    fun disarm() {
        armed = null
    }

    /** Takes the question back when the row that asked it no longer asks it, as [stillAsks] tells. */
    fun settle(stillAsks: (row: String, action: String) -> Boolean) {
        armed?.let { (row, action) -> if (!stillAsks(row, action)) armed = null }
    }

    /** Takes the question back when [row] is the one that asked it. */
    fun leave(row: String) {
        if (armed?.first == row) armed = null
    }
}

/**
 * A sheet a plugin describes, kept up to date while it is open. Left and Right step a row's
 * value, OK presses it, and a row that asks first acts on the second OK (DESIGN.md §5.4).
 */
@Composable
fun PluginSheetContent(
    core: CoreClient,
    plugin: String,
    id: String,
    firstFocus: FocusRequester,
    /** The plugin sheet Back last came from, so the row that leads to it takes focus. */
    cameFrom: String?,
    /** The row focus was on when this sheet was last left for something it opened. */
    leftOn: String?,
    onRowFocused: (String) -> Unit,
    /** Emits when the home changes, which a press on this sheet may have caused. */
    changes: Flow<*>,
    say: (String) -> Unit,
    onOpen: (String) -> Unit,
) {
    val scope = rememberCoroutineScope()
    val resources by rememberUpdatedState(LocalResources.current)
    var sheet by remember(plugin, id) { mutableStateOf<PluginSheet?>(null) }
    var problem by remember(plugin, id) { mutableStateOf<String?>(null) }
    var stale by remember(plugin, id) { mutableStateOf(false) }
    val failed = remember(plugin, id) { mutableStateMapOf<String, String>() }
    val confirm = remember(plugin, id) { ConfirmArm() }
    val reads = remember(plugin, id) { Channel<Unit>(Channel.CONFLATED) }
    // One press at a time, and one more waiting: a held key steps as fast as the plugin answers.
    val presses = remember(plugin, id) { Channel<Pair<String, String>>(1, BufferOverflow.DROP_LATEST) }
    val lastKeyAt = remember(plugin, id) { longArrayOf(System.currentTimeMillis()) }

    WhileStarted(plugin, id) {
        launch { changes.drop(1).collect { reads.trySend(Unit) } }
        launch {
            while (true) {
                delay(if (System.currentTimeMillis() - lastKeyAt[0] < SHEET_BUSY_FOR_MS) SHEET_READ_BUSY_MS else SHEET_READ_QUIET_MS)
                reads.trySend(Unit)
            }
        }
        reads.trySend(Unit)
        for (unused in reads) {
            withContext(Dispatchers.IO) { core.pluginSheet(plugin, id) }
                .onSuccess {
                    sheet = it
                    problem = null
                    stale = false
                }
                .onFailure { e ->
                    // What was read before stays up; a sheet never read says why.
                    if (sheet == null) problem = sheetProblem(resources, e) else stale = true
                }
            delay(150)
        }
    }
    LaunchedEffect(plugin, id) {
        for ((row, action) in presses) {
            failed.remove(row)
            withContext(Dispatchers.IO) { core.pluginSheetPress(plugin, action) }
                .onSuccess { done ->
                    done.say?.let(say)
                    reads.trySend(Unit)
                    // Once more for the refresh the press brings, and for a device that reports late.
                    scope.launch {
                        delay(700)
                        reads.trySend(Unit)
                        delay(1_800)
                        reads.trySend(Unit)
                    }
                }
                .onFailure { e -> failed[row] = sheetProblem(resources, e) }
        }
    }
    fun press(row: String, action: String) {
        presses.trySend(row to action)
    }

    // Back takes a question back before it closes the sheet.
    val back = LocalSheetBack.current
    DisposableEffect(back, confirm) {
        back.handler = { if (confirm.waiting) { confirm.disarm(); true } else false }
        onDispose { back.handler = null }
    }
    LaunchedEffect(confirm.waiting) {
        if (confirm.waiting) {
            delay(CONFIRM_WAITS_MS)
            confirm.disarm()
        }
    }

    val shown = sheet
    if (shown == null) {
        SheetHeading(stringResource(if (problem == null) R.string.plugin_sheets_one_moment else R.string.plugin_sheets_not_answering), problem)
        return
    }
    PluginSheetBody(
        sheet = shown,
        stale = stale,
        failed = failed,
        confirm = confirm,
        firstFocus = firstFocus,
        cameFrom = cameFrom,
        leftOn = leftOn,
        onRowFocused = onRowFocused,
        onKey = { lastKeyAt[0] = System.currentTimeMillis() },
        onPress = ::press,
        onOpen = onOpen,
    )
}

/** Why a sheet, or a press on one of its rows, failed: the plugin's own sentence, or that it did not answer. */
internal fun sheetProblem(resources: android.content.res.Resources, e: Throwable): String {
    val reason = e.reason
    android.util.Log.w("Lounge", "a plugin's sheet failed: $reason")
    return if (e is tv.lounge.core.CoreException.Plugin && readsAsSentence(reason)) reason.trim() else resources.getString(R.string.plugin_sheets_no_answer)
}

/**
 * What a plugin's sheet shows once it has been read. [failed] holds the reason for each row whose
 * last press did not work, and [confirm] the row that has asked its question.
 */
@Composable
internal fun PluginSheetBody(
    sheet: PluginSheet,
    stale: Boolean,
    failed: Map<String, String>,
    confirm: ConfirmArm,
    firstFocus: FocusRequester,
    cameFrom: String?,
    leftOn: String?,
    onRowFocused: (String) -> Unit,
    onKey: () -> Unit,
    onPress: (row: String, action: String) -> Unit,
    onOpen: (String) -> Unit,
) {
    val colors = LoungeTheme.colors
    val rows = sheet.rows
    val focusable = rows.filter { it.takesFocus() }.map { it.id }
    val rowFocus = remember { HashMap<String, FocusRequester>() }
    // The focused row and its place, so one that leaves the sheet hands focus to its neighbour.
    var focused by remember { mutableStateOf<Pair<String, Int>?>(null) }
    LaunchedEffect(focusable) {
        rowFocus.keys.retainAll(focusable.toSet())
        focusAfter(focusable, focused?.first, focused?.second ?: 0)?.let { rowFocus[it]?.requestWhenReady() }
    }
    // A row that has stopped asking, because the thing changed some other way, is not left armed.
    LaunchedEffect(rows) {
        confirm.settle { id, action -> rows.any { it.id == id && (it.kind as? PluginSheetRowKind.Confirm)?.action == action } }
    }
    val opensOn = remember { firstFocusRow(rows, cameFrom, leftOn) }
    // In a sheet with glyphs, a row without one keeps its place, so the titles read as one column.
    val glyphs = rows.any { LoungeGlyphs.named(it.glyph) != null }
    val gap: (@Composable RowScope.() -> Unit)? = if (glyphs) ({ Spacer(Modifier.size(28.dp)) }) else null
    // The rows arrive after the sheet does, so focus is asked for again once they are here.
    LaunchedEffect(Unit) { firstFocus.requestWhenReady() }

    SheetHeading(sheet.title, sheet.subtitle)
    Column(
        Modifier.onPreviewKeyEvent { e ->
            if (e.type == KeyEventType.KeyDown) onKey()
            // Back's key down would move focus off the row and take the question back with it,
            // so the sheet would close on the key up. It is kept here while a row is asking.
            e.isBack() && e.type == KeyEventType.KeyDown && confirm.waiting
        },
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        rows.forEach { row ->
            key(row.id) {
                val glyph = LoungeGlyphs.named(row.glyph)
                val reason = failed[row.id]
                val value = if (reason != null) stringResource(R.string.plugin_sheets_didnt_work) else row.value
                val accent = when {
                    reason != null -> colors.ember
                    row.on == true -> colors.houselight
                    else -> colors.dust
                }
                val at = focusable.indexOf(row.id)
                val modifier = Modifier
                    .focusRequester(rowFocus.getOrPut(row.id) { FocusRequester() })
                    .onFocusChanged {
                        if (it.isFocused) {
                            focused = row.id to at
                            onRowFocused(row.id)
                        } else {
                            confirm.leave(row.id)
                        }
                    }
                val first = firstFocus.takeIf { row.id == opensOn }
                val leading = gap.takeIf { glyph == null }
                // One row for every kind that takes focus, so a row that changes kind under the
                // same id, as a lock does once it is unlocked, keeps the focus it had.
                val kind = row.kind
                val question = (kind as? PluginSheetRowKind.Confirm)?.takeIf { confirm.armedFor(row.id, it.action) }?.ask
                val asking = question != null
                when (kind) {
                    PluginSheetRowKind.Group -> SheetGroup(row.title)
                    PluginSheetRowKind.Text -> SheetText(row, glyph, indent = glyphs)
                    else -> SheetRow(
                        icon = glyph,
                        leading = leading,
                        title = question ?: row.title,
                        titleColor = if (asking) colors.ember else colors.limelight,
                        subtitle = reason ?: row.subtitle.takeUnless { asking },
                        subtitleLines = if (reason != null) WHOLE else 2,
                        value = value.takeUnless { asking },
                        accent = if (asking) colors.ember else accent,
                        focusRequester = first,
                        modifier = modifier,
                        onCycle = (kind as? PluginSheetRowKind.Step)?.let { step -> { by: Int -> (if (by > 0) step.up else step.down)?.let { onPress(row.id, it) } ?: Unit } },
                        onClick = {
                            when (kind) {
                                is PluginSheetRowKind.Press -> onPress(row.id, kind.action)
                                is PluginSheetRowKind.Step -> kind.press?.let { onPress(row.id, it) }
                                is PluginSheetRowKind.Open -> onOpen(kind.sheet)
                                is PluginSheetRowKind.Confirm -> if (confirm.press(row.id, kind.action)) onPress(row.id, kind.action)
                                PluginSheetRowKind.Group, PluginSheetRowKind.Text -> Unit
                            }
                        },
                    )
                }
            }
        }
    }
    val notes = listOfNotNull(sheet.footnote, if (stale) stringResource(R.string.plugin_sheets_stale) else null)
    if (notes.isNotEmpty()) SheetFootnote(*notes.toTypedArray())
}

/** A row to read: the same measure as a row to press, with no focus and no surface. */
@Composable
private fun SheetText(row: PluginSheetRow, glyph: androidx.compose.ui.graphics.vector.ImageVector?, indent: Boolean) {
    val colors = LoungeTheme.colors
    Row(
        Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(horizontal = 20.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        if (glyph != null) {
            Icon(glyph, contentDescription = null, tint = if (row.on == true) colors.houselight else colors.dust, modifier = Modifier.size(28.dp))
        } else if (indent) {
            Spacer(Modifier.size(28.dp))
        }
        Column(Modifier.weight(1f).padding(vertical = 4.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(row.title, style = LoungeTheme.typography.title, color = colors.limelight, maxLines = 2, overflow = TextOverflow.Ellipsis)
            row.subtitle?.let { Text(it, style = LoungeTheme.typography.caption, color = colors.dust, maxLines = 3, overflow = TextOverflow.Ellipsis) }
        }
        row.value?.let { Text(it, style = LoungeTheme.typography.meta, color = if (row.on == true) colors.houselight else colors.dust, maxLines = 1) }
    }
}
