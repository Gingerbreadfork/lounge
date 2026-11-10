package tv.lounge.update

import android.content.pm.PackageInstaller
import java.io.File
import java.util.zip.ZipFile
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

/**
 * A baseline profile published beside a release's APK, for Android [minSdk] and later. Installed
 * with the APK, it has Android compile the app there and then.
 */
data class Profile(
    val name: String,
    val url: String,
    val size: Long,
    /** The profile's SHA-256 when the release carries it; SHA256SUMS lists it otherwise. */
    val sha256: String?,
    val minSdk: Int,
)

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
    val profiles: List<Profile> = emptyList(),
) {
    /** The profile for a device on Android [sdk]: the one made for the newest Android not above it. */
    fun profileFor(sdk: Int): Profile? = profiles.filter { it.minSdk <= sdk }.maxByOrNull { it.minSdk }

    /** The first few lines of the notes, without markdown marks, for a caption. */
    fun summary(lines: Int = 3): String = notes
        .lineSequence()
        .map { it.trim().trimStart('-', '*', '#', ' ').replace("**", "") }
        .filter { it.isNotEmpty() }
        .take(lines)
        .joinToString("\n")

    companion object {
        /** The release from GitHub's `releases/latest` answer, or null when it has no APK or names an address that is not https. */
        fun parse(json: String, apkPrefix: String = "Lounge-"): Release? {
            val o = JSONObject(json)
            if (o.optBoolean("draft") || o.optBoolean("prerelease")) return null
            val tag = o.optString("tag_name")
            val version = Version.parse(tag) ?: return null
            val assets = o.optJSONArray("assets") ?: return null
            var apk: JSONObject? = null
            var sums: JSONObject? = null
            val others = ArrayList<JSONObject>()
            for (i in 0 until assets.length()) {
                val a = assets.getJSONObject(i)
                val name = a.optString("name")
                when {
                    name.startsWith(apkPrefix) && name.endsWith(".apk") && '/' !in name -> apk = a
                    name == "SHA256SUMS" -> sums = a
                    else -> others += a
                }
            }
            val asset = apk ?: return null
            val digest = asset.optString("digest").takeIf { it.startsWith("sha256:") }?.removePrefix("sha256:")?.let(::sha256OrNull)
            val apkUrl = asset.getString("browser_download_url")
            val sumsUrl = sums?.optString("browser_download_url")?.takeIf { it.isNotEmpty() }
            if (!isHttps(apkUrl) || (sumsUrl != null && !isHttps(sumsUrl))) return null
            return Release(
                version = version,
                tag = tag,
                apkName = asset.getString("name"),
                apkUrl = apkUrl,
                apkSize = asset.optLong("size"),
                sha256 = digest,
                sumsUrl = sumsUrl,
                notes = o.optString("body"),
                pageUrl = o.optString("html_url"),
                profiles = others.mapNotNull { profile(it, asset.getString("name").removeSuffix(".apk")) },
            )
        }

        /**
         * The profile in asset [a] when it is named for the APK [stem], as `stem.api31.dm` is for
         * Android 12 and later, and is small and fetched over https. Anything else is no profile.
         */
        private fun profile(a: JSONObject, stem: String): Profile? {
            val name = a.optString("name")
            val minSdk = Regex("\\.api(\\d{1,3})\\.dm").matchEntire(name.removePrefix(stem))?.groupValues?.get(1)?.toInt() ?: return null
            val url = a.optString("browser_download_url")
            val size = a.optLong("size")
            if (!name.startsWith(stem) || !isHttps(url) || size <= 0 || size > MAX_PROFILE_BYTES) return null
            val digest = a.optString("digest").takeIf { it.startsWith("sha256:") }?.removePrefix("sha256:")?.let(::sha256OrNull)
            return Profile(name, url, size, digest, minSdk)
        }

        /** A profile is a few kilobytes; one far larger is not downloaded. */
        const val MAX_PROFILE_BYTES = 4L * 1024 * 1024

        private fun isHttps(url: String): Boolean = url.startsWith("https://", ignoreCase = true)

        private val SHA256_HEX = Regex("[0-9a-f]{64}")

        /** [text] in lower case when it is a SHA-256 written as hex, else null. */
        private fun sha256OrNull(text: String): String? = text.lowercase().takeIf { SHA256_HEX.matches(it) }

        /** The hash listed for [fileName] in a `sha256sum` style file, when it is a well-formed SHA-256. */
        fun hashFromSums(text: String, fileName: String): String? = text
            .lineSequence()
            .map { it.trim().split(Regex("\\s+"), limit = 2) }
            .firstOrNull { it.size == 2 && it[1].trimStart('*') == fileName }
            ?.get(0)
            ?.let(::sha256OrNull)
    }
}

/**
 * Why an APK may not replace this build, or null when it may. It has to be newer than what is
 * installed, and the version its release names; otherwise the same release would look new again
 * after every install.
 */
fun versionProblem(release: Version, apkVersionName: String?, apkVersionCode: Long, installedVersionCode: Long): String? = when {
    apkVersionCode <= installedVersionCode -> "The update is no newer than this copy of Lounge"
    Version.parse(apkVersionName.orEmpty()) != release -> "The release says $release, but the app in it says ${apkVersionName ?: "nothing"}"
    else -> null
}

/**
 * How often one release has failed here. After [MAX_TRIES] failures, or one that cannot change
 * such as the wrong signer, it is not tried again on its own; a newer release starts afresh.
 */
data class Failures(val tag: String?, val tries: Int) {
    fun after(tag: String, final: Boolean): Failures = Failures(tag, if (final) MAX_TRIES else (if (tag == this.tag) tries else 0) + 1)

    fun gaveUpOn(tag: String): Boolean = tag == this.tag && tries >= MAX_TRIES

    companion object {
        const val MAX_TRIES = 3
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

/**
 * Whether [file] is a profile archive with a profile in it and nothing else. Android would also act
 * on other entries, such as verification data, which no signature covers here.
 */
internal fun holdsOnlyAProfile(file: File): Boolean = runCatching {
    ZipFile(file).use { zip ->
        val names = zip.entries().asSequence().map { it.name }.toList()
        "primary.prof" in names && names.all { it == "primary.prof" || it == "primary.profm" }
    }
}.getOrDefault(false)

/** What a failed install says about the profile it carried. */
internal enum class ProfileBlame {
    /** Nothing: it carried none, or failed for a reason of its own. */
    None,
    /** It may be why, so the update goes in again without it. */
    Possible,
    /** Android named it: as [Possible], and once that install lands this Android is known to refuse profiles. */
    Named,
}

internal fun profileBlame(carriedProfile: Boolean, status: Int, message: String?): ProfileBlame = when {
    !carriedProfile -> ProfileBlame.None
    status != PackageInstaller.STATUS_FAILURE && status != PackageInstaller.STATUS_FAILURE_INVALID -> ProfileBlame.None
    message?.contains("DEX_METADATA") == true -> ProfileBlame.Named
    else -> ProfileBlame.Possible
}

/** What becomes, at a start, of the release that was installed again without its profile. */
internal enum class Suspicion {
    /** That install has not landed yet, so the release still goes in without a profile. */
    Keep,
    /** It is over and says nothing about this Android. */
    Drop,
    /** It landed after Android named the profile: this Android refuses them. */
    Refused,
}

internal fun suspicion(suspectTag: String, named: Boolean, runningVersion: String): Suspicion {
    val suspect = Version.parse(suspectTag)
    val running = Version.parse(runningVersion)
    return when {
        suspect == null || running == null -> Suspicion.Drop
        running < suspect -> Suspicion.Keep
        running == suspect && named -> Suspicion.Refused
        else -> Suspicion.Drop
    }
}

/**
 * A profile staged for a Power Setup install: where it is, what a copy of it must hash to, and the
 * `pm` command that opens the session it goes in through.
 */
internal class StagedProfile(val source: String, val sha256: String, val sha1: String, val session: String, val copy: String = "/data/local/tmp/lounge-update.dm")
