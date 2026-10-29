package tv.lounge.platform

import org.junit.Assert.assertEquals
import org.junit.Test

class SizedForTilesTest {
    @Test
    fun a_jellyfin_item_image_is_asked_for_as_a_sized_jpeg() {
        assertEquals(
            "http://jf:8096/Items/0a1b/Images/Primary?format=jpg&maxWidth=1280&maxHeight=1280",
            sizedForTiles("http://jf:8096/Items/0a1b/Images/Primary"),
        )
        assertEquals(
            "https://tv.home/jellyfin/Items/0a1b/Images/Backdrop/0?tag=f00&format=jpg&maxWidth=1280&maxHeight=1280",
            sizedForTiles("https://tv.home/jellyfin/Items/0a1b/Images/Backdrop/0?tag=f00"),
        )
        assertEquals(
            "http://emby:8096/emby/Items/12/Images/Primary?api_key=k&format=jpg&maxWidth=1280&maxHeight=1280",
            sizedForTiles("http://emby:8096/emby/Items/12/Images/Primary?api_key=k"),
        )
    }

    @Test
    fun another_format_gives_way_and_a_size_is_kept() {
        assertEquals(
            "http://jf:8096/Items/0a1b/Images/Primary?tag=f00&format=jpg&maxWidth=1280&maxHeight=1280",
            sizedForTiles("http://jf:8096/Items/0a1b/Images/Primary?tag=f00&format=webp"),
        )
        assertEquals(
            "http://jf:8096/Items/0a1b/Images/Primary?maxWidth=960&quality=90&format=jpg",
            sizedForTiles("http://jf:8096/Items/0a1b/Images/Primary?maxWidth=960&quality=90&Format=png"),
        )
        val already = "http://jf:8096/Items/0a1b/Images/Primary?maxWidth=960&quality=90&format=jpg"
        assertEquals(already, sizedForTiles(already))
    }

    @Test
    fun logos_and_other_addresses_are_kept() {
        for (uri in listOf(
            "http://jf:8096/Items/0a1b/Images/Logo?format=png",
            "https://cdn.example/posters/0a1b.png",
            "content://com.example.tv/images/42",
            "content://org.jellyfin.androidtv.integration.provider.ImageProvider?src=https%3A%2F%2Fjf.home%2FItems%2F2b15%2FImages%2FThumb%3Ftag%3D1ae8&v=0.19.9",
            "http://jf:8096/Items/0a1b/Images/Primary/extra/path",
            "not a uri at all",
        )) {
            assertEquals(uri, sizedForTiles(uri))
        }
    }
}
