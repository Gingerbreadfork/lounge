package tv.lounge.feature.home

import androidx.compose.ui.unit.Density
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
}
