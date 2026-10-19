package tv.lounge.collector.apps

import android.app.AppOpsManager
import android.app.usage.UsageStatsManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.content.pm.ResolveInfo
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.drawable.Drawable
import android.os.Build
import android.os.Process
import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.withContext
import org.json.JSONObject
import tv.lounge.core.AppRecord
import tv.lounge.core.AppUsage
import java.io.File
import java.io.FileOutputStream

/**
 * Reads installed TV apps, renders their banners and icons to PNG files in the cache when a package
 * changes, and reports usage recency. Ordering and visibility are the core's job.
 */
class AppsCollector(private val context: Context) {
    private val pm: PackageManager = context.packageManager
    private val bannerDir = File(context.cacheDir, "banners").apply { mkdirs() }
    private val indexFile = File(bannerDir, "index.json")

    /** Every launchable TV app, with banner and icon files ready for the core. */
    suspend fun snapshot(includeMobileApps: Boolean = false): List<AppRecord> = withContext(Dispatchers.IO) {
        val index = loadIndex()
        val leanback = pm.queryIntentActivities(
            Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LEANBACK_LAUNCHER),
            PackageManager.MATCH_ALL,
        )
        val seen = HashSet<String>()
        val records = ArrayList<AppRecord>(leanback.size)
        for (ri in leanback) {
            val pkg = ri.activityInfo.packageName
            if (pkg == context.packageName || !seen.add(pkg)) continue
            records += record(ri, index, isLeanback = true)
        }
        if (includeMobileApps) {
            val mobile = pm.queryIntentActivities(
                Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER),
                PackageManager.MATCH_ALL,
            )
            for (ri in mobile) {
                val pkg = ri.activityInfo.packageName
                if (pkg == context.packageName || !seen.add(pkg)) continue
                records += record(ri, index, isLeanback = false)
            }
        }
        saveIndex(index)
        pruneOrphans(records)
        records
    }

    /** Last-used time per package from UsageStats, or empty when the permission isn't granted. */
    fun usage(daysBack: Int = 30): List<AppUsage> {
        if (!hasUsageAccess()) return emptyList()
        val usm = context.getSystemService(Context.USAGE_STATS_SERVICE) as UsageStatsManager
        val end = System.currentTimeMillis()
        val start = end - daysBack * 86_400_000L
        val stats = usm.queryAndAggregateUsageStats(start, end)
        return stats.values
            .filter { it.lastTimeUsed > 0 }
            .map { AppUsage(it.packageName, it.lastTimeUsed) }
    }

    fun hasUsageAccess(): Boolean {
        val appOps = context.getSystemService(Context.APP_OPS_SERVICE) as AppOpsManager
        val mode = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            appOps.unsafeCheckOpNoThrow(AppOpsManager.OPSTR_GET_USAGE_STATS, Process.myUid(), context.packageName)
        } else {
            @Suppress("DEPRECATION")
            appOps.checkOpNoThrow(AppOpsManager.OPSTR_GET_USAGE_STATS, Process.myUid(), context.packageName)
        }
        return mode == AppOpsManager.MODE_ALLOWED
    }

    /** Emits whenever a package is added, removed or replaced. */
    fun packageChanges(): Flow<String> = callbackFlow {
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(c: Context, intent: Intent) {
                val pkg = intent.data?.schemeSpecificPart ?: return
                trySend(pkg)
            }
        }
        val filter = IntentFilter().apply {
            addAction(Intent.ACTION_PACKAGE_ADDED)
            addAction(Intent.ACTION_PACKAGE_REMOVED)
            addAction(Intent.ACTION_PACKAGE_REPLACED)
            addAction(Intent.ACTION_PACKAGE_CHANGED)
            addDataScheme("package")
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            context.registerReceiver(receiver, filter, Context.RECEIVER_EXPORTED)
        } else {
            @Suppress("UnspecifiedRegisterReceiverFlag")
            context.registerReceiver(receiver, filter)
        }
        awaitClose { context.unregisterReceiver(receiver) }
    }

    private fun record(ri: ResolveInfo, index: JSONObject, isLeanback: Boolean): AppRecord {
        val ai = ri.activityInfo
        val pkg = ai.packageName
        val appInfo = ai.applicationInfo
        val pkgInfo = runCatching { pm.getPackageInfo(pkg, 0) }.getOrNull()
        val lastUpdate = pkgInfo?.lastUpdateTime ?: 0L
        val firstInstall = pkgInfo?.firstInstallTime ?: 0L
        val label = ri.loadLabel(pm)?.toString()?.ifBlank { pkg } ?: pkg

        val bannerFile = File(bannerDir, "$pkg.banner.png")
        val iconFile = File(bannerDir, "$pkg.icon.png")
        val stamp = index.optLong(pkg, -1L)
        if (stamp != lastUpdate || !iconFile.exists()) {
            val banner = ri.activityInfo.loadBanner(pm) ?: appInfo.loadBanner(pm)
            if (banner != null) render(banner, 640, 360, bannerFile) else bannerFile.delete()
            val icon = ri.loadIcon(pm) ?: appInfo.loadIcon(pm)
            if (icon != null) render(icon, 256, 256, iconFile) else iconFile.delete()
            index.put(pkg, lastUpdate)
        }

        val isGame = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            appInfo.category == ApplicationInfo.CATEGORY_GAME || (appInfo.flags and ApplicationInfo.FLAG_IS_GAME) != 0
        } else {
            @Suppress("DEPRECATION")
            (appInfo.flags and ApplicationInfo.FLAG_IS_GAME) != 0
        }

        return AppRecord(
            `package` = pkg,
            activity = ai.name,
            label = label,
            bannerPath = bannerFile.takeIf { it.exists() }?.absolutePath,
            iconPath = iconFile.takeIf { it.exists() }?.absolutePath,
            lastUsedMs = null,
            installedMs = firstInstall,
            isGame = isGame,
            isLeanback = isLeanback,
        )
    }

    private fun render(drawable: Drawable, width: Int, height: Int, out: File) {
        try {
            val bmp = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
            val canvas = Canvas(bmp)
            drawable.setBounds(0, 0, width, height)
            drawable.draw(canvas)
            val tmp = File(out.path + ".tmp")
            FileOutputStream(tmp).use { bmp.compress(Bitmap.CompressFormat.PNG, 100, it) }
            if (!tmp.renameTo(out)) {
                tmp.copyTo(out, overwrite = true)
                tmp.delete()
            }
            bmp.recycle()
        } catch (e: Exception) {
            Log.w(TAG, "render ${out.name} failed", e)
            out.delete()
        }
    }

    private fun loadIndex(): JSONObject =
        runCatching { JSONObject(indexFile.readText()) }.getOrDefault(JSONObject())

    private fun saveIndex(index: JSONObject) {
        runCatching { indexFile.writeText(index.toString()) }.onFailure { Log.w(TAG, "save index", it) }
    }

    private fun pruneOrphans(records: List<AppRecord>) {
        val keep = records.flatMap { listOfNotNull(it.bannerPath, it.iconPath) }.map { File(it).name }.toHashSet()
        bannerDir.listFiles()?.forEach { f ->
            if (f.name != indexFile.name && f.name !in keep && !f.name.endsWith(".tmp")) f.delete()
        }
    }

    private companion object {
        const val TAG = "LoungeApps"
    }
}
