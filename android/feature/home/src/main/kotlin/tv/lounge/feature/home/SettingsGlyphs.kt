package tv.lounge.feature.home

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.PathFillType
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.PathBuilder
import androidx.compose.ui.graphics.vector.path
import androidx.compose.ui.unit.dp

/** Glyphs for settings rows whose meaning `LoungeIcons` has none for, drawn the same way. */
internal object SettingsGlyphs {
    private fun icon(name: String, builder: ImageVector.Builder.() -> Unit): ImageVector =
        ImageVector.Builder(name = name, defaultWidth = 28.dp, defaultHeight = 28.dp, viewportWidth = 28f, viewportHeight = 28f)
            .apply(builder)
            .build()

    private fun ImageVector.Builder.stroke(d: PathBuilder.() -> Unit) {
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

    private fun ImageVector.Builder.fill(d: PathBuilder.() -> Unit) {
        path(fill = SolidColor(Color.White), stroke = null, pathBuilder = d)
    }

    /** A house: the default home screen. */
    val Home: ImageVector by lazy {
        icon("home") {
            stroke { moveTo(4f, 13f); lineTo(14f, 4.5f); lineTo(24f, 13f) }
            stroke { moveTo(7f, 11f); lineTo(7f, 23.5f); lineTo(21f, 23.5f); lineTo(21f, 11f) }
            stroke { moveTo(12f, 23.5f); lineTo(12f, 17f); lineTo(16f, 17f); lineTo(16f, 23.5f) }
        }
    }

    /** A plug: plugins. */
    val Plugin: ImageVector by lazy {
        icon("plugin") {
            stroke { moveTo(10f, 3.5f); lineTo(10f, 9f) }
            stroke { moveTo(18f, 3.5f); lineTo(18f, 9f) }
            stroke { moveTo(6.5f, 9f); lineTo(21.5f, 9f); lineTo(21.5f, 13f); arcTo(7.5f, 7.5f, 0f, false, true, 6.5f, 13f); close() }
            stroke { moveTo(14f, 20.5f); lineTo(14f, 25f) }
        }
    }

    /** An arrow into a tray: updates. */
    val Download: ImageVector by lazy {
        icon("download") {
            stroke { moveTo(14f, 4f); lineTo(14f, 17f) }
            stroke { moveTo(8.5f, 11.5f); lineTo(14f, 17f); lineTo(19.5f, 11.5f) }
            stroke { moveTo(4.5f, 18f); lineTo(4.5f, 23.5f); lineTo(23.5f, 23.5f); lineTo(23.5f, 18f) }
        }
    }

    /** A clock: what you open, and when. */
    val Clock: ImageVector by lazy {
        icon("clock") {
            stroke { moveTo(14f, 3.5f); arcTo(10.5f, 10.5f, 0f, true, true, 14f, 24.5f); arcTo(10.5f, 10.5f, 0f, true, true, 14f, 3.5f) }
            stroke { moveTo(14f, 8.5f); lineTo(14f, 14f); lineTo(18.5f, 16.5f) }
        }
    }

    /** A gauge: running lighter or faster. */
    val Gauge: ImageVector by lazy {
        icon("gauge") {
            stroke { moveTo(4.5f, 20f); arcTo(9.5f, 9.5f, 0f, false, true, 23.5f, 20f) }
            stroke { moveTo(14f, 20f); lineTo(19f, 13.5f) }
            fill { moveTo(14f, 18.2f); arcTo(1.8f, 1.8f, 0f, true, true, 14f, 21.8f); arcTo(1.8f, 1.8f, 0f, true, true, 14f, 18.2f) }
        }
    }

    /** Two slow waves: a scene drawn behind the home. */
    val Scene: ImageVector by lazy {
        icon("scene") {
            stroke { moveTo(3.5f, 10f); curveTo(7f, 7f, 10.5f, 7f, 14f, 10f); curveTo(17.5f, 13f, 21f, 13f, 24.5f, 10f) }
            stroke { moveTo(3.5f, 18f); curveTo(7f, 15f, 10.5f, 15f, 14f, 18f); curveTo(17.5f, 21f, 21f, 21f, 24.5f, 18f) }
        }
    }

    /** A phone with apps on it: apps made for phones. */
    val PhoneApps: ImageVector by lazy {
        icon("phone-apps") {
            stroke {
                moveTo(10f, 3f); lineTo(18f, 3f); arcTo(2.5f, 2.5f, 0f, false, true, 20.5f, 5.5f); lineTo(20.5f, 22.5f)
                arcTo(2.5f, 2.5f, 0f, false, true, 18f, 25f); lineTo(10f, 25f); arcTo(2.5f, 2.5f, 0f, false, true, 7.5f, 22.5f)
                lineTo(7.5f, 5.5f); arcTo(2.5f, 2.5f, 0f, false, true, 10f, 3f); close()
            }
            fill { moveTo(10.6f, 7.6f); lineTo(13.2f, 7.6f); lineTo(13.2f, 10.2f); lineTo(10.6f, 10.2f); close() }
            fill { moveTo(14.8f, 7.6f); lineTo(17.4f, 7.6f); lineTo(17.4f, 10.2f); lineTo(14.8f, 10.2f); close() }
            fill { moveTo(10.6f, 11.8f); lineTo(13.2f, 11.8f); lineTo(13.2f, 14.4f); lineTo(10.6f, 14.4f); close() }
            fill { moveTo(14.8f, 11.8f); lineTo(17.4f, 11.8f); lineTo(17.4f, 14.4f); lineTo(14.8f, 14.4f); close() }
        }
    }
}
