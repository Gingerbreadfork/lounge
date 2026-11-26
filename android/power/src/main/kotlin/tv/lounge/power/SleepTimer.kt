package tv.lounge.power

import android.app.AlarmManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.PowerManager
import androidx.core.content.ContextCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Turns everything off at a time the person chose. The time is kept on disk, so a timer outlives
 * the process: an alarm brings it back, though Android may deliver that one late. Turning the
 * screen off by hand ends the timer.
 */
class SleepTimer(
    private val context: Context,
    private val clock: () -> Long = System::currentTimeMillis,
    private val screenOn: () -> Boolean = { context.getSystemService(PowerManager::class.java)?.isInteractive != false },
    private val turnOff: suspend () -> Unit,
) {
    private val store by lazy { context.getSharedPreferences("sleep_timer", Context.MODE_PRIVATE) }
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private var waiting: Job? = null
    private var watchingScreen = false
    private val screenOff = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent?) = cancel()
    }
    private val _endsAt = MutableStateFlow<Long?>(null)

    /** When everything turns off, in wall-clock milliseconds, or null while no timer runs. */
    val endsAt: StateFlow<Long?> = _endsAt.asStateFlow()

    /** Picks up a timer set before the process last ended, unless its time passed while the TV was off. */
    fun restore() {
        val at = store.getLong(KEY, 0L)
        when {
            at == 0L -> Unit
            at < clock() - LATE_MS -> cancel()
            else -> arm(at)
        }
    }

    fun set(minutes: Int) {
        val at = clock() + minutes * 60_000L
        store.edit().putLong(KEY, at).apply()
        arm(at)
    }

    fun cancel() {
        store.edit().remove(KEY).apply()
        disarm()
    }

    @Synchronized
    private fun arm(at: Long) {
        disarm()
        _endsAt.value = at
        waiting = scope.launch {
            delay((at - clock()).coerceAtLeast(0L))
            withContext(NonCancellable) { fireIfDue() }
        }
        runCatching { alarms()?.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, alarm()) }
        watchScreen(true)
    }

    private fun watchScreen(watch: Boolean) {
        if (watch == watchingScreen) return
        watchingScreen = watch
        runCatching {
            if (watch) {
                ContextCompat.registerReceiver(context, screenOff, IntentFilter(Intent.ACTION_SCREEN_OFF), ContextCompat.RECEIVER_NOT_EXPORTED)
            } else {
                context.unregisterReceiver(screenOff)
            }
        }
    }

    @Synchronized
    private fun disarm() {
        waiting?.cancel()
        waiting = null
        _endsAt.value = null
        runCatching { alarms()?.cancel(alarm()) }
        watchScreen(false)
    }

    /**
     * Turns everything off if the timer has run out and has not already done so. A timer found
     * long past its time, or with the screen already off, is dropped and turns nothing off.
     */
    suspend fun fireIfDue() {
        val due = synchronized(this) {
            val at = store.getLong(KEY, 0L)
            if (at == 0L || at > clock() + EARLY_MS) {
                false
            } else {
                store.edit().remove(KEY).apply()
                _endsAt.value = null
                waiting = null
                runCatching { alarms()?.cancel(alarm()) }
                watchScreen(false)
                at >= clock() - LATE_MS && screenOn()
            }
        }
        if (due) runCatching { turnOff() }.onFailure { android.util.Log.w("Lounge", "sleep timer: could not turn off", it) }
    }

    private fun alarms(): AlarmManager? = context.getSystemService(AlarmManager::class.java)

    private fun alarm(): PendingIntent =
        PendingIntent.getBroadcast(context, 0, Intent(context, SleepTimerReceiver::class.java), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)

    private companion object {
        const val KEY = "ends_at"
        /** An alarm a moment early still counts as the timer running out. */
        const val EARLY_MS = 5_000L
        /** Later than this, the TV has been off and on again since, and the timer no longer stands. */
        const val LATE_MS = 60_000L
    }
}

/** What holds the app's one [PowerSetup], so a receiver can reach it. The application implements it. */
interface PowerSetupOwner {
    val power: PowerSetup
}

/** The alarm behind [SleepTimer], for when the process had ended before the timer did. */
class SleepTimerReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent?) {
        val timer = (context.applicationContext as? PowerSetupOwner)?.power?.sleepTimer ?: return
        val pending = goAsync()
        CoroutineScope(Dispatchers.Default).launch {
            try {
                timer.fireIfDue()
            } finally {
                pending.finish()
            }
        }
    }
}

/**
 * The time a press moves the timer to, in minutes, and 0 for off. [left] is what remains now, or
 * null with no timer. Forward goes to the next longer time and then off, back to the next shorter.
 */
fun sleepStep(left: Int?, forward: Boolean): Int {
    val steps = listOf(15, 30, 45, 60, 90, 120)
    val now = left ?: 0
    return if (forward) {
        steps.firstOrNull { it > now } ?: 0
    } else {
        if (left == null) steps.last() else steps.lastOrNull { it < now } ?: 0
    }
}
