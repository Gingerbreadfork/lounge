package tv.lounge.design

import androidx.compose.runtime.Composable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.ui.platform.LocalContext
import java.time.Instant
import java.time.LocalTime
import java.time.ZoneId

/**
 * How Lounge writes times and counts, so every screen says them the same way (DESIGN.md §10):
 * clocks follow the TV's 24-hour setting and lengths of time are spelled out.
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

    /** How long ago something [ms] old was, as the phone says it: "just now", "3 minutes ago", "1 hour ago", "2 days ago". */
    fun ago(ms: Long): String {
        val minutes = (ms / 60_000).coerceAtLeast(0)
        return when {
            minutes < 1 -> "just now"
            minutes < 60 -> "${counted(minutes, "minute", "minutes")} ago"
            minutes < 24 * 60 -> "${counted(minutes / 60, "hour", "hours")} ago"
            else -> "${counted(minutes / (24 * 60), "day", "days")} ago"
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
