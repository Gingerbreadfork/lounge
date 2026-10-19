package tv.lounge.update

import org.json.JSONObject

/** A version like 0.1.2 or 1.0.0-beta.1; a pre-release sorts below its release. */
data class Version(val major: Int, val minor: Int, val patch: Int, val pre: String? = null) : Comparable<Version> {
    override fun compareTo(other: Version): Int {
        val byNumber = compareValuesBy(this, other, { it.major }, { it.minor }, { it.patch })
        if (byNumber != 0) return byNumber
        return when {
            pre == other.pre -> 0
            pre == null -> 1
            other.pre == null -> -1
            else -> pre.compareTo(other.pre)
        }
    }

    override fun toString(): String = "$major.$minor.$patch" + (pre?.let { "-$it" } ?: "")

    companion object {
        /** Reads "v0.1.2", "0.1.2" or "0.1.2-beta.1"; anything else is null. */
        fun parse(text: String): Version? {
            val raw = text.trim().removePrefix("v").removePrefix("V")
            val (numbers, pre) = raw.split('-', limit = 2).let { it[0] to it.getOrNull(1) }
            val parts = numbers.split('.')
            if (parts.isEmpty() || parts.size > 3 || parts.any { it.isEmpty() || !it.all(Char::isDigit) }) return null
            return Version(parts[0].toInt(), parts.getOrNull(1)?.toInt() ?: 0, parts.getOrNull(2)?.toInt() ?: 0, pre?.takeIf { it.isNotEmpty() })
        }
    }
}

/** One published release with the APK to fetch. */
data class Release(
    val version: Version,
    val tag: String,
    val apkName: String,
    val apkUrl: String,
    val apkSize: Long,
    /** The APK's SHA-256 when the release carries it, else the address of the SHA256SUMS file. */
    val sha256: String?,
    val sumsUrl: String?,
    val notes: String,
    val pageUrl: String,
) {
    /** The first few lines of the notes, without markdown marks, for a caption. */
    fun summary(lines: Int = 3): String = notes
        .lineSequence()
        .map { it.trim().trimStart('-', '*', '#', ' ').replace("**", "") }
        .filter { it.isNotEmpty() }
        .take(lines)
        .joinToString("\n")

    companion object {
        /** The release from GitHub's `releases/latest` answer, or null when it has no APK. */
        fun parse(json: String, apkPrefix: String = "Lounge-"): Release? {
            val o = JSONObject(json)
            if (o.optBoolean("draft") || o.optBoolean("prerelease")) return null
            val tag = o.optString("tag_name")
            val version = Version.parse(tag) ?: return null
            val assets = o.optJSONArray("assets") ?: return null
            var apk: JSONObject? = null
            var sums: JSONObject? = null
            for (i in 0 until assets.length()) {
                val a = assets.getJSONObject(i)
                val name = a.optString("name")
                when {
                    name.startsWith(apkPrefix) && name.endsWith(".apk") -> apk = a
                    name == "SHA256SUMS" -> sums = a
                }
            }
            val asset = apk ?: return null
            val digest = asset.optString("digest").takeIf { it.startsWith("sha256:") }?.removePrefix("sha256:")?.lowercase()
            return Release(
                version = version,
                tag = tag,
                apkName = asset.getString("name"),
                apkUrl = asset.getString("browser_download_url"),
                apkSize = asset.optLong("size"),
                sha256 = digest,
                sumsUrl = sums?.optString("browser_download_url")?.takeIf { it.isNotEmpty() },
                notes = o.optString("body"),
                pageUrl = o.optString("html_url"),
            )
        }

        /** The hash listed for [fileName] in a `sha256sum` style file. */
        fun hashFromSums(text: String, fileName: String): String? = text
            .lineSequence()
            .map { it.trim().split(Regex("\\s+"), limit = 2) }
            .firstOrNull { it.size == 2 && it[1].trimStart('*') == fileName }
            ?.get(0)
            ?.lowercase()
    }
}

/** How a downloaded update can be installed on this device. */
enum class InstallPath {
    /** No prompt: Lounge installed the current build and Android trusts it to update itself. */
    Silent,
    /** No prompt: Power Setup runs `pm install`. */
    PowerSetup,
    /** Android asks the person to confirm. */
    Confirm,
    /** Lounge is not allowed to install apps yet. */
    NeedsPermission,
}

fun installPath(sdk: Int, selfIsInstaller: Boolean, powerReady: Boolean, canRequestInstalls: Boolean): InstallPath = when {
    sdk >= 31 && selfIsInstaller && canRequestInstalls -> InstallPath.Silent
    powerReady -> InstallPath.PowerSetup
    !canRequestInstalls -> InstallPath.NeedsPermission
    else -> InstallPath.Confirm
}
