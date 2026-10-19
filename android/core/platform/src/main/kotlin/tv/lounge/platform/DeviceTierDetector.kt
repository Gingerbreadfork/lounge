package tv.lounge.platform

import android.app.ActivityManager
import android.content.Context
import tv.lounge.core.DeviceTier

/** RAM-based tiers: 3 GB and up is A, 1.8 GB and up is B, below that is C. */
object DeviceTierDetector {
    fun detect(context: Context): DeviceTier {
        val am = context.getSystemService(Context.ACTIVITY_SERVICE) as ActivityManager
        val info = ActivityManager.MemoryInfo()
        am.getMemoryInfo(info)
        val totalGb = info.totalMem / (1024.0 * 1024.0 * 1024.0)
        return when {
            totalGb >= 3.0 -> DeviceTier.A
            totalGb >= 1.8 -> DeviceTier.B
            else -> DeviceTier.C
        }
    }

    fun totalRamBytes(context: Context): Long {
        val am = context.getSystemService(Context.ACTIVITY_SERVICE) as ActivityManager
        val info = ActivityManager.MemoryInfo()
        am.getMemoryInfo(info)
        return info.totalMem
    }
}
