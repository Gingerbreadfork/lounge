package tv.lounge.design

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioManager
import android.media.SoundPool
import android.os.SystemClock
import kotlin.random.Random

/**
 * The three sounds (DESIGN.md §9): `tick` on focus move, `open` on selection, `close` on Back.
 * Played on the UI sonification stream so the TV's volume applies; silent while music plays or when
 * sounds are off.
 */
class UiSounds(context: Context) {
    private val audio = context.getSystemService(Context.AUDIO_SERVICE) as AudioManager
    private val pool = SoundPool.Builder()
        .setMaxStreams(3)
        .setAudioAttributes(
            AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_ASSISTANCE_SONIFICATION)
                .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                .build(),
        )
        .build()
    private val tick = pool.load(context, R.raw.ui_tick, 1)
    private val open = pool.load(context, R.raw.ui_open, 1)
    private val close = pool.load(context, R.raw.ui_close, 1)

    @Volatile var enabled: Boolean = true
    /** True while a d-pad key is auto-repeating; ticks are quieter and no more than one per 90 ms. */
    @Volatile var repeating: Boolean = false
    private var lastTickAt = 0L

    /** Focus moved. A row change is pitched a little lower, a quiet spatial cue; pitch jitters ±3 % so held keys never machine-gun. */
    fun tick(rowChange: Boolean = false) {
        val now = SystemClock.uptimeMillis()
        if (now - lastTickAt < TICK_GAP_MS) return
        lastTickAt = now
        val base = if (rowChange) 0.94f else 1f
        play(tick, if (repeating) 0.3f else 0.6f, base * (0.97f + Random.nextFloat() * 0.06f))
    }

    fun open() = play(open, 0.9f)
    fun close() = play(close, 0.9f)

    private fun play(id: Int, volume: Float, rate: Float = 1f) {
        if (!enabled || audio.isMusicActive) return
        pool.play(id, volume, volume, 1, 0, rate)
    }

    fun release() = pool.release()

    private companion object {
        const val TICK_GAP_MS = 90L
    }
}
