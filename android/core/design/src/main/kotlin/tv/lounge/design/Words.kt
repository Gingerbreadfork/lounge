package tv.lounge.design

import android.content.res.Resources
import androidx.compose.runtime.Composable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.ui.platform.LocalContext
import java.time.Instant
import java.time.LocalTime
import java.time.ZoneId

/**
 * How Lounge writes times, so every screen says them the same way (DESIGN.md §10): clocks
 * follow the TV's 24-hour setting and lengths of time are spelled out, in the language of
 * the resources given.
 */
object Words {
    /** "21:42" on a 24-hour clock, else "9:42 pm"; without [period] just "9:42", as the top bar shows it. */
    fun clock(resources: Resources, time: LocalTime, is24Hour: Boolean, period: Boolean = true): String {
        val minutes = time.minute.toString().padStart(2, '0')
        if (is24Hour) return "${time.hour.toString().padStart(2, '0')}:$minutes"
        val digits = "${(time.hour + 11) % 12 + 1}:$minutes"
        return if (period) resources.getString(if (time.hour < 12) R.string.words_clock_am else R.string.words_clock_pm, digits) else digits
    }

    fun clock(resources: Resources, epochMs: Long, is24Hour: Boolean, zone: ZoneId = ZoneId.systemDefault(), period: Boolean = true): String =
        clock(resources, Instant.ofEpochMilli(epochMs).atZone(zone).toLocalTime(), is24Hour, period)

    /** How long ago something [ms] old was, as the phone says it: "just now", "3 minutes ago", "1 hour ago", "2 days ago". */
    fun ago(resources: Resources, ms: Long): String {
        val minutes = (ms / 60_000).coerceAtLeast(0)
        fun counted(plural: Int, n: Long) = resources.getQuantityString(plural, n.toInt(), n.toInt())
        return when {
            minutes < 1 -> resources.getString(R.string.words_just_now)
            minutes < 60 -> counted(R.plurals.words_minutes_ago, minutes)
            minutes < 24 * 60 -> counted(R.plurals.words_hours_ago, minutes / 60)
            else -> counted(R.plurals.words_days_ago, minutes / (24 * 60))
        }
    }
}

/** Whether the TV shows a 24-hour clock. Read where the time is drawn, so a change shows the next time it is. */
@Composable
@ReadOnlyComposable
fun is24HourClock(): Boolean = android.text.format.DateFormat.is24HourFormat(LocalContext.current)
