package tv.lounge.feature.home

import android.app.AppOpsManager
import android.content.Context
import android.os.Process
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.focusGroup
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.tv.material3.Icon
import androidx.tv.material3.Text
import coil3.compose.AsyncImage
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import tv.lounge.core.CoreClient
import tv.lounge.core.HomeSnapshot
import tv.lounge.core.PairingInfo
import tv.lounge.design.ButtonKind
import tv.lounge.design.Dimens
import tv.lounge.design.LoungeButton
import tv.lounge.design.LoungeIcons
import tv.lounge.design.LoungeTheme
import tv.lounge.design.SpillMask
import tv.lounge.design.drawSpill
import tv.lounge.design.lightGlow
import tv.lounge.platform.Prefs
import tv.lounge.power.PowerAction
import tv.lounge.power.PowerSetup
import tv.lounge.power.PowerStatus
import tv.lounge.power.PrivilegedException

private const val STEPS = 5

/**
 * First run: five full-screen steps, one question each, a heading and one primary button; progress
 * is a row of dots (DESIGN.md §5). Back goes to the previous step and leaves from the first. Focus
 * lands on the thing to decide.
 */
@Composable
fun OnboardingScreen(
    core: CoreClient,
    prefs: Prefs,
    uiPrefs: Prefs.UiPrefs,
    power: PowerSetup,
    visibleState: MutableTransitionState<Boolean>,
    onDone: () -> Unit,
) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    val context = LocalContext.current
    val snapshot by core.snapshots.collectAsStateWithLifecycle()
    var step by remember { mutableIntStateOf(0) }
    val primaryFocus = remember { FocusRequester() }
    val choiceFocus = remember { FocusRequester() }
    var previewTheme by remember { mutableStateOf(uiPrefs.theme) }
    var tick by remember { mutableIntStateOf(0) }
    val leaving = !visibleState.targetState
    val rise = with(androidx.compose.ui.platform.LocalDensity.current) { 10.dp.roundToPx() }

    // Permissions are re-read when the system screens hand back, not on a timer.
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) { tick++ }
    LaunchedEffect(step) {
        delay(80)
        if (step == 0) choiceFocus.requestWhenReady() else primaryFocus.requestWhenReady()
    }

    fun next() {
        if (step < STEPS - 1) step++ else onDone()
    }

    AnimatedVisibility(visibleState, enter = fadeIn(motion.curtain()), exit = fadeOut(motion.settle())) {
        Box(
            Modifier
                .fillMaxSize()
                .background(colors.velvet)
                .onPreviewKeyEvent { e ->
                    if (e.isBack() && e.type == KeyEventType.KeyUp) {
                        if (step > 0) step-- else onDone()
                        true
                    } else {
                        false
                    }
                }
                .focusProperties { canFocus = !leaving }
                .trapFocus(leaving)
                .focusGroup(),
        ) {
            Row(Modifier.fillMaxSize().padding(horizontal = Dimens.safeX, vertical = Dimens.safeY)) {
                Column(Modifier.width(820.dp).fillMaxHeight(), verticalArrangement = Arrangement.SpaceBetween) {
                    Column {
                        Spacer(Modifier.height(72.dp))
                        AnimatedContent(
                            targetState = step,
                            transitionSpec = {
                                val forward = targetState > initialState
                                (fadeIn(motion.curtain()) + slideInVertically(motion.curtain()) { if (forward) rise else -rise })
                                    .togetherWith(fadeOut(motion.settle()))
                            },
                            label = "onboardingStep",
                        ) { s ->
                            Column {
                                when (s) {
                                    0 -> StepLook(prefs, uiPrefs, choiceFocus, onPreview = { previewTheme = it })
                                    1 -> StepAccess(core, context, tick)
                                    2 -> StepPower(power, context)
                                    3 -> StepPhone(core, prefs, uiPrefs)
                                    else -> StepDone()
                                }
                            }
                        }
                    }
                    Column {
                        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                            LoungeButton(
                                label = when (step) {
                                    STEPS - 1 -> "Start"
                                    else -> "Next"
                                },
                                onClick = { next() },
                                focusRequester = primaryFocus,
                            )
                            if (step in 1 until STEPS - 1) {
                                LoungeButton(label = "Skip", onClick = { next() }, kind = ButtonKind.Secondary)
                            }
                        }
                        Spacer(Modifier.height(28.dp))
                        Row(horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
                            repeat(STEPS) { i ->
                                val width = animateDpAsState(if (i == step) 24.dp else 8.dp, motion.settle(), label = "stepDot")
                                Box(
                                    Modifier
                                        .width(width.value)
                                        .height(8.dp)
                                        .clip(RoundedCornerShape(4.dp))
                                        .background(if (i == step) colors.houselight else colors.curtainEdge),
                                )
                            }
                        }
                    }
                }
                Spacer(Modifier.width(64.dp))
                Box(Modifier.weight(1f).fillMaxHeight(), contentAlignment = Alignment.Center) {
                    AnimatedContent(
                        targetState = step,
                        transitionSpec = { fadeIn(motion.curtain()).togetherWith(fadeOut(motion.settle())) },
                        label = "onboardingArt",
                    ) { s ->
                        when (s) {
                            0 -> ThemePreview(previewTheme, snapshot)
                            3 -> PhoneArt(core, uiPrefs)
                            else -> Lamp(
                                when (s) {
                                    1 -> LoungeIcons.Play
                                    2 -> LoungeIcons.Power
                                    else -> LoungeIcons.Search
                                },
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun Heading(title: String, body: String) {
    val colors = LoungeTheme.colors
    Text(title, style = LoungeTheme.typography.displayL, color = colors.limelight)
    Spacer(Modifier.height(18.dp))
    Text(body, style = LoungeTheme.typography.meta, color = colors.dust)
    Spacer(Modifier.height(36.dp))
}

/** A 12 px houselight dot when chosen, an empty dust ring otherwise. */
@Composable
private fun ChoiceDot(selected: Boolean) {
    val colors = LoungeTheme.colors
    Box(
        Modifier
            .size(12.dp)
            .clip(CircleShape)
            .then(if (selected) Modifier.background(colors.houselight) else Modifier.border(1.5.dp, colors.dust, CircleShape)),
    )
}

@Composable
private fun StepLook(prefs: Prefs, uiPrefs: Prefs.UiPrefs, choiceFocus: FocusRequester, onPreview: (Prefs.Theme) -> Unit) {
    val scope = rememberCoroutineScope()
    Heading("Welcome to Lounge", "Your TV, laid out around what you actually watch. First, how should it look? You can change this any time.")
    // Ten rooms, two to a row, so the grid and the live preview share the screen.
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Prefs.Theme.entries.chunked(2).forEach { pair ->
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                pair.forEach { theme ->
                    val selected = uiPrefs.theme == theme
                    SheetRow(
                        icon = null,
                        title = theme.title,
                        subtitle = theme.tagline,
                        height = 72.dp,
                        onClick = { scope.launch { prefs.setTheme(theme) } },
                        focusRequester = if (selected) choiceFocus else null,
                        leading = { ChoiceDot(selected) },
                        modifier = Modifier.weight(1f).onFocusChanged { if (it.isFocused) onPreview(theme) },
                    )
                }
            }
        }
    }
}

/**
 * The right-hand panel on the first step: a 560 × 315 miniature of the home screen in the focused
 * theme, crossfading over `curtain` as focus moves.
 */
@Composable
private fun ThemePreview(theme: Prefs.Theme, snapshot: HomeSnapshot) {
    val motion = LoungeTheme.motion
    val palette = theme.palette()
    val velvet = androidx.compose.animation.animateColorAsState(palette.velvet, motion.curtain(), label = "previewVelvet")
    val curtain = androidx.compose.animation.animateColorAsState(palette.curtain, motion.curtain(), label = "previewCurtain")
    val edge = androidx.compose.animation.animateColorAsState(palette.curtainEdge, motion.curtain(), label = "previewEdge")
    val art = snapshot.stage.spotlights.mapNotNull { it.artworkPath }
    Box(
        Modifier
            .size(560.dp, 315.dp)
            .clip(RoundedCornerShape(20.dp))
            .drawBehind {
                drawRect(velvet.value)
                drawSpill(SpillMask.soft, Offset(size.width * 0.3f, size.height * 0.48f), size.width * 0.42f, size.height * 0.5f, palette.houselight, 0.22f * palette.spillScale)
            }
            .border(1.dp, edge.value, RoundedCornerShape(20.dp)),
    ) {
        Text("9:41", style = LoungeTheme.typography.title, color = palette.limelight, modifier = Modifier.align(Alignment.TopEnd).padding(top = 20.dp, end = 28.dp))
        Box(
            Modifier
                .offset(32.dp, 56.dp)
                .size(300.dp, 170.dp)
                .lightGlow(palette.houselight, 0.26f * palette.spillScale, cornerRadius = 10.dp)
                .clip(RoundedCornerShape(10.dp))
                .background(curtain.value),
        ) {
            art.firstOrNull()?.let { AsyncImage(model = it, contentDescription = null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize()) }
        }
        Column(Modifier.offset(352.dp, 110.dp)) {
            Box(Modifier.width(140.dp).height(10.dp).clip(RoundedCornerShape(5.dp)).background(palette.limelight))
            Spacer(Modifier.height(10.dp))
            Box(Modifier.width(96.dp).height(8.dp).clip(RoundedCornerShape(4.dp)).background(palette.dust))
            Spacer(Modifier.height(18.dp))
            Box(Modifier.width(72.dp).height(20.dp).clip(RoundedCornerShape(10.dp)).background(palette.houselight))
        }
        Row(Modifier.offset(32.dp, 246.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            repeat(5) { i ->
                Box(Modifier.size(80.dp, 45.dp).clip(RoundedCornerShape(5.dp)).background(curtain.value)) {
                    art.getOrNull(i + 1)?.let { AsyncImage(model = it, contentDescription = null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize()) }
                }
            }
        }
    }
}

@Composable
private fun StepAccess(core: CoreClient, context: Context, tick: Int) {
    val colors = LoungeTheme.colors
    val tv = remember(tick) { hasTvListings(context) }
    val notifications = remember(tick) { hasNotificationAccess(context) }
    val usage = remember(tick) { hasUsageAccess(context) }
    Heading("What may Lounge see?", "Each one adds a part of the home screen. Nothing leaves this device; say no to any of them.")
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        AccessRow(LoungeIcons.Play, "TV listings", "Continue watching, channels and the guide", tv) { core.perform("permission:tv_listings") }
        AccessRow(LoungeIcons.Equaliser, "Now playing", "What is playing, with play and pause", notifications) { core.perform("permission:notifications") }
        AccessRow(LoungeIcons.Apps, "Usage access", "Your apps sorted by what you open", usage) { core.perform("permission:usage") }
    }
    Spacer(Modifier.height(20.dp))
    Text("Now playing and usage access open a system screen; press Back to come back here.", style = LoungeTheme.typography.caption, color = colors.dust)
}

@Composable
private fun AccessRow(icon: ImageVector, title: String, detail: String, granted: Boolean, onAllow: () -> Unit) {
    val colors = LoungeTheme.colors
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        SheetRow(icon = icon, title = title, value = if (granted) "On" else "Allow", accent = if (granted) colors.signal else colors.houselight, onClick = { if (!granted) onAllow() })
        Text(detail, style = LoungeTheme.typography.caption, color = colors.dust, modifier = Modifier.padding(start = 20.dp))
    }
}

@Composable
private fun StepPower(power: PowerSetup, context: Context) {
    val colors = LoungeTheme.colors
    val scope = rememberCoroutineScope()
    val status by power.status.collectAsStateWithLifecycle()
    var pairing by remember { mutableStateOf(false) }
    var homeState by remember { mutableStateOf<String?>(null) }
    var isHome by remember { mutableStateOf(isDefaultHome(context)) }
    val ready = status is PowerStatus.Ready
    val takeHome by produceState(emptyList<PowerAction>(), isHome) { value = withContext(Dispatchers.IO) { power.takeHome() } }
    val turnOff = takeHome.filter { it.id.startsWith("home_off_") }

    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) {
        isHome = isDefaultHome(context)
        scope.launch { power.refresh() }
    }
    // The approval prompt is a system dialog over this screen, so poll while it is up.
    LaunchedEffect(status) {
        if (status == PowerStatus.WaitingForApproval) {
            while (true) {
                delay(3_000)
                if (!pairing) power.refresh()
            }
        }
    }

    Heading(
        "Make it your home screen",
        if (isHome) "Done: pressing Home brings Lounge up. Power Setup can also grant the accesses from the last step and control the TV over HDMI-CEC."
        else "Google TV has no setting for this, so Lounge asks this device's own debugging service once. You will see a prompt; tick “Always allow” and choose Allow.",
    )
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        SheetRow(
            icon = LoungeIcons.Power,
            title = when (status) {
                is PowerStatus.Ready -> "Connected"
                PowerStatus.WaitingForApproval -> "Waiting for you to allow it…"
                is PowerStatus.Unavailable -> "Turn on USB debugging first"
                else -> "Connect Power Setup"
            },
            value = when (status) {
                is PowerStatus.Ready -> "This device"
                PowerStatus.WaitingForApproval -> "Look at the prompt"
                is PowerStatus.Unavailable -> "Open Developer options"
                else -> if (pairing) "Connecting…" else "Asks once"
            },
            accent = if (ready) colors.signal else colors.houselight,
            onClick = {
                when (status) {
                    is PowerStatus.Ready, PowerStatus.WaitingForApproval -> Unit
                    is PowerStatus.Unavailable -> runCatching {
                        context.startActivity(android.content.Intent(android.provider.Settings.ACTION_APPLICATION_DEVELOPMENT_SETTINGS).addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK))
                    }
                    else -> if (!pairing) {
                        pairing = true
                        scope.launch { power.pair(); pairing = false }
                    }
                }
            },
        )
        SheetRow(
            icon = LoungeIcons.Power,
            title = "Make Lounge the home screen",
            value = when {
                isHome -> "Done"
                homeState != null -> homeState!!
                ready -> "Do it"
                else -> "Connect first"
            },
            accent = if (isHome) colors.signal else if (ready) colors.houselight else colors.dust,
            onClick = {
                if (ready && !isHome && homeState != "Working…") {
                    homeState = "Working…"
                    scope.launch {
                        homeState = try {
                            val r = power.runAll(takeHome.ifEmpty { listOf(power.homeRole) })
                            isHome = isDefaultHome(context)
                            if (r.ok || isHome) null else r.output.lineSequence().firstOrNull()?.take(28) ?: "Failed"
                        } catch (e: PrivilegedException) {
                            e.message?.take(28) ?: "Failed"
                        }
                    }
                }
            },
        )
    }
    Spacer(Modifier.height(20.dp))
    Text(
        if (!isHome && turnOff.isNotEmpty()) {
            val them = if (turnOff.size == 1) "it" else "them"
            "This TV's own home comes before any chosen one, so Lounge turns $them off too: " +
                turnOff.joinToString("; ") { it.command } +
                ". Nothing is uninstalled; turn $them back on from Settings → Power Setup, and do that before you uninstall Lounge."
        } else {
            "The stock launcher stays installed, so casting, the gear button and the programme guide keep working. Undo from Settings → Power Setup."
        },
        style = LoungeTheme.typography.caption,
        color = colors.dust,
    )
}

@Composable
private fun StepPhone(core: CoreClient, prefs: Prefs, uiPrefs: Prefs.UiPrefs) {
    val colors = LoungeTheme.colors
    val scope = rememberCoroutineScope()
    Heading("Add your phone", "Scan the code with your phone's camera. It becomes a remote and a keyboard, and the place to arrange rows, add sources and install plugins.")
    SheetRow(
        icon = LoungeIcons.Phone,
        title = "Phone companion",
        value = if (uiPrefs.companionEnabled) "On" else "Turn on",
        accent = if (uiPrefs.companionEnabled) colors.signal else colors.houselight,
        onClick = {
            val on = !uiPrefs.companionEnabled
            scope.launch {
                prefs.setCompanionEnabled(on)
                // Off stops the server too, as Settings does.
                if (!on) core.stopCompanion()
            }
        },
    )
    Spacer(Modifier.height(20.dp))
    Text("Everything stays on your home network. Forget a phone any time from Settings → Phone companion.", style = LoungeTheme.typography.caption, color = colors.dust)
}

@Composable
private fun StepDone() {
    val colors = LoungeTheme.colors
    Heading("That's the house", "Press Home any time to come back to the Stage. Long-press a tile for its menu, Settings has the essentials, and the rest is on your phone.")
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Text("Up from the Stage reaches Search, the guide and Settings.", style = LoungeTheme.typography.meta, color = colors.dust)
        Text("Back always goes up one level; from the rows it returns to the Stage.", style = LoungeTheme.typography.meta, color = colors.dust)
    }
}

/**
 * The right-hand panel: a lamp in houselight with the step's glyph in it. The light reaches zero
 * inside the box.
 */
@Composable
private fun Lamp(icon: ImageVector) {
    val colors = LoungeTheme.colors
    Box(
        Modifier
            .size(560.dp)
            .drawBehind {
                drawSpill(SpillMask.soft, Offset(size.width / 2f, size.height / 2f), size.width / 2f, size.height / 2f, colors.houselight, 0.32f * colors.spillScale)
            },
        contentAlignment = Alignment.Center,
    ) {
        Icon(icon, contentDescription = null, tint = colors.limelight, modifier = Modifier.size(168.dp))
    }
}

/** The pairing code as the right-hand panel while the companion is on. */
@Composable
private fun PhoneArt(core: CoreClient, uiPrefs: Prefs.UiPrefs) {
    val colors = LoungeTheme.colors
    var pairing by remember { mutableStateOf<PairingInfo?>(null) }
    var url by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(uiPrefs.companionEnabled) {
        if (!uiPrefs.companionEnabled) { pairing = null; return@LaunchedEffect }
        if (!core.companionStatus().running) core.startCompanion(CoreClient.COMPANION_PORT)
        url = core.companionStatus().url
        while (true) {
            val p = pairing
            if (p == null || p.expiresMs < System.currentTimeMillis()) core.companionPairing().onSuccess { pairing = it }
            delay(5_000)
        }
    }
    val p = pairing
    if (!uiPrefs.companionEnabled || p == null) {
        Lamp(LoungeIcons.Phone)
    } else {
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            Box(Modifier.size(320.dp).clip(RoundedCornerShape(20.dp)).background(colors.limelight)) {
                AsyncImage(model = p.qrPngPath, contentDescription = "Pairing code", modifier = Modifier.fillMaxSize())
            }
            Spacer(Modifier.height(24.dp))
            Text("Or type this code", style = LoungeTheme.typography.meta, color = colors.dust)
            Spacer(Modifier.height(6.dp))
            Text(p.code.chunked(3).joinToString(" "), style = LoungeTheme.typography.displayL, color = colors.limelight)
            Spacer(Modifier.height(8.dp))
            Text(url?.let { "at $it" } ?: "", style = LoungeTheme.typography.caption, color = colors.dust)
        }
    }
}

internal fun hasUsageAccess(context: Context): Boolean {
    val appOps = context.getSystemService(AppOpsManager::class.java) ?: return false
    // unsafeCheckOpNoThrow arrived in Android 10; Android 9 has only the older name.
    val mode = runCatching {
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.Q) {
            appOps.unsafeCheckOpNoThrow(AppOpsManager.OPSTR_GET_USAGE_STATS, Process.myUid(), context.packageName)
        } else {
            @Suppress("DEPRECATION")
            appOps.checkOpNoThrow(AppOpsManager.OPSTR_GET_USAGE_STATS, Process.myUid(), context.packageName)
        }
    }.getOrDefault(AppOpsManager.MODE_ERRORED)
    return mode == AppOpsManager.MODE_ALLOWED
}
