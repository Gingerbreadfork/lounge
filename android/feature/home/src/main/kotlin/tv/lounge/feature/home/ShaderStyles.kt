package tv.lounge.feature.home

import android.content.Context
import android.os.Build
import org.json.JSONArray
import org.json.JSONObject
import tv.lounge.core.CoreClient

/** A value for one of a program's own uniforms: floats, or a colour as ARGB. */
class ShaderValue(val name: String, val floats: FloatArray = FloatArray(0), val color: Int? = null)

/** An abstract scene: an AGSL program, values for its uniforms, and where it came from. */
data class ShaderStyle(
    val id: String,
    val name: String,
    val description: String?,
    val source: String,
    val values: List<ShaderValue>,
    val origin: String,
    /** Fine detail that blurs at half size, so it starts at full size where the TV allows. */
    val fine: Boolean = false,
)

/** One `uniform` a program declares. */
data class UniformDecl(val name: String, val type: String, val components: Int, val color: Boolean, val count: Int = 1) {
    val size: Int get() = components * count
    val isInt: Boolean get() = type.startsWith("int")
}

/** Reading what a program asks for, without compiling it. */
object ShaderSource {
    const val MAX_BYTES = 32 * 1024

    /** The uniforms Lounge sets on every program that declares them. */
    val PROVIDED = setOf("resolution", "time", "hour", "velvet", "horizon", "curtain", "houselight", "ember", "signal", "dust", "noise")
    /** The one input a program may declare: Lounge's tileable noise texture. */
    const val NOISE = "noise"
    private val noiseInput = Regex("uniform\\s+shader\\s+noise\\s*;")

    /** Whether the program samples the noise texture. */
    fun wantsNoise(source: String): Boolean = noiseInput.containsMatchIn(source.replace(comments, " "))

    private val comments = Regex("//[^\\n]*|/\\*.*?\\*/", RegexOption.DOT_MATCHES_ALL)
    private val uniform = Regex("""(layout\s*\(\s*color\s*\)\s*)?uniform\s+(\w+)\s+(\w+)\s*(?:\[\s*(\d+)\s*])?\s*;""")
    private val entry = Regex("""\bmain\s*\(""")

    fun uniforms(source: String): List<UniformDecl> = uniform.findAll(source.replace(comments, " ")).mapNotNull { m ->
        val type = m.groupValues[2]
        val components = componentsOf(type) ?: return@mapNotNull null
        UniformDecl(m.groupValues[3], type, components, m.groupValues[1].isNotEmpty(), m.groupValues[4].toIntOrNull() ?: 1)
    }.toList()

    /** Why a program cannot run as a screensaver, or null when it can be tried. */
    fun problem(source: String): String? {
        if (source.length > MAX_BYTES) return "the program is larger than ${MAX_BYTES / 1024} KB"
        val code = source.replace(comments, " ")
        if (!entry.containsMatchIn(code)) return "the program has no main function"
        uniform.findAll(code).forEach { m ->
            if (m.groupValues[2] == "shader" && m.groupValues[3] == NOISE) return@forEach
            if (componentsOf(m.groupValues[2]) == null) return "uniform ${m.groupValues[3]} is a ${m.groupValues[2]}, which a scene cannot be given"
        }
        return null
    }

    private fun componentsOf(type: String): Int? = when (type) {
        "float", "half", "int" -> 1
        "float2", "half2", "int2" -> 2
        "float3", "half3", "int3" -> 3
        "float4", "half4", "int4", "float2x2", "half2x2" -> 4
        "float3x3", "half3x3" -> 9
        "float4x4", "half4x4" -> 16
        else -> null
    }

    /** "#RRGGBB" or "#AARRGGBB" as ARGB. */
    fun parseColor(text: String): Int? {
        val hex = text.trim().removePrefix("#")
        if (hex.length != 6 && hex.length != 8) return null
        val v = hex.toLongOrNull(16) ?: return null
        return (if (hex.length == 6) v or 0xFF000000L else v).toInt()
    }

    /** Uniform values in the plugin wire format: a number, a list of numbers, or a colour string. */
    fun parseValues(o: JSONObject?): List<ShaderValue> {
        if (o == null) return emptyList()
        return o.keys().asSequence().mapNotNull { k ->
            when (val v = o.get(k)) {
                is Number -> ShaderValue(k, floatArrayOf(v.toFloat()))
                is JSONArray -> ShaderValue(k, FloatArray(v.length()) { v.getDouble(it).toFloat() })
                is String -> parseColor(v)?.let { ShaderValue(k, color = it) }
                else -> null
            }
        }.toList()
    }
}

/** Every abstract scene this TV can show, as a screensaver or a live background: Lounge's own, those from plugins, and local files. */
object ShaderStyles {
    const val PICTURES = "pictures"

    fun all(context: Context, core: CoreClient): List<ShaderStyle> =
        if (Build.VERSION.SDK_INT < 33) emptyList() else bundled(context) + fromPlugins(core) + local(context)

    fun find(context: Context, core: CoreClient, id: String): ShaderStyle? = when {
        Build.VERSION.SDK_INT < 33 -> null
        id.startsWith("lounge:") -> bundled(context).firstOrNull { it.id == id }
        id.startsWith("plugin:") -> fromPlugins(core).firstOrNull { it.id == id }
        id.startsWith("local:") -> local(context).firstOrNull { it.id == id }
        else -> null
    }

    fun bundled(context: Context): List<ShaderStyle> = runCatching {
        val index = JSONArray(context.assets.open("screensavers/index.json").bufferedReader().use { it.readText() })
        (0 until index.length()).mapNotNull { i ->
            val o = index.getJSONObject(i)
            val source = runCatching { context.assets.open("screensavers/" + o.getString("file")).bufferedReader().use { it.readText() } }.getOrNull()
                ?: return@mapNotNull null
            ShaderStyle(
                id = "lounge:" + o.getString("id"),
                name = o.getString("name"),
                description = o.optString("description").ifEmpty { null },
                source = source,
                values = ShaderSource.parseValues(o.optJSONObject("uniforms")),
                origin = "Lounge",
                fine = o.optBoolean("fine"),
            )
        }
    }.getOrDefault(emptyList())

    fun fromPlugins(core: CoreClient): List<ShaderStyle> = core.screensavers().map { s ->
        ShaderStyle(
            id = s.id,
            name = s.name,
            description = s.description,
            source = s.source,
            values = s.uniforms.map { u -> ShaderValue(u.name, u.floats.toFloatArray(), u.colorArgb?.toInt()) },
            origin = s.pluginName,
        )
    }

    /** Programs in the app's `screensavers` folder, for trying one on the TV before packaging it. */
    fun local(context: Context): List<ShaderStyle> {
        val dir = context.getExternalFilesDir("screensavers") ?: return emptyList()
        return dir.listFiles { f -> f.isFile && f.name.endsWith(".agsl") }.orEmpty().sortedBy { it.name }.mapNotNull { f ->
            val source = runCatching { f.readText() }.getOrNull() ?: return@mapNotNull null
            ShaderStyle(
                id = "local:" + f.name,
                name = f.nameWithoutExtension.replace('_', ' ').replace('-', ' ').replaceFirstChar { it.uppercase() },
                description = "From this TV's screensavers folder",
                source = source,
                values = emptyList(),
                origin = "This TV",
            )
        }
    }
}
