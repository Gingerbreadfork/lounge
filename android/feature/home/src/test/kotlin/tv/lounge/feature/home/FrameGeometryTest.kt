package tv.lounge.feature.home

import androidx.compose.ui.unit.Density
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Outline
import androidx.compose.ui.unit.LayoutDirection
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class FrameGeometryTest {
    @Test
    fun pictureTopIsInRangeAtEveryWidth() {
        for (width in 1280..3840) {
            val density = Density(width / 1920f)
            for (focalY in listOf(0f, 0.3f, 0.5f, 1f)) {
                val g = FrameGeometry(density, focalY)
                for (step in 0..100) {
                    val p = step / 100f
                    val top = g.pictureTop(p)
                    assertTrue("width $width focal $focalY p $p top $top", top <= 0f)
                }
            }
        }
    }

    @Test
    fun theBandShutsFromBelowToNothing() {
        val g = FrameGeometry(Density(1f), 0.5f)
        assertEquals(200f, g.height(0f, 0f), 0f)
        assertEquals(100f, g.height(0f, 0.5f), 0f)
        assertEquals(0f, g.height(0f, 1f), 0f)
        // Part-way through the lowering it shuts what there is of the frame.
        assertEquals(g.height(0.5f) / 2f, g.height(0.5f, 0.5f), 0f)
    }

    @Test
    fun aFrameNearlyShutKeepsRoundEnds() {
        val density = Density(1f)
        val g = FrameGeometry(density, 0.5f)
        fun outline(closing: Float) = g.frameShape(0f, closing = closing).createOutline(Size(1920f, 630f), LayoutDirection.Ltr, density) as Outline.Rounded
        assertEquals(20f, outline(0f).roundRect.topLeftCornerRadius.y, 0f)
        val line = outline(0.95f).roundRect
        assertEquals(10f, line.height, 0.001f)
        assertEquals(5f, line.topLeftCornerRadius.y, 0.001f)
        assertEquals(0f, line.top, 0f)
    }
}
