package tv.lounge.feature.home

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.tv.material3.Text
import coil3.compose.AsyncImage
import tv.lounge.core.GroupInfo
import tv.lounge.core.PluginSettingInfo
import tv.lounge.core.Tile
import tv.lounge.design.Hairline
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme
import tv.lounge.design.sounds

/** The id of the group a home row shows, or null for any other row. */
internal fun groupOfRow(rowId: String): String? = rowId.removePrefix("group:").takeIf { it != rowId }

/** A text setting that is a group's name, for [SettingField]. */
private fun nameSetting(key: String, label: String, value: String?) =
    PluginSettingInfo(key = key, label = label, kind = "text", required = true, help = null, defaultValue = null, value = value, hasValue = value != null)

/** "3 apps", or that there are none yet. */
@Composable
private fun appsCounted(count: Int): String =
    if (count == 0) stringResource(R.string.groups_no_apps) else pluralStringResource(R.plurals.groups_apps, count, count)

/**
 * The groups of apps: one row for each, then a field that names a new one. With [app], OK on a
 * group puts that app in it or takes it out, and a new group starts with it. Without, OK opens
 * the group. [groups] is null until read.
 */
@Composable
fun GroupsSheet(
    app: Tile?,
    groups: List<GroupInfo>?,
    onToggle: (GroupInfo, Boolean) -> Unit,
    onOpen: (GroupInfo) -> Unit,
    onCreate: (String) -> Unit,
    firstFocus: FocusRequester,
    returnTo: String? = null,
    /** The apps on the TV and not hidden, which are the ones a group's row shows. Null counts them all. */
    onTv: Set<String>? = null,
) {
    val colors = LoungeTheme.colors
    val sounds = LoungeTheme.sounds
    val pkg = app?.id?.removePrefix("app:")
    val field = remember { FocusRequester() }
    // A new key empties the field once its name has been taken, and focus stays in it.
    var made by remember { mutableIntStateOf(0) }
    // Focus opens on the group just left, or on the first when that one has gone.
    val opensOn = groups?.firstOrNull { it.id == returnTo }?.id ?: groups?.firstOrNull()?.id
    var focusedGroup by remember { mutableStateOf<String?>(null) }
    val full = (groups?.size ?: 0) >= MOST_GROUPS

    SheetHeading(
        stringResource(R.string.groups_title),
        if (app != null) stringResource(R.string.groups_subtitle_app, app.title) else stringResource(R.string.groups_subtitle),
    )
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        when {
            groups == null -> Unit
            groups.isEmpty() -> Text(stringResource(R.string.groups_none), style = LoungeTheme.typography.meta, color = colors.dust)
            else -> groups.forEach { group ->
                key(group.id) {
                    val has = pkg != null && pkg in group.apps
                    SheetRow(
                        icon = LoungeIcons.Apps,
                        title = group.name,
                        subtitle = appsCounted(group.apps.count { onTv == null || it in onTv }),
                        modifier = Modifier.onFocusChanged { if (it.hasFocus) focusedGroup = group.id },
                        value = if (pkg == null) null else stringResource(if (has) R.string.groups_added else R.string.groups_not_added),
                        accent = if (has) colors.signal else colors.dust,
                        onClick = {
                            if (pkg == null) {
                                onOpen(group)
                            } else {
                                sounds?.tick()
                                onToggle(group, !has)
                            }
                        },
                        onCycle = if (pkg == null) null else { _ -> sounds?.tick(); onToggle(group, !has) },
                        focusRequester = if (group.id == opensOn) firstFocus else null,
                    )
                }
            }
        }
        Spacer(Modifier.height(8.dp))
        if (!full) SettingField(
            setting = nameSetting("new-group-$made", stringResource(R.string.groups_new_label), null),
            focusRequester = field,
            alsoFocus = null,
            onCommit = { name ->
                if (!name.isNullOrBlank()) {
                    onCreate(name)
                    made++
                }
            },
            onToggle = {},
            hint = stringResource(R.string.groups_new_hint),
        )
    }
    SheetFootnote(stringResource(R.string.groups_footnote))
    // The list can arrive after the sheet's own request for focus has given up. With no group
    // to land on, focus starts in the field.
    LaunchedEffect(groups != null) { if (groups != null) (if (groups.isEmpty()) field else firstFocus).requestWhenReady() }
    // A group deleted while it had focus hands focus to what is left.
    LaunchedEffect(groups?.map { it.id }) {
        val gone = focusedGroup?.takeIf { id -> groups?.none { it.id == id } == true } ?: return@LaunchedEffect
        if (focusedGroup == gone) focusedGroup = null
        (if (groups.isNullOrEmpty()) field else firstFocus).requestWhenReady()
    }
}

/** The core keeps no more groups than this, so the field that names a new one leaves at that many. */
internal const val MOST_GROUPS = 24

/**
 * One group: its name, every app on the TV with those in the group first, and the way to delete
 * it. OK on an app puts it in or takes it out. The apps keep the order they were first listed in,
 * so a row does not move from under the press that changed it.
 */
@Composable
fun GroupSheet(
    group: GroupInfo,
    apps: List<Tile>,
    onRename: (String) -> Unit,
    onToggle: (Tile, Boolean) -> Unit,
    onDelete: () -> Unit,
    firstFocus: FocusRequester,
) {
    val colors = LoungeTheme.colors
    val sounds = LoungeTheme.sounds
    var confirmDelete by remember { mutableStateOf(false) }
    // A name left empty is not taken, and a new key puts the old one back in the field.
    var refused by remember { mutableIntStateOf(0) }
    val listed = remember(group.id, apps.isEmpty()) {
        val by = apps.associateBy { it.id.removePrefix("app:") }
        val inGroup = group.apps.mapNotNull { by[it] }
        inGroup + apps.filter { it !in inGroup && !it.hidden }
    }

    SheetHeading(group.name, stringResource(R.string.group_subtitle))
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        SettingField(
            setting = nameSetting("group-name-${group.id}-$refused", stringResource(R.string.group_name_label), group.name),
            focusRequester = firstFocus,
            alsoFocus = null,
            onCommit = { name -> if (name.isNullOrBlank()) refused++ else onRename(name) },
            onToggle = {},
        )
        SheetGroup(stringResource(R.string.group_apps_heading))
        listed.forEach { app ->
            key(app.id) {
                val has = app.id.removePrefix("app:") in group.apps
                SheetRow(
                    icon = if (app.iconPath == null) LoungeIcons.Apps else null,
                    leading = app.iconPath?.let { path ->
                        { AsyncImage(model = path, contentDescription = null, contentScale = ContentScale.Fit, modifier = Modifier.size(28.dp).clip(RoundedCornerShape(7.dp))) }
                    },
                    title = app.title,
                    value = stringResource(if (has) R.string.groups_added else R.string.groups_not_added),
                    accent = if (has) colors.signal else colors.dust,
                    onClick = { sounds?.tick(); onToggle(app, !has) },
                    onCycle = { _ -> sounds?.tick(); onToggle(app, !has) },
                )
            }
        }
        Spacer(Modifier.height(8.dp))
        Hairline(Modifier.fillMaxWidth())
        // A deleted group cannot be had back, so it asks once more first; moving away takes the question back.
        SheetRow(
            icon = LoungeIcons.Trash,
            title = stringResource(if (confirmDelete) R.string.group_delete_confirm else R.string.group_delete),
            titleColor = if (confirmDelete) colors.ember else colors.limelight,
            subtitle = stringResource(R.string.group_delete_subtitle),
            modifier = Modifier.onFocusChanged { if (!it.isFocused) confirmDelete = false },
            onClick = {
                if (!confirmDelete) {
                    confirmDelete = true
                } else {
                    sounds?.close()
                    onDelete()
                }
            },
        )
    }
    // The group is read after the sheet opens, by when the sheet's own request for focus may have given up.
    LaunchedEffect(group.id) { firstFocus.requestWhenReady() }
}
