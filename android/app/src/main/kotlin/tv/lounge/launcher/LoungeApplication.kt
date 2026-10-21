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
import tv.lounge.design.GlowMasks
import tv.lounge.design.LoungeFonts
import tv.lounge.design.SpillMask
import kotlinx.coroutines.flow.first
import tv.lounge.platform.AndroidPlatform
import tv.lounge.platform.DeviceTierDetector
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
    @Volatile private var coreError: Throwable? = null
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
                coreError = t
            } finally {
                coreReady.countDown()
            }
        }, "lounge-open").start()
        Thread({
            startPrefs = kotlinx.coroutines.runBlocking { prefs.all.first() }
        }, "lounge-prefs").start()
        Thread({
            val t0 = android.os.SystemClock.uptimeMillis()
            LoungeFonts.warmUp(this)
            SpillMask.warmUp()
            GlowMasks.warmUp(resources.displayMetrics.widthPixels / 1920f)
            android.util.Log.i("Lounge", "startup: fonts and masks warm in ${android.os.SystemClock.uptimeMillis() - t0} ms")
        }, "lounge-warmup").start()
        updates.start(appScope)
        android.util.Log.i("Lounge", "startup: application onCreate took ${android.os.SystemClock.uptimeMillis() - startedAt} ms")
    }

    /** The core, opened at process start; blocks only if the activity got here first. */
    fun awaitCore(): CoreClient {
        val t0 = android.os.SystemClock.uptimeMillis()
        coreReady.await()
        val waited = android.os.SystemClock.uptimeMillis() - t0
        if (waited > 1) android.util.Log.i("Lounge", "startup: waited ${waited} ms for the core")
        return core ?: throw IllegalStateException("core failed to open", coreError)
    }

    private fun coreConfig() = CoreConfig(
        dataDir = filesDir.absolutePath,
        cacheDir = cacheDir.absolutePath,
        tier = DeviceTierDetector.detect(this),
        utcOffsetMinutes = java.util.TimeZone.getDefault().getOffset(System.currentTimeMillis()) / 60_000,
        hasTvControls = power.hasTvControls,
        deviceName = android.os.Build.MODEL,
    )

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
