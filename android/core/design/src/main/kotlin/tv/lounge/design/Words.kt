package tv.lounge.design

import androidx.compose.runtime.Composable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.ui.platform.LocalContext
import java.time.Instant
import java.time.LocalTime
import java.time.ZoneId

/**
 * How Lounge writes times, durations and counts, so every screen says them the same way
 * (DESIGN.md §10): clocks follow the TV's 24-hour setting and durations are spelled out.
 */
object Words {
    /** "21:42" on a 24-hour clock, else "9:42 pm"; without [period] just "9:42", as the top bar shows it. */
    fun clock(time: LocalTime, is24Hour: Boolean, period: Boolean = true): String {
        val minutes = time.minute.toString().padStart(2, '0')
        if (is24Hour) return "${time.hour.toString().padStart(2, '0')}:$minutes"
        val digits = "${(time.hour + 11) % 12 + 1}:$minutes"
        return if (period) "$digits ${if (time.hour < 12) "am" else "pm"}" else digits
    }

    fun clock(epochMs: Long, is24Hour: Boolean, zone: ZoneId = ZoneId.systemDefault(), period: Boolean = true): String =
        clock(Instant.ofEpochMilli(epochMs).atZone(zone).toLocalTime(), is24Hour, period)

    /** "1 hour 20 minutes", "23 minutes", to the nearest minute and never less than one. */
    fun duration(ms: Long): String {
        val minutes = ((ms + 30_000) / 60_000).coerceAtLeast(1)
        val hours = counted(minutes / 60, "hour", "hours")
        val rest = counted(minutes % 60, "minute", "minutes")
        return when {
            minutes < 60 -> rest
            minutes % 60 == 0L -> hours
            else -> "$hours $rest"
        }
    }

    /** "23 minutes left", "1 hour 20 minutes left", or "Almost finished" in the last half minute. */
    fun left(ms: Long): String = if (ms < 30_000) "Almost finished" else "${duration(ms)} left"

    /** "just now", "3 minutes ago", "an hour ago", "5 hours ago", "yesterday" or "3 days ago". */
    fun ago(ms: Long): String {
        val minutes = (ms / 60_000).coerceAtLeast(0)
        return when {
            minutes < 2 -> "just now"
            minutes < 60 -> "$minutes minutes ago"
            minutes < 120 -> "an hour ago"
            minutes < 24 * 60 -> "${minutes / 60} hours ago"
            minutes < 48 * 60 -> "yesterday"
            else -> "${minutes / (24 * 60)} days ago"
        }
    }

    /** "1 app", "3 apps". */
    fun counted(n: Long, one: String, many: String): String = if (n == 1L) "1 $one" else "$n $many"

    fun counted(n: Int, one: String, many: String): String = counted(n.toLong(), one, many)
}

/** Whether the TV shows a 24-hour clock. Read where the time is drawn, so a change shows the next time it is. */
@Composable
@ReadOnlyComposable
fun is24HourClock(): Boolean = android.text.format.DateFormat.is24HourFormat(LocalContext.current)
