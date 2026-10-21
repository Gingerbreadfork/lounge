package tv.lounge.launcher

import android.content.Intent
import android.os.Bundle
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.addCallback
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.repeatOnLifecycle
import kotlinx.coroutines.FlowPreview
import kotlinx.coroutines.flow.conflate
import kotlinx.coroutines.flow.debounce
import tv.lounge.collector.inputs.InputsCollector
import tv.lounge.collector.media.MediaSessionsCollector
import tv.lounge.collector.tvprovider.TvProviderCollector
import tv.lounge.core.Permissions
import tv.lounge.platform.UiCommand
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import tv.lounge.design.LocalUiSounds
import tv.lounge.design.UiSounds
import androidx.core.view.WindowCompat
import androidx.lifecycle.lifecycleScope
import dagger.hilt.android.AndroidEntryPoint
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import tv.lounge.collector.apps.AppsCollector
import tv.lounge.core.CoreClient
import tv.lounge.design.LoungeTheme
import tv.lounge.design.LoungeThemeVariant
import tv.lounge.design.LoungeVisuals
import tv.lounge.design.systemReducedMotion
import tv.lounge.core.DeviceTier
import tv.lounge.feature.home.HomeScreen
import tv.lounge.platform.AndroidPlatform
import tv.lounge.platform.DeviceTierDetector
import tv.lounge.platform.Prefs
import java.util.TimeZone
import javax.inject.Inject

@OptIn(FlowPreview::class)
@AndroidEntryPoint
class MainActivity : ComponentActivity() {

    @Inject lateinit var core: CoreClient
    @Inject lateinit var power: tv.lounge.power.PowerSetup
    @Inject lateinit var updates: tv.lounge.update.UpdateManager
    @Inject lateinit var apps: AppsCollector
    @Inject lateinit var tvProvider: TvProviderCollector
    @Inject lateinit var media: MediaSessionsCollector
    @Inject lateinit var inputs: InputsCollector
    @Inject lateinit var platform: AndroidPlatform
    @Inject lateinit var prefs: Prefs
    private var uiSounds: UiSounds? = null
    private val soundsState = mutableStateOf<UiSounds?>(null)

    private val tvListingsRequest = registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        pushPermissions()
        if (granted) collectTvNow()
    }

    override fun onDestroy() {
        uiSounds?.release()
        uiSounds = null
        super.onDestroy()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        val t0 = android.os.SystemClock.uptimeMillis()
        activityStartedAt = t0
        super.onCreate(savedInstanceState)
        android.util.Log.i("Lounge", "startup: activity onCreate +${t0 - LoungeApplication.startedAt} ms after process start")
        window.decorView.post { android.util.Log.i("Lounge", "startup: first frame posted +${android.os.SystemClock.uptimeMillis() - t0} ms after activity start") }
        WindowCompat.setDecorFitsSystemWindows(window, false)
        window.addFlags(WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS)
        // A home screen never finishes on Back; the UI decides what Back means.
        onBackPressedDispatcher.addCallback(this) { }

        tv.lounge.feature.home.startupReference = { activityStartedAt }
        val tier = DeviceTierDetector.detect(this)
        setContent {
            val uiPrefs by prefs.all.collectAsState(initial = Prefs.UiPrefs())
            val reduced = uiPrefs.reducedMotion ?: systemReducedMotion()
            val visuals = when {
                uiPrefs.liteMode == true -> LoungeVisuals.Lite
                uiPrefs.liteMode == false -> if (tier == DeviceTier.A) LoungeVisuals.TierA else LoungeVisuals.TierB
                tier == DeviceTier.A -> LoungeVisuals.TierA
                tier == DeviceTier.B -> LoungeVisuals.TierB
                else -> LoungeVisuals.Lite
            }
            LoungeTheme(
                variant = LoungeThemeVariant.valueOf(uiPrefs.theme.name),
                reducedMotion = reduced,
                visuals = visuals,
            ) {
                CompositionLocalProvider(LocalUiSounds provides soundsState.value) {
                    HomeScreen(core = core, platform = platform, prefs = prefs, uiPrefs = uiPrefs, power = power, updates = updates)
                }
            }
        }

        android.util.Log.i("Lounge", "startup: setContent returned +${android.os.SystemClock.uptimeMillis() - t0} ms after activity start")

        // Sound decoding and re-indexing wait for the persisted snapshot to draw; the collectors
        // then start one at a time. A first run with nothing persisted starts them at once.
        afterFirstFrames(1) {
            uiSounds = UiSounds(this).also { soundsState.value = it }
        }
        val fresh = core.snapshots.value.rows.isEmpty()
        afterFirstFrames(2) {
            pushPermissions()
            core.ingestInputs(inputs.snapshot())
            if (fresh) {
                collectAppsNow()
                collectTvNow()
                installBundledPlugins()
            } else {
                lifecycleScope.launch {
                    delay(1_000)
                    collectAppsNow()
                    delay(1_500)
                    collectTvNow()
                    delay(1_000)
                    installBundledPlugins()
                }
            }
        }
        lifecycleScope.launch {
            repeatOnLifecycle(Lifecycle.State.STARTED) {
                inputs.changes().collect { core.ingestInputs(inputs.snapshot()) }
            }
        }
        lifecycleScope.launch {
            apps.packageChanges().collectLatest { collectAppsNow() }
        }
        // Collections run one at a time, and requests made while one runs fold into a single rerun.
        lifecycleScope.launch(Dispatchers.IO) {
            for (request in appCollects) {
                val includeMobile = prefs.showMobileApps.first()
                core.ingestApps(apps.snapshot(includeMobileApps = includeMobile))
                val usage = apps.usage()
                if (usage.isNotEmpty()) core.reportUsage(usage)
            }
        }
        lifecycleScope.launch(Dispatchers.IO) {
            for (request in tvCollects) core.ingestPrograms(tvProvider.snapshot())
        }
        lifecycleScope.launch {
            platform.commands.collect { cmd ->
                when (cmd) {
                    is UiCommand.RequestPermission -> requestAccess(cmd.name)
                    // Phone keys reach Lounge's own window while it is in front, and whatever app is
                    // in front through Power Setup; text goes to the focused field through the home screen.
                    is UiCommand.Key -> if (lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)) {
                        val now = android.os.SystemClock.uptimeMillis()
                        dispatchKeyEvent(android.view.KeyEvent(now, now, android.view.KeyEvent.ACTION_DOWN, cmd.keyCode, 0))
                        dispatchKeyEvent(android.view.KeyEvent(now, now, android.view.KeyEvent.ACTION_UP, cmd.keyCode, 0))
                    } else {
                        power.inputKey(cmd.keyCode)?.let { action ->
                            lifecycleScope.launch(Dispatchers.IO) { runCatching { power.run(action) }.onFailure { android.util.Log.w("Lounge", "key through Power Setup: ${it.message}") } }
                        }
                    }
                    UiCommand.InstallUpdate -> updates.interact(this@MainActivity, lifecycleScope)
                    else -> Unit
                }
            }
        }
        lifecycleScope.launch {
            updates.confirmRequests.collect { intent -> runCatching { startActivity(intent) } }
        }
        lifecycleScope.launch {
            val on = prefs.companionEnabled.first()
            android.util.Log.i("Lounge", "companion: enabled=$on at start")
            if (on) {
                core.startCompanion(COMPANION_PORT)
                    .onSuccess { android.util.Log.i("Lounge", "companion: ${it.url}") }
                    .onFailure { android.util.Log.w("Lounge", "companion: ${it.message}") }
            }
        }
        lifecycleScope.launch {
            repeatOnLifecycle(Lifecycle.State.STARTED) {
                // Players report position and buffering constantly; the home only needs real changes.
                media.sessions().conflate().debounce(500).collect { core.ingestSessions(it) }
            }
        }
        lifecycleScope.launch {
            repeatOnLifecycle(Lifecycle.State.STARTED) {
                tvProvider.changes().debounce(1500).collect { collectTvNow() }
            }
        }
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus) power.noteWindowFocused()
    }

    /** Home pressed while Lounge is in front, or on the way back to it: close what is open and show the Stage. */
    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        if (intent.action == Intent.ACTION_MAIN) platform.goHome()
    }

    private fun afterFirstFrames(frames: Int, block: () -> Unit) {
        val choreographer = android.view.Choreographer.getInstance()
        var left = frames
        lateinit var callback: android.view.Choreographer.FrameCallback
        callback = android.view.Choreographer.FrameCallback {
            left--
            if (left <= 0) block() else choreographer.postFrameCallback(callback)
        }
        choreographer.postFrameCallback(callback)
    }

    private fun requestAccess(name: String) {
        when (name) {
            "tv_listings" -> tvListingsRequest.launch("android.permission.READ_TV_LISTINGS")
            "notifications" -> openSettings(
                android.provider.Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS,
                "Allow notification access for Lounge in Settings, then come back.",
            )
            "usage" -> openSettings(android.provider.Settings.ACTION_USAGE_ACCESS_SETTINGS, "Allow usage access for Lounge in Settings.")
        }
    }

    private fun openSettings(action: String, fallbackHint: String) {
        val intent = android.content.Intent(action).addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK)
        if (intent.resolveActivity(packageManager) != null) {
            runCatching { startActivity(intent) }.onFailure { android.widget.Toast.makeText(this, fallbackHint, android.widget.Toast.LENGTH_LONG).show() }
        } else {
            android.widget.Toast.makeText(this, fallbackHint, android.widget.Toast.LENGTH_LONG).show()
        }
    }

    /** The first-party plugins ship in the APK; (re)install them once per APK build. */
    private fun installBundledPlugins() {
        lifecycleScope.launch(Dispatchers.IO) {
            val stamp = runCatching { packageManager.getPackageInfo(packageName, 0).lastUpdateTime }.getOrDefault(0L)
            if (prefs.bundledPluginsStamp.first() == stamp) return@launch
            val names = runCatching { assets.list("plugins")?.toList().orEmpty() }.getOrDefault(emptyList())
            val dir = java.io.File(cacheDir, "bundled-plugins").apply { mkdirs() }
            var ok = true
            for (name in names.filter { it.endsWith(".lgp") }) {
                val out = java.io.File(dir, name)
                runCatching { assets.open("plugins/$name").use { input -> out.outputStream().use { input.copyTo(it) } } }
                    .onFailure { ok = false; android.util.Log.w("Lounge", "copy $name", it) }
                core.installPlugin(out.absolutePath, builtIn = true)
                    .onFailure { ok = false; android.util.Log.w("Lounge", "install $name: ${it.message}") }
            }
            if (ok) prefs.setBundledPluginsStamp(stamp)
        }
    }

    private fun pushPermissions() {
        core.setPermissions(Permissions(tvListings = tvProvider.hasPermission(), notifications = media.isEnabled(), usage = apps.hasUsageAccess()))
    }

    private var lastTvCollectMs = 0L
    private val tvCollects = Channel<Unit>(Channel.CONFLATED)
    private val appCollects = Channel<Unit>(Channel.CONFLATED)

    private fun collectTvNow() {
        if (!tvProvider.hasPermission()) return
        lastTvCollectMs = System.currentTimeMillis()
        tvCollects.trySend(Unit)
    }

    private var resumedBefore = false

    companion object {
        /** When the current activity instance started; the home screen logs its first composition against it. */
        @Volatile var activityStartedAt: Long = 0L
        const val COMPANION_PORT = 8787
    }

    override fun onResume() {
        super.onResume()
        core.setScreenState(true)
        updates.noteForeground(true)
        core.setUtcOffsetMinutes(TimeZone.getDefault().getOffset(System.currentTimeMillis()) / 60_000)
        if (resumedBefore) {
            pushPermissions()
            // The content observer already refreshes on change; a quick hop out and back is not one.
            if (System.currentTimeMillis() - lastTvCollectMs > 5 * 60_000L) collectTvNow()
        }
        resumedBefore = true
        lifecycleScope.launch(Dispatchers.IO) {
            val usage = apps.usage()
            if (usage.isNotEmpty()) core.reportUsage(usage)
        }
    }

    override fun onStop() {
        super.onStop()
        core.setScreenState(false)
        updates.noteForeground(false)
    }

    private fun collectAppsNow() {
        appCollects.trySend(Unit)
    }
}
