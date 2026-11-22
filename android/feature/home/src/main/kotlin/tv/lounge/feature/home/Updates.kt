package tv.lounge.feature.home

import android.app.Activity
import android.content.res.Resources
import androidx.annotation.StringRes
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.tv.material3.Text
import kotlin.math.roundToInt
import kotlinx.coroutines.launch
import tv.lounge.design.Hairline
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme
import tv.lounge.design.R as DesignR
import tv.lounge.design.Words
import tv.lounge.update.InstallPath
import tv.lounge.update.UpdateManager
import tv.lounge.update.UpdateState

/** A word or two for where the updater stands: the text, and the version that fills its place when it has one. */
data class UpdateWord(@StringRes val text: Int, val version: String? = null)

@Composable
fun UpdateWord.shown(): String = if (version == null) stringResource(text) else stringResource(text, version)

/** The one word the Settings row shows for the updater. */
fun updateStatusWord(state: UpdateState, auto: Boolean): UpdateWord = when (state) {
    UpdateState.Checking -> UpdateWord(R.string.updates_word_checking)
    is UpdateState.Available -> UpdateWord(R.string.updates_word_available, state.release.version.toString())
    is UpdateState.Downloading -> UpdateWord(R.string.updates_word_downloading)
    is UpdateState.Ready, is UpdateState.NeedsConfirmation -> UpdateWord(R.string.updates_word_ready, state.release?.version.toString())
    is UpdateState.Installing -> UpdateWord(R.string.updates_word_installing)
    is UpdateState.Failed -> UpdateWord(R.string.updates_word_problem)
    is UpdateState.UpToDate -> UpdateWord(R.string.updates_word_up_to_date)
    is UpdateState.Idle -> UpdateWord(if (auto) DesignR.string.common_on else R.string.updates_word_manual)
}

/** When the last check was, as a line: [ms] is its time, and nothing or less means there has been none. */
private fun checked(resources: Resources, ms: Long): String =
    if (ms <= 0) resources.getString(R.string.updates_checked_never) else resources.getString(R.string.updates_checked_ago, Words.ago(resources, System.currentTimeMillis() - ms))

/** Updates, from Settings: where Lounge stands against its releases and what, if anything, needs a press. */
@Composable
fun UpdatesSheet(updates: UpdateManager, firstFocus: FocusRequester) {
    val colors = LoungeTheme.colors
    val context = LocalContext.current
    val resources = LocalResources.current
    val scope = rememberCoroutineScope()
    val state by updates.state.collectAsStateWithLifecycle()
    val auto by updates.autoCheck.collectAsStateWithLifecycle()
    val activity = context as? Activity
    fun act() { activity?.let { updates.interact(it, scope) } }

    // What OK says first on a row that opens Android's list of apps allowed to install.
    val installHint = stringResource(R.string.updates_install_hint)
    SheetHeading(stringResource(R.string.updates_title))
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        SheetRow(
            icon = LoungeIcons.Info,
            title = "Lounge ${updates.currentVersion}",
            value = if (state is UpdateState.Idle) stringResource(R.string.updates_not_checked) else updateStatusWord(state, auto).shown(),
            accent = when (state) {
                is UpdateState.UpToDate -> colors.signal
                is UpdateState.Failed -> colors.ember
                else -> colors.dust
            },
            subtitle = when (val s = state) {
                is UpdateState.UpToDate -> checked(resources, s.checkedAtMs)
                is UpdateState.Idle -> stringResource(R.string.updates_press_to_check)
                is UpdateState.Failed -> s.message
                else -> null
            },
            subtitleLines = WHOLE,
            onClick = { scope.launch { updates.check() } },
            focusRequester = firstFocus,
        )
        SheetRow(
            icon = SettingsGlyphs.Clock,
            title = stringResource(R.string.updates_check_twice),
            value = stringResource(if (auto) DesignR.string.common_on else DesignR.string.common_off),
            accent = if (auto) colors.signal else colors.dust,
            onClick = { updates.setAutoCheck(!auto) },
        )
        when (val s = state) {
            is UpdateState.Available -> SheetRow(icon = SettingsGlyphs.Download, title = stringResource(R.string.updates_get, s.release.version), value = stringResource(R.string.updates_download), accent = colors.houselight, onClick = ::act)
            is UpdateState.Downloading -> SheetRow(icon = SettingsGlyphs.Download, title = stringResource(R.string.updates_downloading, s.release.version), value = stringResource(R.string.updates_percent, (s.progress * 100).roundToInt()), onClick = {})
            is UpdateState.Ready -> SheetRow(
                icon = if (s.path == InstallPath.NeedsPermission) LoungeIcons.Settings else SettingsGlyphs.Download,
                title = if (s.path == InstallPath.NeedsPermission) stringResource(R.string.updates_allow_install) else stringResource(R.string.updates_install, s.release.version),
                subtitle = if (s.path == InstallPath.NeedsPermission) installHint else null,
                accent = colors.houselight,
                value = when (s.path) {
                    InstallPath.NeedsPermission -> stringResource(DesignR.string.common_open)
                    InstallPath.Confirm -> stringResource(R.string.updates_confirm)
                    else -> stringResource(R.string.updates_now)
                },
                onClick = ::act,
            )
            is UpdateState.NeedsConfirmation -> SheetRow(icon = SettingsGlyphs.Download, title = stringResource(R.string.updates_install, s.release.version), value = stringResource(R.string.updates_confirm), accent = colors.houselight, onClick = ::act)
            is UpdateState.Installing -> SheetRow(icon = SettingsGlyphs.Download, title = stringResource(R.string.updates_installing, s.release.version), onClick = {})
            is UpdateState.Failed -> SheetRow(icon = LoungeIcons.Refresh, title = stringResource(DesignR.string.common_try_again), value = s.release?.let { "Lounge ${it.version}" }, onClick = ::act)
            else -> Unit
        }
        state.release?.let { r ->
            val summary = r.summary()
            if (summary.isNotEmpty()) {
                Spacer(Modifier.height(8.dp))
                Text(stringResource(R.string.updates_whats_new, r.version), style = LoungeTheme.typography.meta, color = colors.limelight, modifier = Modifier.padding(horizontal = 16.dp))
                Text(summary, style = LoungeTheme.typography.caption, color = colors.dust, modifier = Modifier.padding(horizontal = 16.dp))
            }
        }
        Spacer(Modifier.height(16.dp))
        Hairline(Modifier.fillMaxWidth())
        Spacer(Modifier.height(8.dp))
        val path = updates.currentPath()
        val how = when (path) {
            InstallPath.Silent -> R.string.updates_how_silent
            InstallPath.PowerSetup -> R.string.updates_how_power_setup
            InstallPath.Confirm -> R.string.updates_how_confirm
            InstallPath.NeedsPermission -> R.string.updates_how_needs_permission
        }
        Text(stringResource(how), style = LoungeTheme.typography.caption, color = colors.dust, modifier = Modifier.padding(horizontal = 16.dp))
        if (path == InstallPath.NeedsPermission) {
            Spacer(Modifier.height(8.dp))
            SheetRow(icon = LoungeIcons.Settings, title = stringResource(R.string.updates_allow_install), subtitle = installHint, onClick = { activity?.let { updates.askForInstallPermission(it, scope) } })
        }
    }
}
