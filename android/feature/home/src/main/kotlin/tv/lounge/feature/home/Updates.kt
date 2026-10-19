package tv.lounge.feature.home

import android.app.Activity
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
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.tv.material3.Text
import kotlin.math.roundToInt
import kotlinx.coroutines.launch
import tv.lounge.design.Hairline
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme
import tv.lounge.update.InstallPath
import tv.lounge.update.UpdateManager
import tv.lounge.update.UpdateState

/** The one word the Settings row shows for the updater. */
fun updateStatusWord(state: UpdateState, auto: Boolean): String = when (state) {
    UpdateState.Checking -> "Checking…"
    is UpdateState.Available, is UpdateState.Downloading, is UpdateState.Ready, is UpdateState.NeedsConfirmation -> "${state.release?.version} ready"
    is UpdateState.Installing -> "Installing…"
    is UpdateState.Failed -> "Problem"
    is UpdateState.UpToDate -> "Up to date"
    is UpdateState.Idle -> if (auto) "On" else "Manual"
}

private fun ago(ms: Long): String {
    if (ms <= 0) return "never"
    val minutes = (System.currentTimeMillis() - ms) / 60_000
    return when {
        minutes < 2 -> "just now"
        minutes < 60 -> "$minutes min ago"
        minutes < 48 * 60 -> "${minutes / 60} h ago"
        else -> "${minutes / (24 * 60)} days ago"
    }
}

/** Settings → Updates: where Lounge stands against its releases and what, if anything, needs a press. */
@Composable
fun UpdatesSheet(updates: UpdateManager, firstFocus: FocusRequester) {
    val colors = LoungeTheme.colors
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val state by updates.state.collectAsStateWithLifecycle()
    val auto by updates.autoCheck.collectAsStateWithLifecycle()
    val activity = context as? Activity
    fun act() { activity?.let { updates.interact(it, scope) } }

    SheetHeading("Updates")
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        SheetRow(
            icon = LoungeIcons.Info,
            title = "Lounge ${updates.currentVersion}",
            value = if (state is UpdateState.Idle) "Not checked yet" else updateStatusWord(state, auto),
            accent = if (state is UpdateState.UpToDate) colors.signal else colors.dust,
            subtitle = when (val s = state) {
                is UpdateState.UpToDate -> "Checked ${ago(s.checkedAtMs)}"
                is UpdateState.Idle -> "Press to check now"
                is UpdateState.Failed -> s.message
                else -> null
            },
            onClick = { scope.launch { updates.check() } },
            focusRequester = firstFocus,
        )
        SheetRow(
            icon = LoungeIcons.Refresh,
            title = "Check twice a day",
            value = if (auto) "On" else "Off",
            accent = if (auto) colors.signal else colors.dust,
            onClick = { updates.setAutoCheck(!auto) },
        )
        when (val s = state) {
            is UpdateState.Available -> SheetRow(icon = LoungeIcons.Refresh, title = "Get Lounge ${s.release.version}", value = "Download", onClick = ::act)
            is UpdateState.Downloading -> SheetRow(icon = LoungeIcons.Refresh, title = "Downloading Lounge ${s.release.version}", value = "${(s.progress * 100).roundToInt()} %", onClick = {})
            is UpdateState.Ready -> SheetRow(
                icon = LoungeIcons.Power,
                title = if (s.path == InstallPath.NeedsPermission) "Allow Lounge to install updates" else "Install Lounge ${s.release.version}",
                value = when (s.path) {
                    InstallPath.NeedsPermission -> "Open"
                    InstallPath.Confirm -> "Confirm"
                    else -> "Now"
                },
                onClick = ::act,
            )
            is UpdateState.NeedsConfirmation -> SheetRow(icon = LoungeIcons.Power, title = "Install Lounge ${s.release.version}", value = "Confirm", onClick = ::act)
            is UpdateState.Installing -> SheetRow(icon = LoungeIcons.Power, title = "Installing Lounge ${s.release.version}…", onClick = {})
            is UpdateState.Failed -> SheetRow(icon = LoungeIcons.Refresh, title = "Try again", value = s.release?.let { "Lounge ${it.version}" }, onClick = ::act)
            else -> Unit
        }
        state.release?.let { r ->
            val summary = r.summary()
            if (summary.isNotEmpty()) {
                Spacer(Modifier.height(8.dp))
                Text("What's new in ${r.version}", style = LoungeTheme.typography.meta, color = colors.limelight, modifier = Modifier.padding(horizontal = 16.dp))
                Text(summary, style = LoungeTheme.typography.caption, color = colors.dust, modifier = Modifier.padding(horizontal = 16.dp))
            }
        }
        Spacer(Modifier.height(16.dp))
        Hairline(Modifier.fillMaxWidth())
        Spacer(Modifier.height(8.dp))
        val how = when (updates.currentPath()) {
            InstallPath.Silent -> "Updates install by themselves while you are in another app, and Lounge comes back as it was."
            InstallPath.PowerSetup -> "Power Setup installs updates without asking, while you are in another app."
            InstallPath.Confirm -> "Android asks you to confirm this update, and Play Protect may add that it has not seen this developer: choose More details, then Install anyway. After this one, updates install by themselves."
            InstallPath.NeedsPermission -> "Android does not let Lounge install apps yet. Allow it in Settings → Apps → Special app access → Install unknown apps, or connect Power Setup."
        }
        Text("Lounge looks at its releases on GitHub twice a day. $how", style = LoungeTheme.typography.caption, color = colors.dust, modifier = Modifier.padding(horizontal = 16.dp))
        if (updates.currentPath() == InstallPath.NeedsPermission) {
            Spacer(Modifier.height(8.dp))
            SheetRow(icon = LoungeIcons.Settings, title = "Allow Lounge to install updates", value = "Open", onClick = { activity?.let { updates.openInstallPermission(it) } })
        }
    }
}
