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
        assertNull(ShaderSource.problem("uniform shader noise; half4 main(float2 p) { return noise.eval(p); }"))
        assertTrue(ShaderSource.wantsNoise("uniform shader noise;\nhalf4 main(float2 p) { return noise.eval(p); }"))
        assertTrue(!ShaderSource.wantsNoise("// uniform shader noise;\nhalf4 main(float2 p) { return half4(1); }"))
        assertTrue(ShaderSource.problem("x".repeat(ShaderSource.MAX_BYTES + 1))!!.contains("larger"))
    }

    @Test
    fun everyBundledSceneCanRun() {
        val dir = java.io.File("src/main/assets/screensavers")
        val index = org.json.JSONArray(java.io.File(dir, "index.json").readText())
        val ids = mutableSetOf<String>()
        for (i in 0 until index.length()) {
            val o = index.getJSONObject(i)
            val id = o.getString("id")
            assertTrue("$id is listed twice", ids.add(id))
            assertTrue("$id needs a name and a description", o.getString("name").isNotBlank() && o.getString("description").isNotBlank())
            val source = java.io.File(dir, o.getString("file")).readText()
            assertNull(id, ShaderSource.problem(source))
            // A bundled scene is given no values, so it may only ask for what Lounge provides.
            val given = o.optJSONObject("uniforms")?.keys()?.asSequence()?.toSet().orEmpty()
            val missing = ShaderSource.uniforms(source).map { it.name }.filter { it !in ShaderSource.PROVIDED && it !in given }
            assertTrue("$id declares $missing", missing.isEmpty())
        }
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
