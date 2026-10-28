package tv.lounge.design

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.PathFillType
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.path
import androidx.compose.ui.unit.dp

/** Stroke icons on a 28-unit grid. */
object LoungeIcons {
    private fun icon(name: String, builder: androidx.compose.ui.graphics.vector.ImageVector.Builder.() -> Unit): ImageVector =
        ImageVector.Builder(name = name, defaultWidth = 28.dp, defaultHeight = 28.dp, viewportWidth = 28f, viewportHeight = 28f)
            .apply(builder)
            .build()

    private fun androidx.compose.ui.graphics.vector.ImageVector.Builder.stroke(d: androidx.compose.ui.graphics.vector.PathBuilder.() -> Unit) {
        path(
            fill = null,
            stroke = SolidColor(Color.White),
            strokeLineWidth = 2.2f,
            strokeLineCap = StrokeCap.Round,
            strokeLineJoin = StrokeJoin.Round,
            pathFillType = PathFillType.NonZero,
            pathBuilder = d,
        )
    }

    private fun androidx.compose.ui.graphics.vector.ImageVector.Builder.fill(d: androidx.compose.ui.graphics.vector.PathBuilder.() -> Unit) {
        path(fill = SolidColor(Color.White), stroke = null, pathBuilder = d)
    }

    val Search: ImageVector by lazy {
        icon("search") {
            stroke { moveTo(12f, 4.5f); arcTo(7.5f, 7.5f, 0f, true, true, 12f, 19.5f); arcTo(7.5f, 7.5f, 0f, true, true, 12f, 4.5f) }
            stroke { moveTo(17.5f, 17.5f); lineTo(24f, 24f) }
        }
    }

    val Tv: ImageVector by lazy {
        icon("tv") {
            stroke { moveTo(5.5f, 5f); lineTo(22.5f, 5f); arcTo(2.5f, 2.5f, 0f, false, true, 25f, 7.5f); lineTo(25f, 17.5f); arcTo(2.5f, 2.5f, 0f, false, true, 22.5f, 20f); lineTo(5.5f, 20f); arcTo(2.5f, 2.5f, 0f, false, true, 3f, 17.5f); lineTo(3f, 7.5f); arcTo(2.5f, 2.5f, 0f, false, true, 5.5f, 5f); close() }
            stroke { moveTo(10f, 24f); lineTo(18f, 24f) }
        }
    }

    /** A programme grid: a channel column and two rows of time. */
    val Guide: ImageVector by lazy {
        icon("guide") {
            stroke { moveTo(5.5f, 5f); lineTo(22.5f, 5f); arcTo(2.5f, 2.5f, 0f, false, true, 25f, 7.5f); lineTo(25f, 20.5f); arcTo(2.5f, 2.5f, 0f, false, true, 22.5f, 23f); lineTo(5.5f, 23f); arcTo(2.5f, 2.5f, 0f, false, true, 3f, 20.5f); lineTo(3f, 7.5f); arcTo(2.5f, 2.5f, 0f, false, true, 5.5f, 5f); close() }
            stroke { moveTo(10f, 5f); lineTo(10f, 23f) }
            stroke { moveTo(10f, 11f); lineTo(25f, 11f) }
            stroke { moveTo(10f, 17f); lineTo(25f, 17f) }
            stroke { moveTo(16f, 11f); lineTo(16f, 17f) }
        }
    }

    val Settings: ImageVector by lazy {
        icon("settings") {
            stroke { moveTo(4f, 8f); lineTo(24f, 8f) }
            stroke { moveTo(4f, 14f); lineTo(24f, 14f) }
            stroke { moveTo(4f, 20f); lineTo(24f, 20f) }
            fill { moveTo(10f, 5.4f); arcTo(2.6f, 2.6f, 0f, true, true, 10f, 10.6f); arcTo(2.6f, 2.6f, 0f, true, true, 10f, 5.4f) }
            fill { moveTo(18f, 11.4f); arcTo(2.6f, 2.6f, 0f, true, true, 18f, 16.6f); arcTo(2.6f, 2.6f, 0f, true, true, 18f, 11.4f) }
            fill { moveTo(8f, 17.4f); arcTo(2.6f, 2.6f, 0f, true, true, 8f, 22.6f); arcTo(2.6f, 2.6f, 0f, true, true, 8f, 17.4f) }
        }
    }

    val Controller: ImageVector by lazy {
        icon("controller") {
            // A pad with two grips, so it still reads as one at the size of a row badge.
            stroke {
                moveTo(20.2f, 6f); lineTo(7.8f, 6f)
                arcTo(4.6f, 4.6f, 0f, false, false, 3.2f, 10.2f)
                lineTo(2.4f, 18.6f)
                arcTo(3.4f, 3.4f, 0f, false, false, 5.8f, 22f)
                curveTo(7f, 22f, 7.6f, 21.4f, 8.2f, 20.8f)
                lineTo(9.8f, 19.2f)
                arcTo(2.3f, 2.3f, 0f, false, true, 11.4f, 18.6f)
                lineTo(16.6f, 18.6f)
                arcTo(2.3f, 2.3f, 0f, false, true, 18.2f, 19.2f)
                lineTo(19.8f, 20.8f)
                curveTo(20.4f, 21.4f, 21f, 22f, 22.2f, 22f)
                arcTo(3.4f, 3.4f, 0f, false, false, 25.6f, 18.6f)
                lineTo(24.8f, 10.2f)
                arcTo(4.6f, 4.6f, 0f, false, false, 20.2f, 6f)
                close()
            }
            stroke { moveTo(9.2f, 10.4f); lineTo(9.2f, 15.2f) }
            stroke { moveTo(6.8f, 12.8f); lineTo(11.6f, 12.8f) }
            fill { moveTo(17.6f, 12.9f); arcTo(1.3f, 1.3f, 0f, true, true, 17.6f, 15.5f); arcTo(1.3f, 1.3f, 0f, true, true, 17.6f, 12.9f) }
            fill { moveTo(20.8f, 10.3f); arcTo(1.3f, 1.3f, 0f, true, true, 20.8f, 12.9f); arcTo(1.3f, 1.3f, 0f, true, true, 20.8f, 10.3f) }
        }
    }

    val Power: ImageVector by lazy {
        icon("power") {
            stroke { moveTo(14f, 4f); lineTo(14f, 14f) }
            stroke { moveTo(8.2f, 8.2f); arcTo(8f, 8f, 0f, true, false, 19.8f, 8.2f) }
        }
    }

    val Volume: ImageVector by lazy {
        icon("volume") {
            stroke { moveTo(4f, 11f); lineTo(9f, 11f); lineTo(15f, 6f); lineTo(15f, 22f); lineTo(9f, 17f); lineTo(4f, 17f); close() }
            stroke { moveTo(19f, 10f); arcTo(5f, 5f, 0f, false, true, 19f, 18f) }
        }
    }

    val Mute: ImageVector by lazy {
        icon("mute") {
            stroke { moveTo(4f, 11f); lineTo(9f, 11f); lineTo(15f, 6f); lineTo(15f, 22f); lineTo(9f, 17f); lineTo(4f, 17f); close() }
            stroke { moveTo(19f, 11f); lineTo(25f, 17f) }
            stroke { moveTo(25f, 11f); lineTo(19f, 17f) }
        }
    }

    val Soundbar: ImageVector by lazy {
        icon("soundbar") {
            stroke { moveTo(5f, 10f); lineTo(23f, 10f); arcTo(3f, 3f, 0f, false, true, 26f, 13f); lineTo(26f, 15f); arcTo(3f, 3f, 0f, false, true, 23f, 18f); lineTo(5f, 18f); arcTo(3f, 3f, 0f, false, true, 2f, 15f); lineTo(2f, 13f); arcTo(3f, 3f, 0f, false, true, 5f, 10f); close() }
            fill { moveTo(8f, 12.4f); arcTo(1.6f, 1.6f, 0f, true, true, 8f, 15.6f); arcTo(1.6f, 1.6f, 0f, true, true, 8f, 12.4f) }
            fill { moveTo(14f, 12.4f); arcTo(1.6f, 1.6f, 0f, true, true, 14f, 15.6f); arcTo(1.6f, 1.6f, 0f, true, true, 14f, 12.4f) }
            fill { moveTo(20f, 12.4f); arcTo(1.6f, 1.6f, 0f, true, true, 20f, 15.6f); arcTo(1.6f, 1.6f, 0f, true, true, 20f, 12.4f) }
        }
    }

    val Hdmi: ImageVector by lazy {
        icon("hdmi") {
            stroke { moveTo(3f, 9f); lineTo(25f, 9f); lineTo(25f, 15f); lineTo(21f, 19f); lineTo(7f, 19f); lineTo(3f, 15f); close() }
            stroke { moveTo(8f, 13f); lineTo(20f, 13f) }
        }
    }

    val Mic: ImageVector by lazy {
        icon("mic") {
            stroke { moveTo(14f, 3f); arcTo(4f, 4f, 0f, false, true, 18f, 7f); lineTo(18f, 13f); arcTo(4f, 4f, 0f, false, true, 10f, 13f); lineTo(10f, 7f); arcTo(4f, 4f, 0f, false, true, 14f, 3f); close() }
            stroke { moveTo(6f, 13f); arcTo(8f, 8f, 0f, false, false, 22f, 13f) }
            stroke { moveTo(14f, 21f); lineTo(14f, 25f) }
            stroke { moveTo(10f, 25f); lineTo(18f, 25f) }
        }
    }

    /** A disc half in shadow: the theme. */
    val Theme: ImageVector by lazy {
        icon("theme") {
            stroke { moveTo(14f, 4f); arcTo(10f, 10f, 0f, true, true, 14f, 24f); arcTo(10f, 10f, 0f, true, true, 14f, 4f) }
            fill { moveTo(14f, 4f); arcTo(10f, 10f, 0f, false, true, 14f, 24f); close() }
        }
    }

    /** A picture frame with a horizon: the screensaver. */
    val Frame: ImageVector by lazy {
        icon("frame") {
            stroke { moveTo(5f, 6f); lineTo(23f, 6f); lineTo(23f, 22f); lineTo(5f, 22f); close() }
            stroke { moveTo(5f, 17f); lineTo(11f, 12f); lineTo(15f, 15f); lineTo(18f, 13f); lineTo(23f, 17f) }
            fill { moveTo(18f, 9f); arcTo(1.5f, 1.5f, 0f, true, true, 18f, 12f); arcTo(1.5f, 1.5f, 0f, true, true, 18f, 9f) }
        }
    }

    val Phone: ImageVector by lazy {
        icon("phone") {
            stroke {
                moveTo(10f, 3f); lineTo(18f, 3f); arcTo(2.5f, 2.5f, 0f, false, true, 20.5f, 5.5f); lineTo(20.5f, 22.5f)
                arcTo(2.5f, 2.5f, 0f, false, true, 18f, 25f); lineTo(10f, 25f); arcTo(2.5f, 2.5f, 0f, false, true, 7.5f, 22.5f)
                lineTo(7.5f, 5.5f); arcTo(2.5f, 2.5f, 0f, false, true, 10f, 3f); close()
            }
            stroke { moveTo(12.5f, 21f); lineTo(15.5f, 21f) }
        }
    }

    val Refresh: ImageVector by lazy {
        icon("refresh") {
            stroke { moveTo(20.1f, 8.9f); arcTo(8f, 8f, 0f, true, true, 7.9f, 8.9f) }
            stroke { moveTo(7.9f, 3.8f); lineTo(7.9f, 8.9f); lineTo(13f, 8.9f) }
        }
    }

    val Pause: ImageVector by lazy {
        icon("pause") {
            fill { moveTo(7f, 5f); lineTo(11.5f, 5f); lineTo(11.5f, 23f); lineTo(7f, 23f); close() }
            fill { moveTo(16.5f, 5f); lineTo(21f, 5f); lineTo(21f, 23f); lineTo(16.5f, 23f); close() }
        }
    }

    val SkipNext: ImageVector by lazy {
        icon("skip-next") {
            fill { moveTo(5f, 6f); lineTo(17f, 14f); lineTo(5f, 22f); close() }
            fill { moveTo(19.5f, 6f); lineTo(23f, 6f); lineTo(23f, 22f); lineTo(19.5f, 22f); close() }
        }
    }

    val SkipPrevious: ImageVector by lazy {
        icon("skip-previous") {
            fill { moveTo(23f, 6f); lineTo(11f, 14f); lineTo(23f, 22f); close() }
            fill { moveTo(5f, 6f); lineTo(8.5f, 6f); lineTo(8.5f, 22f); lineTo(5f, 22f); close() }
        }
    }

    val Equaliser: ImageVector by lazy {
        icon("equaliser") {
            fill { moveTo(5f, 14f); lineTo(9f, 14f); lineTo(9f, 23f); lineTo(5f, 23f); close() }
            fill { moveTo(12f, 7f); lineTo(16f, 7f); lineTo(16f, 23f); lineTo(12f, 23f); close() }
            fill { moveTo(19f, 11f); lineTo(23f, 11f); lineTo(23f, 23f); lineTo(19f, 23f); close() }
        }
    }

    val Play: ImageVector by lazy {
        icon("play") {
            fill { moveTo(9f, 6f); lineTo(22f, 14f); lineTo(9f, 22f); close() }
        }
    }

    val Info: ImageVector by lazy {
        icon("info") {
            stroke { moveTo(14f, 3.5f); arcTo(10.5f, 10.5f, 0f, true, true, 14f, 24.5f); arcTo(10.5f, 10.5f, 0f, true, true, 14f, 3.5f) }
            stroke { moveTo(14f, 13f); lineTo(14f, 19f) }
            fill { moveTo(14f, 8.2f); arcTo(1.3f, 1.3f, 0f, true, true, 14f, 10.8f); arcTo(1.3f, 1.3f, 0f, true, true, 14f, 8.2f) }
        }
    }

    val Hide: ImageVector by lazy {
        icon("hide") {
            stroke { moveTo(3f, 14f); curveTo(6f, 8f, 10f, 6f, 14f, 6f); curveTo(18f, 6f, 22f, 8f, 25f, 14f); curveTo(22f, 20f, 18f, 22f, 14f, 22f); curveTo(10f, 22f, 6f, 20f, 3f, 14f); close() }
            stroke { moveTo(5f, 23f); lineTo(23f, 5f) }
        }
    }

    val Pin: ImageVector by lazy {
        icon("pin") {
            stroke { moveTo(10f, 3f); lineTo(18f, 3f); lineTo(17f, 11f); lineTo(21f, 15f); lineTo(7f, 15f); lineTo(11f, 11f); close() }
            stroke { moveTo(14f, 15f); lineTo(14f, 25f) }
        }
    }

    val Move: ImageVector by lazy {
        icon("move") {
            stroke { moveTo(3f, 14f); lineTo(25f, 14f) }
            stroke { moveTo(7f, 10f); lineTo(3f, 14f); lineTo(7f, 18f) }
            stroke { moveTo(21f, 10f); lineTo(25f, 14f); lineTo(21f, 18f) }
        }
    }

    val Apps: ImageVector by lazy {
        icon("apps") {
            stroke { moveTo(4f, 4f); lineTo(11f, 4f); lineTo(11f, 11f); lineTo(4f, 11f); close() }
            stroke { moveTo(17f, 4f); lineTo(24f, 4f); lineTo(24f, 11f); lineTo(17f, 11f); close() }
            stroke { moveTo(4f, 17f); lineTo(11f, 17f); lineTo(11f, 24f); lineTo(4f, 24f); close() }
            stroke { moveTo(17f, 17f); lineTo(24f, 17f); lineTo(24f, 24f); lineTo(17f, 24f); close() }
        }
    }

    val Trash: ImageVector by lazy {
        icon("trash") {
            stroke { moveTo(4f, 7f); lineTo(24f, 7f) }
            stroke { moveTo(10f, 7f); lineTo(10f, 4f); lineTo(18f, 4f); lineTo(18f, 7f) }
            stroke { moveTo(7f, 7f); lineTo(8f, 24f); lineTo(20f, 24f); lineTo(21f, 7f) }
        }
    }
}
