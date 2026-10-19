package tv.lounge.feature.home

import org.json.JSONObject
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ShaderSourceTest {
    private val program = """
        // uniform float commented;
        uniform float2 resolution;
        uniform float time;
        layout(color) uniform half4 velvet;
        layout( color ) uniform half4 smoke;
        uniform float3 points[4];
        uniform int steps;
        /* uniform float hidden; */
        half4 main(float2 coord) { return half4(1); }
    """.trimIndent()

    @Test
    fun declarationsAreRead() {
        val u = ShaderSource.uniforms(program).associateBy { it.name }
        assertEquals(setOf("resolution", "time", "velvet", "smoke", "points", "steps"), u.keys)
        assertEquals(2, u.getValue("resolution").size)
        assertTrue(u.getValue("velvet").color)
        assertTrue(u.getValue("smoke").color)
        assertEquals(12, u.getValue("points").size)
        assertTrue(u.getValue("steps").isInt)
    }

    @Test
    fun problemsAreNamed() {
        assertNull(ShaderSource.problem(program))
        assertEquals("the program has no main function", ShaderSource.problem("uniform float time;"))
        assertTrue(ShaderSource.problem("uniform shader image; half4 main(float2 p) { return image.eval(p); }")!!.contains("image"))
        assertTrue(ShaderSource.problem("x".repeat(ShaderSource.MAX_BYTES + 1))!!.contains("larger"))
    }

    @Test
    fun valuesUseThePluginFormat() {
        val values = ShaderSource.parseValues(JSONObject("""{"speed": 0.5, "drift": [0.1, 0.2], "smoke": "#C8B8A0", "bad": "blue"}""")).associateBy { it.name }
        assertEquals(setOf("speed", "drift", "smoke"), values.keys)
        assertArrayEquals(floatArrayOf(0.5f), values.getValue("speed").floats, 0f)
        assertArrayEquals(floatArrayOf(0.1f, 0.2f), values.getValue("drift").floats, 0f)
        assertEquals(0xFFC8B8A0.toInt(), values.getValue("smoke").color)
        assertEquals(0x80FFA86B.toInt(), ShaderSource.parseColor("#80FFA86B"))
        assertNull(ShaderSource.parseColor("#FFA86"))
    }
}
