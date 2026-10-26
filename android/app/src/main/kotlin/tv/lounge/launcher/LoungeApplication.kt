package tv.lounge.launcher

import android.app.Application
import coil3.ImageLoader
import coil3.SingletonImageLoader
import coil3.memory.MemoryCache
import coil3.request.allowHardware
import coil3.request.crossfade
import dagger.hilt.android.HiltAndroidApp
import tv.lounge.core.Core
import tv.lounge.core.CoreClient
import tv.lounge.core.CoreConfig
import tv.lounge.core.DeviceTier
import tv.lounge.core.setStorageAside
import tv.lounge.design.GlowMasks
import tv.lounge.design.LoungeFonts
import tv.lounge.design.SpillMask
import kotlinx.coroutines.flow.first
import tv.lounge.platform.AndroidPlatform
import tv.lounge.platform.DeviceTierDetector
import tv.lounge.platform.PanelSize
import tv.lounge.platform.Prefs

@HiltAndroidApp
class LoungeApplication : Application(), SingletonImageLoader.Factory {

    lateinit var platform: AndroidPlatform
        private set
    private val coreReady = java.util.concurrent.CountDownLatch(1)
    val power: tv.lounge.power.PowerSetup by lazy { tv.lounge.power.PowerSetup(this) }
    private val appScope = kotlinx.coroutines.CoroutineScope(kotlinx.coroutines.SupervisorJob() + kotlinx.coroutines.Dispatchers.Default)
    val updates: tv.lounge.update.UpdateManager by lazy {
        val version = runCatching { packageManager.getPackageInfo(packageName, 0).versionName }.getOrNull() ?: "0.0.0"
        tv.lounge.update.UpdateManager(this, power, version)
    }
    @Volatile private var core: CoreClient? = null
    /** Why the core did not open, while it has not. */
    @Volatile var coreError: Throwable? = null
        private set
    val prefs: Prefs by lazy { Prefs(this) }
    /** The preferences as they were at process start, once read; the first frame is drawn in that theme. */
    @Volatile var startPrefs: Prefs.UiPrefs? = null
        private set

    override fun onCreate() {
        startedAt = android.os.SystemClock.uptimeMillis()
        super.onCreate()
        platform = AndroidPlatform(this)
        // Open the core and warm the fonts on their own threads now, so the first composition finds
        // both ready.
        Thread({
            try {
                core = CoreClient(Core.open(coreConfig(), platform))
            } catch (t: Throwable) {
                android.util.Log.e("Lounge", "core failed to open", t)
                coreError = t
            } finally {
                coreReady.countDown()
            }
        }, "lounge-open").start()
        // Neither thread is needed to start, so a failure in one is logged rather than ending the process.
        Thread({
            startPrefs = runCatching { kotlinx.coroutines.runBlocking { prefs.all.first() } }
                .onFailure { android.util.Log.w("Lounge", "startup: preferences unreadable", it) }
                .getOrNull()
        }, "lounge-prefs").start()
        Thread({
            runCatching {
                val t0 = android.os.SystemClock.uptimeMillis()
                LoungeFonts.warmUp(this)
                SpillMask.warmUp()
                GlowMasks.warmUp(resources.displayMetrics.widthPixels / 1920f)
                android.util.Log.i("Lounge", "startup: fonts and masks warm in ${android.os.SystemClock.uptimeMillis() - t0} ms")
            }.onFailure { android.util.Log.w("Lounge", "startup: warm-up failed", it) }
        }, "lounge-warmup").start()
        updates.start(appScope)
        android.util.Log.i("Lounge", "startup: application onCreate took ${android.os.SystemClock.uptimeMillis() - startedAt} ms")
    }

    /** The core, opened at process start, or null if it failed to open; blocks only if the caller got here first. */
    fun awaitCoreOrNull(): CoreClient? {
        val t0 = android.os.SystemClock.uptimeMillis()
        coreReady.await()
        val waited = android.os.SystemClock.uptimeMillis() - t0
        if (waited > 1) android.util.Log.i("Lounge", "startup: waited ${waited} ms for the core")
        return core
    }

    /**
     * Opens the core again after it failed to, first setting its database aside when [fresh].
     * Returns the error if it still fails. Blocks, so call it off the main thread.
     */
    @Synchronized
    fun reopenCore(fresh: Boolean): Throwable? {
        if (core != null) return null
        return try {
            // With no file to move, opening again is still worth trying.
            if (fresh) runCatching { setStorageAside(filesDir.absolutePath) }.onFailure { android.util.Log.w("Lounge", "set the database aside", it) }
            core = CoreClient(Core.open(coreConfig(), platform))
            coreError = null
            null
        } catch (t: Throwable) {
            android.util.Log.e("Lounge", "core failed to open again", t)
            coreError = t
            t
        }
    }

    private fun coreConfig(): CoreConfig {
        val panel = PanelSize.of(this)
        return CoreConfig(
            dataDir = filesDir.absolutePath,
            cacheDir = cacheDir.absolutePath,
            tier = DeviceTierDetector.detect(this),
            utcOffsetMinutes = java.util.TimeZone.getDefault().getOffset(System.currentTimeMillis()) / 60_000,
            hasTvControls = power.hasTvControls,
            deviceName = android.os.Build.MODEL,
            screenWidth = panel.width.coerceAtLeast(0).toUInt(),
            screenHeight = panel.height.coerceAtLeast(0).toUInt(),
        )
    }

    companion object {
        @Volatile var startedAt: Long = android.os.SystemClock.uptimeMillis()
    }

    override fun newImageLoader(context: coil3.PlatformContext): ImageLoader {
        // Files on disk are already pre-scaled; the cache only needs what is on screen plus the row
        // being prefetched. Hardware bitmaps live in graphics memory, not the Java heap.
        val budgetBytes = when (DeviceTierDetector.detect(this)) {
            DeviceTier.A -> 40L
            DeviceTier.B -> 28L
            DeviceTier.C -> 14L
        } * 1024 * 1024
        // Every image is a local file the core already wrote, so there is nothing to disk-cache.
        return ImageLoader.Builder(context)
            .memoryCache { MemoryCache.Builder().maxSizeBytes(budgetBytes).build() }
            .diskCache(null)
            .allowHardware(true)
            .crossfade(180)
            .build()
    }
}
