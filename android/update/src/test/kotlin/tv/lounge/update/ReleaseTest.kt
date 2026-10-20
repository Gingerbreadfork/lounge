package tv.lounge.update

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ReleaseTest {
    @Test
    fun versionsParseAndOrder() {
        assertEquals(Version(0, 1, 2), Version.parse("v0.1.2"))
        assertEquals(Version(1, 0, 0, "beta.1"), Version.parse("1.0.0-beta.1"))
        assertEquals(Version(2, 0, 0), Version.parse("2"))
        assertNull(Version.parse("latest"))
        assertNull(Version.parse("1.x"))
        assertTrue(Version.parse("0.1.2")!! > Version.parse("0.1.1")!!)
        assertTrue(Version.parse("0.2.0")!! > Version.parse("0.1.9")!!)
        assertTrue(Version.parse("1.0.0-beta.1")!! < Version.parse("1.0.0")!!)
        assertEquals(0, Version.parse("v0.1.1")!!.compareTo(Version.parse("0.1.1")!!))
    }

    @Test
    fun theLatestReleaseIsRead() {
        val release = Release.parse(LATEST)!!
        assertEquals("v0.1.2", release.tag)
        assertEquals(Version(0, 1, 2), release.version)
        assertEquals("Lounge-0.1.2.apk", release.apkName)
        assertEquals("https://github.com/x/lounge/releases/download/v0.1.2/Lounge-0.1.2.apk", release.apkUrl)
        assertEquals(19_813_655L, release.apkSize)
        assertEquals("ab".repeat(32), release.sha256)
        assertEquals("https://github.com/x/lounge/releases/download/v0.1.2/SHA256SUMS", release.sumsUrl)
        assertEquals("Nine new plugins.\nPlex and Emby.", release.summary())
    }

    @Test
    fun prereleasesAndReleasesWithoutAnApkAreSkipped() {
        assertNull(Release.parse(LATEST.replace("\"prerelease\": false", "\"prerelease\": true")))
        assertNull(Release.parse(LATEST.replace("Lounge-0.1.2.apk", "notes.txt")))
    }

    @Test
    fun sumsFilesAreRead() {
        val sums = "cafe  Lounge-0.1.2.apk\nbeef *SHA256SUMS\n"
        assertEquals("cafe", Release.hashFromSums(sums, "Lounge-0.1.2.apk"))
        assertEquals("beef", Release.hashFromSums(sums, "SHA256SUMS"))
        assertNull(Release.hashFromSums(sums, "other.apk"))
    }

    @Test
    fun installPathFollowsWhatTheDeviceAllows() {
        assertEquals(InstallPath.Silent, installPath(sdk = 34, selfIsInstaller = true, powerReady = false, canRequestInstalls = true))
        assertEquals(InstallPath.PowerSetup, installPath(sdk = 34, selfIsInstaller = false, powerReady = true, canRequestInstalls = false))
        assertEquals(InstallPath.PowerSetup, installPath(sdk = 30, selfIsInstaller = true, powerReady = true, canRequestInstalls = true))
        assertEquals(InstallPath.NeedsPermission, installPath(sdk = 34, selfIsInstaller = false, powerReady = false, canRequestInstalls = false))
        assertEquals(InstallPath.Confirm, installPath(sdk = 30, selfIsInstaller = true, powerReady = false, canRequestInstalls = true))
    }

    @Test
    fun anUpdateMustCarryTheInstalledSigners() {
        assertTrue(UpdateManager.signedLike(setOf("aa"), setOf("aa")))
        // A rotated key lists the one it replaced in its history.
        assertTrue(UpdateManager.signedLike(setOf("aa"), setOf("bb", "aa")))
        assertFalse(UpdateManager.signedLike(setOf("aa"), setOf("bb")))
        assertFalse(UpdateManager.signedLike(setOf("aa", "cc"), setOf("aa")))
        assertFalse(UpdateManager.signedLike(emptySet(), setOf("aa")))
    }

    private companion object {
        val LATEST = """
            {
              "tag_name": "v0.1.2",
              "name": "Lounge 0.1.2",
              "draft": false,
              "prerelease": false,
              "html_url": "https://github.com/x/lounge/releases/tag/v0.1.2",
              "body": "Nine new plugins.\n\n- **Plex** and **Emby**.\n",
              "assets": [
                {"name": "SHA256SUMS", "size": 83, "browser_download_url": "https://github.com/x/lounge/releases/download/v0.1.2/SHA256SUMS"},
                {"name": "Lounge-0.1.2.apk", "size": 19813655, "digest": "sha256:${"ab".repeat(32)}", "browser_download_url": "https://github.com/x/lounge/releases/download/v0.1.2/Lounge-0.1.2.apk"}
              ]
            }
        """.trimIndent()
    }
}
