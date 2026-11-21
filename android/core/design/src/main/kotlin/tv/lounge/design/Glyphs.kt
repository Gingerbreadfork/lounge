package tv.lounge.design

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp
import java.util.concurrent.ConcurrentHashMap

/**
 * The glyphs a plugin's control or sheet row may name, on the icons' 28-unit grid. They are
 * kept as path strings, the same ones the phone draws in companion/src/lib/Icon.svelte.
 */
object LoungeGlyphs {
    private class Shape(val d: String, val filled: Boolean = false)

    private val drawings: Map<String, List<Shape>> = mapOf(
        "battery" to listOf(Shape("M5.5 9h15a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2h-15a2 2 0 0 1-2-2v-6a2 2 0 0 1 2-2z"), Shape("M25 12.5v3"), Shape("M7.5 12.5v3")),
        "blind" to listOf(Shape("M4 5h20"), Shape("M6 5v16.5h16V5"), Shape("M6 9.5h16M6 13.5h16M6 17.5h16"), Shape("M14 21.5v3")),
        "camera" to listOf(Shape("M5.5 8.5h13a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2z"), Shape("M20.5 12.5l4.5-2.5v8l-4.5-2.5")),
        "door" to listOf(Shape("M7 24.5v-19A1.5 1.5 0 0 1 8.5 4h11A1.5 1.5 0 0 1 21 5.5v19"), Shape("M4.5 24.5h19"), Shape("M16.2 15a1.3 1.3 0 1 0 2.6 0a1.3 1.3 0 1 0 -2.6 0z", filled = true)),
        "drop" to listOf(Shape("M14 3.5C14 3.5 6.5 12 6.5 16.5a7.5 7.5 0 0 0 15 0C21.5 12 14 3.5 14 3.5z")),
        "fan" to listOf(Shape("M11.8 14a2.2 2.2 0 1 0 4.4 0a2.2 2.2 0 1 0 -4.4 0z"), Shape("M14 11.6C14.6 7.5 12.5 3.8 9.6 4.6C7.2 5.3 8 9.6 12 12.4"), Shape("M16.1 15.2C19.3 17.8 23.6 17.8 24.3 14.9C24.9 12.5 20.8 11 16.4 13.1"), Shape("M11.9 15.2C8.1 16.7 5.9 20.4 8.1 22.5C9.9 24.2 13.2 21.4 13.6 16.5")),
        "garage" to listOf(Shape("M3.5 12L14 4.5 24.5 12"), Shape("M6 10.5V24M22 10.5V24"), Shape("M9.5 24v-9h9v9"), Shape("M9.5 18h9M9.5 21h9")),
        "home" to listOf(Shape("M4 13L14 4.5 24 13"), Shape("M7 11v12.5h14V11"), Shape("M12 23.5V17h4v6.5")),
        "humidity" to listOf(Shape("M8 23c-2-3 2-5 0-8s2-5 0-8"), Shape("M14 23c-2-3 2-5 0-8s2-5 0-8"), Shape("M20 23c-2-3 2-5 0-8s2-5 0-8")),
        "light" to listOf(Shape("M10 17.5C7.5 15.9 6 13.3 6 10.5a8 8 0 0 1 16 0c0 2.8-1.5 5.4-4 7V20h-8z"), Shape("M11 23.5h6")),
        "lock" to listOf(Shape("M8 12.5h12a2.5 2.5 0 0 1 2.5 2.5v6.5A2.5 2.5 0 0 1 20 24H8a2.5 2.5 0 0 1-2.5-2.5V15A2.5 2.5 0 0 1 8 12.5z"), Shape("M9.5 12.5V9a4.5 4.5 0 0 1 9 0v3.5"), Shape("M12.6 18.3a1.4 1.4 0 1 0 2.8 0a1.4 1.4 0 1 0 -2.8 0z", filled = true)),
        "motion" to listOf(Shape("M12 14a2 2 0 1 0 4 0a2 2 0 1 0 -4 0z", filled = true), Shape("M9.5 9.5a6.4 6.4 0 0 0 0 9M18.5 9.5a6.4 6.4 0 0 1 0 9"), Shape("M6 6a11.3 11.3 0 0 0 0 16M22 6a11.3 11.3 0 0 1 0 16")),
        "person" to listOf(Shape("M10 9a4 4 0 1 0 8 0a4 4 0 1 0 -8 0z"), Shape("M5.5 24a8.5 8.5 0 0 1 17 0")),
        "plug" to listOf(Shape("M7.5 4h13A3.5 3.5 0 0 1 24 7.5v13a3.5 3.5 0 0 1-3.5 3.5h-13A3.5 3.5 0 0 1 4 20.5v-13A3.5 3.5 0 0 1 7.5 4z"), Shape("M11 10.5v3.5M17 10.5v3.5"), Shape("M12.5 18.5h3")),
        "scene" to listOf(Shape("M14 4c.8 5.5 4.5 9.2 10 10-5.5.8-9.2 4.5-10 10-.8-5.5-4.5-9.2-10-10 5.5-.8 9.2-4.5 10-10z")),
        "script" to listOf(Shape("M5 7h10M5 12h7M5 17h5"), Shape("M16 13l7 4.5-7 4.5z")),
        "shield" to listOf(Shape("M14 3.5l8.5 3v7c0 5-3.5 9-8.5 11-5-2-8.5-6-8.5-11v-7z")),
        "smoke" to listOf(Shape("M4 14a10 10 0 1 0 20 0a10 10 0 1 0 -20 0z"), Shape("M10.4 14a3.6 3.6 0 1 0 7.2 0a3.6 3.6 0 1 0 -7.2 0z"), Shape("M14 6.5v1.5M14 20v1.5M6.5 14H8M20 14h1.5")),
        "speaker" to listOf(Shape("M4 11h5l6-5v16l-6-5H4z"), Shape("M19 10a5 5 0 0 1 0 8")),
        "sun" to listOf(Shape("M9.5 14a4.5 4.5 0 1 0 9 0a4.5 4.5 0 1 0 -9 0z"), Shape("M14 3.5V6M14 22v2.5M3.5 14H6M22 14h2.5M6.6 6.6l1.8 1.8M19.6 19.6l1.8 1.8M6.6 21.4l1.8-1.8M19.6 8.4l1.8-1.8")),
        "thermostat" to listOf(Shape("M14 4a2.5 2.5 0 0 1 2.5 2.5v8.7a4.5 4.5 0 1 1-5 0V6.5A2.5 2.5 0 0 1 14 4z"), Shape("M12.2 19a1.8 1.8 0 1 0 3.6 0a1.8 1.8 0 1 0 -3.6 0z", filled = true), Shape("M19.5 7.5H22M19.5 11H22")),
        "toggle" to listOf(Shape("M9 8h10a6 6 0 0 1 0 12H9A6 6 0 0 1 9 8z"), Shape("M16.4 14a2.6 2.6 0 1 0 5.2 0a2.6 2.6 0 1 0 -5.2 0z", filled = true)),
        "tv" to listOf(Shape("M5.5 5h17A2.5 2.5 0 0 1 25 7.5v10a2.5 2.5 0 0 1-2.5 2.5h-17A2.5 2.5 0 0 1 3 17.5v-10A2.5 2.5 0 0 1 5.5 5z"), Shape("M10 24h8")),
        "vacuum" to listOf(Shape("M4 14a10 10 0 1 0 20 0a10 10 0 1 0 -20 0z"), Shape("M7.2 9.5a8.3 8.3 0 0 1 13.6 0"), Shape("M11.8 15.5a2.2 2.2 0 1 0 4.4 0a2.2 2.2 0 1 0 -4.4 0z")),
        "window" to listOf(Shape("M5 4.5h18v19H5z"), Shape("M14 4.5v19M5 14h18")),
    )

    private val built = ConcurrentHashMap<String, ImageVector>()

    /** Every name [named] knows. */
    val names: Set<String> get() = drawings.keys

    /** The glyph called [name], or null when Lounge draws none by that name. */
    fun named(name: String?): ImageVector? {
        val shapes = drawings[name ?: return null] ?: return null
        return built.getOrPut(name) { vector(name, shapes) }
    }

    /** A plain ring, for a name this Lounge does not draw. */
    val Thing: ImageVector by lazy { vector("thing", listOf(Shape("M8 14a6 6 0 1 0 12 0a6 6 0 1 0 -12 0z"))) }

    private fun vector(name: String, shapes: List<Shape>): ImageVector =
        ImageVector.Builder(name = name, defaultWidth = 28.dp, defaultHeight = 28.dp, viewportWidth = 28f, viewportHeight = 28f).apply {
            for (shape in shapes) {
                if (shape.filled) {
                    addPath(pathData = addPathNodes(shape.d), fill = SolidColor(Color.White))
                } else {
                    addPath(
                        pathData = addPathNodes(shape.d),
                        stroke = SolidColor(Color.White),
                        strokeLineWidth = 2.2f,
                        strokeLineCap = StrokeCap.Round,
                        strokeLineJoin = StrokeJoin.Round,
                    )
                }
            }
        }.build()
}
