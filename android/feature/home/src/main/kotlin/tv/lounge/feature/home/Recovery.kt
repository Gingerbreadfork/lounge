package tv.lounge.feature.home

import androidx.compose.foundation.background
import androidx.compose.foundation.focusGroup
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.tv.material3.Text
import kotlinx.coroutines.launch
import tv.lounge.design.ButtonKind
import tv.lounge.design.Dimens
import tv.lounge.design.LoungeButton
import tv.lounge.design.LoungeTheme
import tv.lounge.design.R as DesignR

/** Why the home could not start. */
enum class RecoveryReason { Storage, StorageFull, Other }

/** Another home screen on the TV, such as the stock launcher Lounge replaced. */
data class OtherHome(val packageName: String, val label: String, val enabled: Boolean)

/** What did not open when asked: the TV's settings, or another home screen. */
private sealed interface Stuck {
    data object Settings : Stuck
    data class Home(val label: String) : Stuck
}

/**
 * Shown in place of the home when the core will not open, so the TV is never left without a way
 * on. [onTryAgain] returns when the attempt is over; if it worked, the caller shows the home.
 * [onOpenSettings] and [onOtherHome] return false when nothing could be opened.
 */
@Composable
fun RecoveryScreen(
    reason: RecoveryReason,
    otherHomes: List<OtherHome>,
    onTryAgain: suspend (fresh: Boolean) -> Unit,
    onOpenSettings: () -> Boolean,
    onOtherHome: (OtherHome) -> Boolean,
) {
    val colors = LoungeTheme.colors
    val type = LoungeTheme.typography
    val scope = rememberCoroutineScope()
    var busy by remember { mutableStateOf(false) }
    var failedAttempts by remember { mutableIntStateOf(0) }
    var confirmFresh by remember { mutableStateOf(false) }
    var stuck by remember { mutableStateOf<Stuck?>(null) }
    val first = remember { FocusRequester() }
    LaunchedEffect(Unit) { first.requestWhenReady() }
    // Buttons stay enabled while an attempt runs, so focus never falls off the one just pressed.
    fun attempt(fresh: Boolean) {
        if (busy) return
        busy = true
        scope.launch {
            try {
                onTryAgain(fresh)
            } finally {
                busy = false
                confirmFresh = false
                failedAttempts++
            }
        }
    }
    Box(
        Modifier
            .fillMaxSize()
            .background(colors.velvet)
            .padding(horizontal = Dimens.safeX, vertical = Dimens.safeY)
            .focusGroup(),
        contentAlignment = Alignment.CenterStart,
    ) {
        Column(Modifier.width(1100.dp), verticalArrangement = Arrangement.spacedBy(28.dp)) {
            // A heading and a live region: TalkBack says it as the screen comes up.
            Text(stringResource(R.string.recovery_title), style = type.displayL, color = colors.limelight, modifier = Modifier.semantics { heading(); liveRegion = LiveRegionMode.Polite })
            Text(
                stringResource(
                    when (reason) {
                        RecoveryReason.StorageFull -> R.string.recovery_storage_full
                        RecoveryReason.Storage -> R.string.recovery_storage
                        RecoveryReason.Other -> R.string.recovery_other
                    },
                ),
                style = type.body,
                color = colors.limelight,
            )
            Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                LoungeButton(label = stringResource(if (busy) R.string.recovery_opening else DesignR.string.common_try_again), onClick = { attempt(fresh = false) }, focusRequester = first)
                if (reason != RecoveryReason.StorageFull) {
                    // One button that changes, rather than two, so the confirmation keeps focus.
                    LoungeButton(
                        label = stringResource(if (confirmFresh) R.string.recovery_start_fresh_confirm else R.string.recovery_start_fresh),
                        onClick = { if (confirmFresh) attempt(fresh = true) else confirmFresh = true },
                        kind = if (confirmFresh) ButtonKind.Primary else ButtonKind.Secondary,
                        light = if (confirmFresh) colors.ember else colors.houselight,
                    )
                }
                LoungeButton(
                    label = stringResource(R.string.recovery_settings),
                    onClick = { stuck = if (onOpenSettings()) null else Stuck.Settings },
                    kind = ButtonKind.Secondary,
                )
            }
            if (otherHomes.isNotEmpty()) {
                Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                    for (home in otherHomes) {
                        LoungeButton(
                            label = stringResource(if (home.enabled) R.string.recovery_open_home else R.string.recovery_turn_home_on, home.label),
                            onClick = { stuck = if (onOtherHome(home)) null else Stuck.Home(home.label) },
                            kind = ButtonKind.Secondary,
                        )
                    }
                }
            }
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                if (failedAttempts > 0 && !busy) Text(stringResource(R.string.recovery_still_wont_open), style = type.meta, color = colors.dust)
                when (val s = stuck) {
                    Stuck.Settings -> Text(stringResource(R.string.recovery_settings_stuck), style = type.meta, color = colors.dust)
                    is Stuck.Home -> Text(stringResource(R.string.recovery_home_stuck, s.label), style = type.meta, color = colors.dust)
                    null -> Unit
                }
            }
        }
    }
}
