package tv.lounge.feature.home

import tv.lounge.core.HomeSnapshot
import tv.lounge.core.Row
import tv.lounge.core.RowSize
import tv.lounge.core.Stage
import tv.lounge.core.Tile
import tv.lounge.core.TileKind

/** A plain tile for tests: no pictures, so nothing is loaded. */
internal fun testTile(
    id: String,
    title: String = id,
    subtitle: String? = null,
    kind: TileKind = TileKind.APP_BANNER,
    provider: String? = null,
    hidden: Boolean = false,
    action: String = "open:$id",
) = Tile(
    id = id,
    kind = kind,
    title = title,
    subtitle = subtitle,
    artworkPath = null,
    iconPath = null,
    lightArgb = 0xFFF2B263u,
    progress = null,
    badge = null,
    action = action,
    pinned = false,
    stagePinned = false,
    borrowedArt = false,
    artTitle = null,
    openedRank = null,
    installedRank = null,
    provider = provider,
    providerName = null,
    hidden = hidden,
    timeLeft = null,
)

internal fun testRow(id: String, vararg tiles: Tile, title: String = id, kind: TileKind = TileKind.LANDSCAPE) =
    Row(id = id, title = title, kind = kind, size = RowSize.M, badge = null, tiles = tiles.toList(), note = null)

internal fun testSnapshot(rows: List<Row> = emptyList(), allApps: List<Tile> = emptyList(), strip: List<Tile> = emptyList()) = HomeSnapshot(
    revision = 1uL,
    generatedAtMs = 0L,
    stage = Stage(spotlights = emptyList(), strip = strip),
    rows = rows,
    hiddenRows = emptyList(),
    allApps = allApps,
    nowPlaying = null,
    wallpapers = emptyList(),
)
