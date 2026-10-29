package tv.lounge.platform

import java.net.URI

/** Item image paths of Jellyfin and Emby servers, as the apps publish them. */
private val ITEM_IMAGE = Regex("""/Items/[^/]+/Images/([A-Za-z]+)(/\d+)?$""")
private val SIZES = setOf("maxwidth", "maxheight", "width", "height", "fillwidth", "fillheight")

/**
 * A picture's address asked for as a JPEG no larger than the tiles need, where the address is a
 * Jellyfin or Emby item image: the apps publish originals, or ask for WebP, and either is many
 * times the bytes of a JPEG and far slower to decode. A size the address names is kept; a logo,
 * which needs its transparency, is left alone, as is any other address, including the Jellyfin
 * app's own content provider, which answers a changed address with a placeholder.
 */
fun sizedForTiles(uri: String): String {
    val parsed = runCatching { URI(uri) }.getOrNull() ?: return uri
    if (parsed.scheme != "http" && parsed.scheme != "https") return uri
    val kind = ITEM_IMAGE.find(parsed.rawPath ?: return uri)?.groupValues?.get(1) ?: return uri
    if (kind.equals("Logo", ignoreCase = true)) return uri
    val params = parsed.rawQuery.orEmpty().split('&').filter { it.isNotEmpty() }.filterNot { it.substringBefore('=').equals("format", ignoreCase = true) }
    val sized = params.any { it.substringBefore('=').lowercase() in SIZES }
    val query = (params + "format=jpg" + if (sized) emptyList() else listOf("maxWidth=1280", "maxHeight=1280")).joinToString("&")
    val fragment = parsed.rawFragment?.let { "#$it" }.orEmpty()
    return uri.substringBefore('?').substringBefore('#') + "?" + query + fragment
}
