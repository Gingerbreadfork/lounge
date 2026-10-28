package tv.lounge.design

import android.graphics.Bitmap
import android.graphics.BitmapShader
import android.graphics.Shader
import kotlin.math.roundToInt

/**
 * Three smooth noises that tile, in the channels of one 256-square bitmap, for a scene to sample
 * instead of computing noise for every pixel: a texture fetch costs a TV's GPU far less than the
 * arithmetic. Sixteen lattice cells span the tile, so `noise.eval(p * 16.0)` reads one cell per
 * unit of `p`.
 */
object NoiseTexture {
    const val SIZE = 256
    private const val CELLS = 16

    val bitmap: Bitmap by lazy { make() }
    val shader: BitmapShader by lazy {
        BitmapShader(bitmap, Shader.TileMode.REPEAT, Shader.TileMode.REPEAT).apply { filterMode = BitmapShader.FILTER_MODE_LINEAR }
    }

    private fun make(): Bitmap {
        val random = java.util.Random(11)
        val lattice = Array(3) { FloatArray(CELLS * CELLS) { random.nextFloat() } }
        val px = IntArray(SIZE * SIZE)
        val step = CELLS / SIZE.toFloat()
        for (y in 0 until SIZE) {
            val fy = y * step
            val iy = fy.toInt()
            val ty = smooth(fy - iy)
            for (x in 0 until SIZE) {
                val fx = x * step
                val ix = fx.toInt()
                val tx = smooth(fx - ix)
                var argb = 0xFF shl 24
                for (c in 0..2) {
                    val l = lattice[c]
                    fun at(i: Int, j: Int) = l[(j % CELLS) * CELLS + (i % CELLS)]
                    val v = lerp(lerp(at(ix, iy), at(ix + 1, iy), tx), lerp(at(ix, iy + 1), at(ix + 1, iy + 1), tx), ty)
                    argb = argb or ((v * 255f).roundToInt().coerceIn(0, 255) shl (16 - 8 * c))
                }
                px[y * SIZE + x] = argb
            }
        }
        return Bitmap.createBitmap(px, SIZE, SIZE, Bitmap.Config.ARGB_8888)
    }

    private fun smooth(t: Float) = t * t * (3f - 2f * t)
    private fun lerp(a: Float, b: Float, t: Float) = a + (b - a) * t
}
