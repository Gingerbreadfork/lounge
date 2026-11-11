package tv.lounge.update

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.security.MessageDigest

class ReleaseTest {
    @Test
    fun theInstallScriptInstallsOnlyACopyThatStillMatches() {
        val dir = kotlin.io.path.createTempDirectory().toFile()
        val apk = File(dir, "it's.apk").apply { writeBytes(ByteArray(4096) { it.toByte() }) }
        val sha256 = hex(MessageDigest.getInstance("SHA-256").digest(apk.readBytes()))
        val sha1 = hex(MessageDigest.getInstance("SHA-1").digest(apk.readBytes()))
        val copy = File(dir, "copy.apk")
        fun run(sha256: String, sha1: String): Pair<Int, String> {
            val script = UpdateManager.installCommand(apk.path, sha256, sha1, "echo installed; cmp ${apk.path.replace("'", "'\\''").let { "'$it'" }}", copy.path)
            val p = ProcessBuilder("sh", "-c", script).redirectErrorStream(true).start()
            val out = p.inputStream.bufferedReader().readText().trim()
            return p.waitFor() to out
        }
        assertEquals(0 to "installed", run(sha256, sha1))
        val (code, out) = run("0".repeat(64), "0".repeat(40))
        assertEquals(1, code)
        assertEquals("The update changed before it was installed", out)
        assertFalse(copy.exists())
        // A hash is only ever compared, never run.
        val marker = File(dir, "ran")
        assertEquals(1, run("\$(touch ${marker.path})", "\$(touch ${marker.path})").first)
        assertFalse(marker.exists())
        dir.deleteRecursively()
    }

    private fun hex(bytes: ByteArray) = bytes.joinToString("") { "%02x".format(it) }

    /**
     * Runs the install script with a profile against a stand-in `pm` that writes down what it is
     * asked and fails where [failing] says. Gives the exit code and the calls made.
     */
    private fun runWithProfile(failing: String = "", profileSha256: String? = null, created: String = "Success: created install session [42]", profileThere: Boolean = true, reopen: Boolean = false): Pair<Int, List<String>> {
        val dir = kotlin.io.path.createTempDirectory().toFile()
        try {
            val apk = File(dir, "update.apk").apply { writeBytes(ByteArray(4096) { it.toByte() }) }
            val dm = File(dir, "update.dm").apply { writeBytes(ByteArray(512) { (it * 7).toByte() }) }
            val calls = File(dir, "calls")
            fun sha(file: File, algorithm: String) = hex(MessageDigest.getInstance(algorithm).digest(file.readBytes()))
            val profile = StagedProfile(dm.path, profileSha256 ?: sha(dm, "SHA-256"), if (profileSha256 == null) sha(dm, "SHA-1") else "0".repeat(40), "pm install-create -r -i tv.lounge", File(dir, "copy.dm").path)
            val script = UpdateManager.installCommand(apk.path, sha(apk, "SHA-256"), sha(apk, "SHA-1"), "pm install -r -i tv.lounge", File(dir, "copy.apk").path, profile,
                reopen = if (reopen) "{ echo reopen >> '${calls.path}'; }" else null)
            if (!profileThere) dm.delete()
            // Only session 42 exists, so a number read wrongly from pm's answer shows as a failed write.
            val pm = """
                pm() {
                  case "${'$'}1" in
                    install-create) echo "${'$'}1" >> '${calls.path}'; echo '$created' ;;
                    install-write) echo "${'$'}1 ${'$'}2 ${'$'}3" >> '${calls.path}'; [ "${'$'}2" = 42 ] && [ -f "${'$'}4" ] || return 1 ;;
                    install-commit|install-abandon) echo "${'$'}1 ${'$'}2" >> '${calls.path}' ;;
                    install) echo "${'$'}1" >> '${calls.path}' ;;
                  esac
                  case " $failing " in *" ${'$'}1 "*) return 1 ;; esac
                  return 0
                }
            """.trimIndent()
            val p = ProcessBuilder("sh", "-c", "$pm\n$script").redirectErrorStream(true).start()
            p.inputStream.bufferedReader().readText()
            val code = p.waitFor()
            assertFalse(File(dir, "copy.apk").exists())
            assertFalse(File(dir, "copy.dm").exists())
            return code to (if (calls.exists()) calls.readLines() else emptyList())
        } finally {
            dir.deleteRecursively()
        }
    }

    @Test
    fun aProfileGoesInWithTheApkThroughOneSession() {
        assertEquals(0 to listOf("install-create", "install-write 42 lounge.apk", "install-write 42 lounge.dm", "install-commit 42"), runWithProfile())
    }

    @Test
    fun aProfileThatChangedOrIsGoneIsLeftOutAndTheApkStillInstalls() {
        assertEquals(0 to listOf("install"), runWithProfile(profileSha256 = "0".repeat(64)))
        assertEquals(0 to listOf("install"), runWithProfile(profileThere = false))
    }

    @Test
    fun anInstallRefusedWithItsProfileIsTriedAgainWithout() {
        assertEquals(0 to listOf("install-create", "install-write 42 lounge.apk", "install-write 42 lounge.dm", "install-commit 42", "install-abandon 42", "install"), runWithProfile(failing = "install-commit"))
        // No session was opened, so nothing can be written to one and the APK goes in alone.
        val none = "Error: no sessions here"
        assertEquals(0 to listOf("install-create", "install-write $none lounge.apk", "install-abandon $none", "install"), runWithProfile(failing = "install-create", created = none))
        // When the APK will not install either, the script says so.
        assertEquals(1, runWithProfile(failing = "install-commit install").first)
    }

    @Test
    fun loungeOpensAgainOnlyAfterAnInstallThatWentIn() {
        assertEquals(0 to listOf("install-create", "install-write 42 lounge.apk", "install-write 42 lounge.dm", "install-commit 42", "reopen"), runWithProfile(reopen = true))
        assertEquals(0 to listOf("install-create", "install-write 42 lounge.apk", "install-write 42 lounge.dm", "install-commit 42", "install-abandon 42", "install", "reopen"), runWithProfile(failing = "install-commit", reopen = true))
        assertEquals(1 to listOf("install-create", "install-write 42 lounge.apk", "install-write 42 lounge.dm", "install-commit 42", "install-abandon 42", "install"), runWithProfile(failing = "install-commit install", reopen = true))
    }

    @Test
    fun loungeStaysClosedAfterAnInstallThatFailed() {
        val dir = kotlin.io.path.createTempDirectory().toFile()
        try {
            val apk = File(dir, "update.apk").apply { writeBytes(ByteArray(4096) { it.toByte() }) }
            val reopened = File(dir, "reopened")
            fun sha(algorithm: String) = hex(MessageDigest.getInstance(algorithm).digest(apk.readBytes()))
            fun run(install: String) = ProcessBuilder("sh", "-c", UpdateManager.installCommand(apk.path, sha("SHA-256"), sha("SHA-1"), install, File(dir, "copy.apk").path, reopen = "touch '${reopened.path}'")).start().waitFor()
            assertEquals(1, run("false"))
            assertFalse(reopened.exists())
            assertEquals(0, run("true"))
            assertTrue(reopened.exists())
        } finally {
            dir.deleteRecursively()
        }
    }

    @Test
    fun onlyAnInstallSomeoneAskedForLeavesTheSilentPathForPowerSetup() {
        assertTrue(throughPowerSetup(InstallPath.PowerSetup, asked = false, adbReady = false))
        assertTrue(throughPowerSetup(InstallPath.Silent, asked = true, adbReady = true))
        assertFalse(throughPowerSetup(InstallPath.Silent, asked = false, adbReady = true))
        // Shizuku stops what it runs when Lounge goes, so only adb is worth the detour.
        assertFalse(throughPowerSetup(InstallPath.Silent, asked = true, adbReady = false))
    }

    @Test
    fun theInstallScriptFinishesAfterItsConnectionCloses() {
        val dir = kotlin.io.path.createTempDirectory().toFile()
        try {
            val apk = File(dir, "update.apk").apply { writeBytes(ByteArray(4096) { it.toByte() }) }
            val copy = File(dir, "copy.apk")
            val reopened = File(dir, "reopened")
            fun sha(algorithm: String) = hex(MessageDigest.getInstance(algorithm).digest(apk.readBytes()))
            // The install outlasts the connection and then prints into it, as pm does when Lounge has been stopped.
            val script = UpdateManager.installCommand(apk.path, sha("SHA-256"), sha("SHA-1"), "sleep 1; echo installed; true", copy.path, reopen = "touch '${reopened.path}'")
            val pid = File(dir, "pid")
            val p = ProcessBuilder("sh", "-c", "echo \$\$ > '${pid.path}'\n$script").redirectErrorStream(true).start()
            val until = System.currentTimeMillis() + 5_000
            while (pid.length() == 0L && System.currentTimeMillis() < until) Thread.sleep(20)
            p.inputStream.close()
            ProcessBuilder("kill", "-HUP", pid.readText().trim()).start().waitFor()
            assertEquals(0, p.waitFor())
            assertTrue(reopened.exists())
            assertFalse(copy.exists())
        } finally {
            dir.deleteRecursively()
        }
    }

    @Test
    fun onlyAnArchiveWithAProfileAndNothingElseIsAProfile() {
        val dir = kotlin.io.path.createTempDirectory().toFile()
        try {
            fun archive(vararg names: String) = File(dir, names.joinToString("+").ifEmpty { "empty" } + ".dm").also { file ->
                java.util.zip.ZipOutputStream(file.outputStream()).use { zip ->
                    for (name in names) {
                        zip.putNextEntry(java.util.zip.ZipEntry(name))
                        zip.write(byteArrayOf(1, 2, 3))
                        zip.closeEntry()
                    }
                }
            }
            assertTrue(holdsOnlyAProfile(archive("primary.prof")))
            assertTrue(holdsOnlyAProfile(archive("primary.prof", "primary.profm")))
            // Verification data would be taken on trust, so an archive that brings any is no profile.
            assertFalse(holdsOnlyAProfile(archive("primary.prof", "primary.vdex")))
            assertFalse(holdsOnlyAProfile(archive("primary.profm")))
            assertFalse(holdsOnlyAProfile(archive()))
            assertFalse(holdsOnlyAProfile(File(dir, "noise.dm").apply { writeBytes(ByteArray(512) { (it * 7).toByte() }) }))
            assertFalse(holdsOnlyAProfile(File(dir, "missing.dm")))
        } finally {
            dir.deleteRecursively()
        }
    }

    @Test
    fun aFailedInstallBlamesItsProfileOnlyWhenItCouldBeWhy() {
        val failure = android.content.pm.PackageInstaller.STATUS_FAILURE
        val invalid = android.content.pm.PackageInstaller.STATUS_FAILURE_INVALID
        val named = "INSTALL_FAILED_BAD_DEX_METADATA: Failed parse during installPackageLI: Error opening base.dm: Invalid file"
        assertEquals(ProfileBlame.Named, profileBlame(true, invalid, named))
        assertEquals(ProfileBlame.Possible, profileBlame(true, failure, "INSTALL_FAILED_INTERNAL_ERROR"))
        assertEquals(ProfileBlame.Possible, profileBlame(true, invalid, null))
        // It carried none, or the person or the storage is why.
        assertEquals(ProfileBlame.None, profileBlame(false, invalid, named))
        assertEquals(ProfileBlame.None, profileBlame(true, android.content.pm.PackageInstaller.STATUS_FAILURE_ABORTED, "INSTALL_FAILED_ABORTED: User rejected permissions"))
        assertEquals(ProfileBlame.None, profileBlame(true, android.content.pm.PackageInstaller.STATUS_FAILURE_STORAGE, null))
        assertEquals(ProfileBlame.None, profileBlame(true, android.content.pm.PackageInstaller.STATUS_FAILURE_BLOCKED, null))
    }

    @Test
    fun aSuspectedProfileIsSettledOnlyOnceItsReleaseIsRunning() {
        // Still on the older version: the install without the profile has not landed.
        assertEquals(Suspicion.Keep, suspicion("v0.6.1", named = true, runningVersion = "0.6.0"))
        assertEquals(Suspicion.Keep, suspicion("v0.6.1", named = false, runningVersion = "0.6.0"))
        assertEquals(Suspicion.Refused, suspicion("v0.6.1", named = true, runningVersion = "0.6.1"))
        // It landed, but Android never said the profile was why.
        assertEquals(Suspicion.Drop, suspicion("v0.6.1", named = false, runningVersion = "0.6.1"))
        // A later release got in some other way.
        assertEquals(Suspicion.Drop, suspicion("v0.6.1", named = true, runningVersion = "0.6.2"))
        assertEquals(Suspicion.Drop, suspicion("nonsense", named = true, runningVersion = "0.6.1"))
    }

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
    fun anUpdateMustBeNewerAndTheVersionItsReleaseNames() {
        val release = Version(0, 3, 2)
        assertNull(versionProblem(release, "0.3.2", apkVersionCode = 7, installedVersionCode = 6))
        // A tag pushed before the version was bumped: installing it would leave 0.3.2 looking new forever.
        assertTrue(versionProblem(release, "0.3.1", apkVersionCode = 7, installedVersionCode = 6)!!.contains("says 0.3.1"))
        assertTrue(versionProblem(release, "0.3.2", apkVersionCode = 6, installedVersionCode = 6)!!.contains("no newer"))
        assertTrue(versionProblem(release, null, apkVersionCode = 7, installedVersionCode = 6) != null)
    }

    @Test
    fun failuresAreSaidAsSentencesWithWhatHappensNext() {
        fun said(e: Throwable, next: Next) = failureCause(e).let { (cause, settled) -> UpdateState.Failed(cause, next.takeUnless { settled }, null, 0L).message }
        assertEquals("Lounge couldn't reach GitHub. Check the TV's internet connection. Lounge tries again later.", said(java.net.UnknownHostException("api.github.com"), Next.Later))
        assertEquals("GitHub took too long to answer. Lounge tries again later.", said(java.net.SocketTimeoutException("Read timed out"), Next.Later))
        assertEquals("The connection to GitHub dropped. Lounge tries again later.", said(java.io.EOFException(), Next.Later))
        assertEquals("Already a sentence.", said(UpdateProblem("Already a sentence.", settled = true), Next.Later))
        assertFalse(said(IllegalStateException("java.lang.NullPointerException"), Next.Later).contains("java"))
        // Nothing is promised that will not happen: no later try with the checks off or once Lounge has given up.
        assertEquals(Next.Later, nextAfter(auto = true, gaveUp = false))
        assertEquals(Next.WhenAsked, nextAfter(auto = false, gaveUp = false))
        assertEquals(Next.NextRelease, nextAfter(auto = true, gaveUp = true))
        assertEquals(Next.NextRelease, nextAfter(auto = false, gaveUp = true))
        assertEquals("The connection to GitHub dropped. Try again when you are ready.", said(java.io.EOFException(), Next.WhenAsked))
        assertEquals("GitHub didn't send the update. Lounge waits for the next release.", said(UpdateProblem("GitHub didn't send the update."), Next.NextRelease))
    }

    @Test
    fun aFileThatWillNotWriteIsNotBlamedOnTheNetwork() {
        val full = runCatching { onStorage { throw java.io.IOException("write failed: ENOSPC (No space left on device)") } }.exceptionOrNull()!!
        assertEquals(NOT_STORED to false, failureCause(full))
        assertTrue(full.cause is java.io.IOException)
        assertEquals("kept", onStorage { "kept" })
        assertFalse(NOT_STORED.contains("GitHub"))
    }

    @Test
    fun anInstallAndroidWillNeverTakeDoesNotSayTryAgain() {
        for (status in listOf(android.content.pm.PackageInstaller.STATUS_FAILURE_INCOMPATIBLE, android.content.pm.PackageInstaller.STATUS_FAILURE_CONFLICT, android.content.pm.PackageInstaller.STATUS_FAILURE_INVALID)) {
            val problem = installProblem(status)
            assertTrue(installNeverWorks(status) && problem.settled)
            assertFalse(problem.message!!, problem.message!!.contains("Try again", ignoreCase = true))
            assertTrue(problem.message!!.endsWith("Lounge waits for the next release."))
        }
        // One that could go better says what the person can do, or leaves the ending to what happens next.
        assertTrue(installProblem(android.content.pm.PackageInstaller.STATUS_FAILURE_STORAGE).message!!.contains("Free some space"))
        val other = installProblem(android.content.pm.PackageInstaller.STATUS_FAILURE)
        assertEquals("Android couldn't install the update." to false, other.message to other.settled)
    }

    @Test
    fun aReleaseThatKeepsFailingIsLeftAlone() {
        var f = Failures(null, 0)
        repeat(Failures.MAX_TRIES - 1) { f = f.after("v0.3.2", final = false) }
        assertFalse(f.gaveUpOn("v0.3.2"))
        f = f.after("v0.3.2", final = false)
        assertTrue(f.gaveUpOn("v0.3.2"))
        // A newer release starts afresh; one that can never install is left at once.
        assertFalse(f.after("v0.3.3", final = false).gaveUpOn("v0.3.3"))
        assertTrue(Failures(null, 0).after("v0.3.3", final = true).gaveUpOn("v0.3.3"))
        assertFalse(Failures(null, 0).after("v0.3.3", final = true).gaveUpOn("v0.3.4"))
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
    fun aReleasesProfilesAreReadAndTheOneForThisAndroidChosen() {
        val base = "https://github.com/x/lounge/releases/download/v0.1.2"
        fun asset(name: String, url: String = "$base/$name", size: Long = 11_364, digest: String? = null) =
            """{"name": "$name", "size": $size, ${digest?.let { """"digest": "sha256:$it", """ } ?: ""}"browser_download_url": "$url"},"""
        val assets = listOf(
            asset("Lounge-0.1.2.api28.dm"),
            asset("Lounge-0.1.2.api31.dm", digest = "cd".repeat(32)),
            // Not this release's, not https, not a size a profile has, and not named for an Android version.
            asset("Lounge-0.1.1.api31.dm"),
            asset("Lounge-0.1.2.api33.dm", url = "http://example.com/Lounge-0.1.2.api33.dm"),
            asset("Lounge-0.1.2.api34.dm", size = Release.MAX_PROFILE_BYTES + 1),
            asset("Lounge-0.1.2.dm"),
        ).joinToString("\n")
        val release = Release.parse(LATEST.replace("\"assets\": [", "\"assets\": [\n$assets"))!!
        assertEquals(listOf("Lounge-0.1.2.api28.dm", "Lounge-0.1.2.api31.dm"), release.profiles.map { it.name })
        assertEquals("Lounge-0.1.2.apk", release.apkName)
        assertNull(release.profileFor(27))
        assertEquals(28, release.profileFor(28)!!.minSdk)
        assertEquals(28, release.profileFor(30)!!.minSdk)
        assertEquals(31, release.profileFor(31)!!.minSdk)
        assertNull(release.profileFor(30)!!.sha256)
        assertEquals(31, release.profileFor(34)!!.minSdk)
        assertEquals("cd".repeat(32), release.profileFor(34)!!.sha256)
        assertEquals("$base/Lounge-0.1.2.api31.dm", release.profileFor(34)!!.url)
        assertTrue(Release.parse(LATEST)!!.profiles.isEmpty())
    }

    @Test
    fun prereleasesAndReleasesWithoutAnApkAreSkipped() {
        assertNull(Release.parse(LATEST.replace("\"prerelease\": false", "\"prerelease\": true")))
        assertNull(Release.parse(LATEST.replace("Lounge-0.1.2.apk", "notes.txt")))
        assertNull(Release.parse(LATEST.replace("\"name\": \"Lounge-0.1.2.apk\"", "\"name\": \"Lounge-/../../x.apk\"")))
    }

    @Test
    fun aDigestThatIsNotASha256IsIgnored() {
        assertNull(Release.parse(LATEST.replace("sha256:${"ab".repeat(32)}", "sha256:\$(reboot)"))!!.sha256)
        assertEquals("ab".repeat(32), Release.parse(LATEST.replace("sha256:${"ab".repeat(32)}", "sha256:${"AB".repeat(32)}"))!!.sha256)
    }

    @Test
    fun aReleaseNamingAPlainAddressIsRefused() {
        val apk = "https://github.com/x/lounge/releases/download/v0.1.2/Lounge-0.1.2.apk"
        val sums = "https://github.com/x/lounge/releases/download/v0.1.2/SHA256SUMS"
        assertNull(Release.parse(LATEST.replace(apk, apk.replace("https://", "http://"))))
        assertNull(Release.parse(LATEST.replace(sums, sums.replace("https://", "http://"))))
        assertNull(Release.parse(LATEST.replace(apk, "file:///sdcard/Lounge-0.1.2.apk")))
        assertNull(Release.parse(LATEST.replace(apk, "")))
    }

    @Test
    fun sumsFilesAreRead() {
        val cafe = "cafe".repeat(16)
        val beef = "beef".repeat(16)
        val sums = "${cafe.uppercase()}  Lounge-0.1.2.apk\n$beef *SHA256SUMS\n\$(reboot) Lounge-0.1.3.apk\n"
        assertEquals(cafe, Release.hashFromSums(sums, "Lounge-0.1.2.apk"))
        assertEquals(beef, Release.hashFromSums(sums, "SHA256SUMS"))
        assertNull(Release.hashFromSums(sums, "Lounge-0.1.3.apk"))
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
