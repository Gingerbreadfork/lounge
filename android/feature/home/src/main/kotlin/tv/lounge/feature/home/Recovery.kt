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
import androidx.compose.ui.unit.dp
import androidx.tv.material3.Text
import kotlinx.coroutines.launch
import tv.lounge.design.ButtonKind
import tv.lounge.design.Dimens
import tv.lounge.design.LoungeButton
import tv.lounge.design.LoungeTheme

/** Why the home could not start. */
enum class RecoveryReason { Storage, StorageFull, Other }

/** Another home screen on the TV, such as the stock launcher Lounge replaced. */
data class OtherHome(val packageName: String, val label: String, val enabled: Boolean)

/**
 * Shown in place of the home when the core will not open, so the TV is never left without a way
 * on. [onTryAgain] returns when the attempt is over; if it worked, the caller shows the home.
 */
@Composable
fun RecoveryScreen(
    reason: RecoveryReason,
    detail: String?,
    otherHomes: List<OtherHome>,
    onTryAgain: suspend (fresh: Boolean) -> Unit,
    onOpenSettings: () -> Unit,
    onOtherHome: (OtherHome) -> Unit,
) {
    val colors = LoungeTheme.colors
    val type = LoungeTheme.typography
    val scope = rememberCoroutineScope()
    var busy by remember { mutableStateOf(false) }
    var failedAttempts by remember { mutableIntStateOf(0) }
    var confirmFresh by remember { mutableStateOf(false) }
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
            Text("Lounge couldn't start", style = type.displayL, color = colors.limelight)
            Text(
                when (reason) {
                    RecoveryReason.StorageFull ->
                        "The TV's storage is full, so Lounge can't open its saved layout and settings. Free some space in Settings, then try again."
                    RecoveryReason.Storage ->
                        "Its saved layout and settings could not be opened. Try again first. If it keeps happening, start fresh: they are set aside, Lounge begins again, and your apps, channels and inputs come back on their own."
                    RecoveryReason.Other ->
                        "Lounge's core did not load. Try again first. If it keeps happening, start fresh, or reinstall Lounge."
                },
                style = type.body,
                color = colors.limelight,
            )
            Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                LoungeButton(label = if (busy) "Opening…" else "Try again", onClick = { attempt(fresh = false) }, focusRequester = first)
                if (reason != RecoveryReason.StorageFull) {
                    // One button that changes, rather than two, so the confirmation keeps focus.
                    LoungeButton(
                        label = if (confirmFresh) "Yes, start fresh" else "Start fresh",
                        onClick = { if (confirmFresh) attempt(fresh = true) else confirmFresh = true },
                        kind = if (confirmFresh) ButtonKind.Primary else ButtonKind.Secondary,
                        light = if (confirmFresh) colors.ember else colors.houselight,
                    )
                }
                LoungeButton(label = "Settings", onClick = onOpenSettings, kind = ButtonKind.Secondary)
            }
            if (otherHomes.isNotEmpty()) {
                Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                    for (home in otherHomes) {
                        LoungeButton(
                            label = if (home.enabled) "Open ${home.label}" else "Turn ${home.label} back on",
                            onClick = { onOtherHome(home) },
                            kind = ButtonKind.Secondary,
                        )
                    }
                }
            }
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                if (failedAttempts > 0 && !busy) Text("It still won't open.", style = type.meta, color = colors.dust)
                if (detail != null) Text(detail, style = type.caption, color = colors.dust, maxLines = 3)
            }
        }
    }
}
