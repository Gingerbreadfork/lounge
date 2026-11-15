package tv.lounge.platform

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.ContextWrapper
import android.content.Intent
import android.os.Bundle
import android.provider.Browser
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import tv.lounge.core.PlatformException

/** What a stream's player is handed: the kind it is asked for as, its title, its start and its headers. */
@RunWith(RobolectricTestRunner::class)
class PlayStreamTest {
    /** A TV whose players take the intents [takes] accepts, and which remembers what it opened. */
    private class Tv(base: Context, private val takes: (Intent) -> Boolean = { true }) : ContextWrapper(base) {
        val opened = mutableListOf<Intent>()

        override fun startActivity(intent: Intent) {
            if (!takes(intent)) throw ActivityNotFoundException("no player for ${intent.type}")
            opened += intent
        }
    }

    private fun tv(takes: (Intent) -> Boolean = { true }) = Tv(RuntimeEnvironment.getApplication(), takes)

    @Test
    fun a_film_goes_to_a_video_player_from_where_it_was_left() {
        val tv = tv()
        AndroidPlatform(tv).playStream("http://dvr/film.mkv", "A film", 600_000L, emptyMap(), audio = false)
        val intent = tv.opened.single()
        assertEquals(Intent.ACTION_VIEW, intent.action)
        assertEquals("http://dvr/film.mkv", intent.dataString)
        assertEquals("video/*", intent.type)
        assertEquals("A film", intent.getStringExtra(Intent.EXTRA_TITLE))
        assertEquals(600_000L, intent.getLongExtra("position", -1))
        // A stream that asks for no headers is handed none.
        assertFalse(intent.hasExtra("headers"))
        assertFalse(intent.hasExtra(Browser.EXTRA_HEADERS))
    }

    @Test
    fun headers_are_handed_over_both_ways_players_read_them() {
        val tv = tv()
        val headers = linkedMapOf("User-Agent" to "Player/2.0", "Referer" to "https://two.example/")
        AndroidPlatform(tv).playStream("http://s/two.m3u8", "Two", null, headers, audio = false)
        val intent = tv.opened.single()
        assertArrayEquals(arrayOf("User-Agent", "Player/2.0", "Referer", "https://two.example/"), intent.getStringArrayExtra("headers"))
        val bundle = intent.getBundleExtra(Browser.EXTRA_HEADERS) ?: Bundle()
        assertEquals(setOf("User-Agent", "Referer"), bundle.keySet())
        assertEquals("Player/2.0", bundle.getString("User-Agent"))
        assertFalse(intent.hasExtra("position"))
    }

    @Test
    fun sound_is_offered_to_an_audio_player_and_then_to_a_video_one() {
        val both = tv()
        AndroidPlatform(both).playStream("http://radio/live", "A station", null, emptyMap(), audio = true)
        assertEquals(listOf("audio/*"), both.opened.map { it.type })

        // A TV with only a video player still plays it.
        val videoOnly = tv { it.type == "video/*" }
        AndroidPlatform(videoOnly).playStream("http://radio/live", "A station", null, emptyMap(), audio = true)
        assertEquals(listOf("video/*"), videoOnly.opened.map { it.type })
        assertEquals("http://radio/live", videoOnly.opened.single().dataString)
    }

    @Test
    fun a_stream_no_player_takes_says_what_to_install() {
        val none = tv { false }
        val film = assertThrows(PlatformException.Failed::class.java) {
            AndroidPlatform(none).playStream("http://dvr/film.mkv", "A film", null, emptyMap(), audio = false)
        }
        assertEquals("No video player can open this stream. Install VLC or another player.", film.detail)
        val station = assertThrows(PlatformException.Failed::class.java) {
            AndroidPlatform(none).playStream("http://radio/live", "A station", null, emptyMap(), audio = true)
        }
        assertEquals("No player can open this stream. Install VLC or another player.", station.detail)
        assertNull(none.opened.firstOrNull())
    }
}
