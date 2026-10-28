package tv.lounge.update

import android.app.Activity
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInfo
import android.content.pm.PackageInstaller
import android.content.pm.PackageManager
import android.content.pm.Signature
import android.net.Uri
import android.os.Build
import android.provider.Settings
import android.util.Log
import android.widget.Toast
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import tv.lounge.power.PowerAction
import tv.lounge.power.PowerSetup
import tv.lounge.power.PowerStatus

/** Where the updater stands. */
sealed interface UpdateState {
    val release: Release? get() = null
    /** A readable name for logs; class names are shortened in release builds. */
    val label: String

    /** Nothing checked yet this run. */
    data class Idle(val checkedAtMs: Long) : UpdateState {
        override val label: String get() = "Idle"
    }
    data object Checking : UpdateState {
        override val label: String get() = "Checking"
    }
    data class UpToDate(val checkedAtMs: Long) : UpdateState {
        override val label: String get() = "UpToDate"
    }
    data class Available(override val release: Release) : UpdateState {
        override val label: String get() = "Available"
    }
    data class Downloading(override val release: Release, val progress: Float) : UpdateState {
        override val label: String get() = "Downloading"
    }
    /** Downloaded and verified against [sha256], waiting for a quiet moment or a press. */
    data class Ready(override val release: Release, val file: File, val sha256: String, val path: InstallPath) : UpdateState {
        override val label: String get() = "Ready ($path)"
    }
    data class Installing(override val release: Release) : UpdateState {
        override val label: String get() = "Installing"
    }
    /** Android wants the person to confirm; [confirm] opens its dialog. */
    data class NeedsConfirmation(override val release: Release, val file: File, val confirm: Intent) : UpdateState {
        override val label: String get() = "NeedsConfirmation"
    }
    data class Failed(val message: String, override val release: Release?, val atMs: Long) : UpdateState {
        override val label: String get() = "Failed"
    }
}

/**
 * Keeps Lounge current from its GitHub releases: a check every half day, a verified download,
 * and an install that asks nothing when Android or Power Setup allows it. Everything happens in
 * the background and off the main thread; the Settings sheet and the home notice only show where
 * it stands.
 */
class UpdateManager(
    private val context: Context,
    private val power: PowerSetup,
    val currentVersion: String,
    private val repo: String = "Gingerbreadfork/lounge",
) {
    private val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
    private val _state = MutableStateFlow<UpdateState>(UpdateState.Idle(0L))
    private val _autoCheck = MutableStateFlow(true)
    private val _confirmRequests = MutableSharedFlow<Intent>(extraBufferCapacity = 1)
    private val work = Mutex()
    @Volatile private var inForeground = false
    @Volatile private var lastAttemptMs = 0L
    @Volatile private var installingSince = 0L
    private var loop: Job? = null

    val state: StateFlow<UpdateState> = _state.asStateFlow()
    val autoCheck: StateFlow<Boolean> = _autoCheck.asStateFlow()
    /** Android's confirmation dialogs, to be started by the activity. */
    val confirmRequests: SharedFlow<Intent> = _confirmRequests.asSharedFlow()
    val lastCheckedMs: Long get() = prefs.getLong(KEY_LAST_CHECK, 0L)

    /** What the last run left: the release it found and, if its download still checks out, that it is ready. */
    private fun restoredState(): UpdateState {
        val release = prefs.getString(KEY_RELEASE_JSON, null)?.let { runCatching { Release.parse(it) }.getOrNull() }
        val installed = Version.parse(currentVersion)
        val checked = lastCheckedMs
        return when {
            checked == 0L -> UpdateState.Idle(0L)
            release == null || installed == null || release.version <= installed -> UpdateState.UpToDate(checked)
            failures.gaveUpOn(release.tag) -> UpdateState.Failed(gaveUpMessage(release), release, checked)
            else -> readyIfDownloaded(release) ?: UpdateState.Available(release)
        }
    }

    /** The version this process replaced, once, so the home can say so. */
    fun takeJustUpdatedFrom(): String? {
        val previous = prefs.getString(KEY_SEEN_VERSION, null)
        if (previous == currentVersion) return null
        prefs.edit().putString(KEY_SEEN_VERSION, currentVersion).apply()
        return previous
    }

    fun setAutoCheck(on: Boolean) {
        prefs.edit().putBoolean(KEY_AUTO, on).apply()
        _autoCheck.value = on
    }

    /** The activity tells the updater when the home screen is on screen, so quiet installs wait. */
    fun noteForeground(foreground: Boolean) {
        inForeground = foreground
        // Back from Android's list with Lounge allowed: carry on with the install that asked.
        if (foreground && awaitingPermission) {
            awaitingPermission = false
            if (canRequestInstalls() && _state.value is UpdateState.Ready) {
                returnAfterInstall()
                workScope?.launch(Dispatchers.IO) { install(interactive = true) }
            }
        }
    }

    /** Starts the background work. Restoring the last run's state reads and hashes files, so it happens here, off the caller's thread. */
    fun start(scope: CoroutineScope) {
        current = this
        workScope = scope
        if (loop != null) return
        scope.launch {
            var last = ""
            _state.collect {
                val line = "${it.label}${it.release?.let { r -> " ${r.tag}" } ?: ""}"
                if (line != last) Log.i(TAG, "updates: $line")
                last = line
            }
        }
        loop = scope.launch(Dispatchers.IO) {
            work.withLock {
                runCatching { context.packageManager.packageInstaller.let { pi -> pi.mySessions.forEach { pi.abandonSession(it.sessionId) } } }
                _autoCheck.value = prefs.getBoolean(KEY_AUTO, true)
                lastAttemptMs = lastCheckedMs
                if (_state.value is UpdateState.Idle) _state.value = restoredState()
                // A download that is not newer than this version has been installed or overtaken.
                val keep = _state.value.release?.takeIf { r -> Version.parse(currentVersion)?.let { r.version > it } ?: true }?.apkName
                downloadDir().listFiles()?.filter { it.name != keep }?.forEach { it.delete() }
                stagingDir()?.listFiles()?.forEach { it.delete() }
            }
            delay(FIRST_DELAY_MS)
            while (true) {
                runCatching { tick() }.onFailure { Log.w(TAG, "updates: ${it.message}") }
                delay(TICK_MS)
            }
        }
    }

    /** One pass of the background work: a due check, then the download, then a quiet install. */
    private suspend fun tick() {
        repeat(3) {
            if (!_autoCheck.value) return
            val s = _state.value
            val now = System.currentTimeMillis()
            when {
                s is UpdateState.Installing -> {
                    // Power Setup can report success without Android replacing the app; give up and retry later.
                    if (now - installingSince > INSTALL_TIMEOUT_MS) {
                        Log.w(TAG, "updates: the install of ${s.release.tag} never finished")
                        noteFailure(s.release, final = false)
                        _state.value = UpdateState.Failed("The install did not finish", s.release, now)
                    }
                    return
                }
                s is UpdateState.Checking || s is UpdateState.Downloading -> return
                // Due checks run from every resting state, so a newer release replaces one still waiting.
                now - lastAttemptMs > CHECK_EVERY_MS -> work.withLock { checkLocked(quiet = s.isWaiting()) }
                s is UpdateState.Failed && s.release != null && failures.gaveUpOn(s.release.tag) -> return
                s is UpdateState.Failed && now - s.atMs > RETRY_AFTER_MS -> if (s.release != null) download(s.release) else check()
                s is UpdateState.Available -> download(s.release)
                // A quiet install waits until the home is off screen; one that needs a confirmation
                // only prepares the session, so Android has the dialog ready for the notice.
                s is UpdateState.Ready && currentPath().let { it == InstallPath.Confirm || (!inForeground && it != InstallPath.NeedsPermission) } -> install(interactive = false)
                else -> return
            }
        }
    }

    private fun UpdateState.isWaiting(): Boolean =
        this is UpdateState.Available || this is UpdateState.Ready || this is UpdateState.NeedsConfirmation

    /** Looks for a newer release right now. */
    suspend fun check(): UpdateState = withContext(Dispatchers.IO) {
        when (val found = work.withLock { checkLocked(quiet = false) }) {
            is UpdateState.Available -> prepare(download(found.release))
            is UpdateState.Ready -> prepare(found)
            else -> found
        }
    }

    /**
     * After a check someone asked for, the next steps run now rather than on the background
     * loop's next pass: the download, then Android's dialog where the install needs one.
     */
    private suspend fun prepare(s: UpdateState): UpdateState =
        if (s is UpdateState.Ready && currentPath() == InstallPath.Confirm) install(interactive = false) else s

    /**
     * One look at GitHub. [quiet] leaves a waiting update on show while it looks and keeps it if the
     * look fails; the same release found again keeps its state, confirmation and all.
     */
    private suspend fun checkLocked(quiet: Boolean): UpdateState {
        val before = _state.value
        if (!quiet) _state.value = UpdateState.Checking
        lastAttemptMs = System.currentTimeMillis()
        val result = runCatching { fetchLatest() }
        val now = System.currentTimeMillis()
        val next = result.fold(
            onSuccess = { (release, json) ->
                prefs.edit().putLong(KEY_LAST_CHECK, now).putString(KEY_RELEASE_JSON, json).apply()
                val installed = Version.parse(currentVersion)
                when {
                    release == null -> UpdateState.UpToDate(now)
                    installed != null && release.version <= installed -> UpdateState.UpToDate(now)
                    failures.gaveUpOn(release.tag) -> UpdateState.Failed(gaveUpMessage(release), release, now)
                    before.isWaiting() && before.release?.version == release.version -> before
                    else -> readyIfDownloaded(release) ?: UpdateState.Available(release)
                }
            },
            onFailure = { if (quiet) before else UpdateState.Failed(it.message ?: "Could not reach GitHub", null, now) },
        )
        _state.value = next
        return next
    }

    /** Fetches and verifies the APK; the state ends at [UpdateState.Ready] or [UpdateState.Failed]. */
    suspend fun download(release: Release): UpdateState = withContext(Dispatchers.IO) {
        work.withLock {
            readyIfDownloaded(release)?.let { _state.value = it; return@withLock it }
            _state.value = UpdateState.Downloading(release, 0f)
            val result = runCatching { fetchApk(release) }
            val next = result.fold(
                onSuccess = { (file, hash) -> UpdateState.Ready(release, file, hash, currentPath()) },
                onFailure = {
                    noteFailure(release, final = false)
                    UpdateState.Failed(it.message ?: "The download failed", release, System.currentTimeMillis())
                },
            )
            _state.value = next
            next
        }
    }

    /**
     * Installs the downloaded update. Without a prompt where Android or Power Setup allow it;
     * otherwise Android's dialog is opened at once when [interactive], or kept for the notice.
     * Every path checks the file again first: its hash, that it is Lounge, and that it is signed
     * like the installed copy.
     */
    suspend fun install(interactive: Boolean): UpdateState = withContext(Dispatchers.IO) {
        work.withLock { installLocked(interactive) }
    }

    private suspend fun installLocked(interactive: Boolean): UpdateState {
        val ready = _state.value as? UpdateState.Ready ?: return _state.value
        val path = currentPath()
        if (path == InstallPath.NeedsPermission) {
            _state.value = ready.copy(path = path)
            return _state.value
        }
        val fail = { message: String, final: Boolean ->
            noteFailure(ready.release, final)
            UpdateState.Failed(message, ready.release, System.currentTimeMillis()).also { _state.value = it }
        }
        runCatching { verify(ready.file, ready.sha256, ready.release) }.onFailure { e ->
            Log.w(TAG, "updates: ${ready.file.name} failed its check: ${e.message}")
            ready.file.delete()
            return fail(e.message ?: "The update did not check out", e is NotInstallable)
        }
        _state.value = UpdateState.Installing(ready.release)
        installingSince = System.currentTimeMillis()
        pendingInteractive = interactive
        pendingFile = ready.file
        pendingSilent = path == InstallPath.Silent
        if (path == InstallPath.PowerSetup) {
            val r = runCatching {
                val staged = stage(ready.file, ready.sha256, ready.release)
                try {
                    power.run(installAction(staged, ready.sha256, digest(ready.file, "SHA-1")))
                } finally {
                    staged.delete()
                }
            }
            if (r.isSuccess && r.getOrThrow().ok) {
                Log.i(TAG, "updates: installed ${ready.release.tag} through Power Setup")
                return _state.value
            }
            Log.w(TAG, "updates: Power Setup install failed: ${r.exceptionOrNull()?.message ?: r.getOrNull()?.output}")
            if (!canRequestInstalls()) return fail("Power Setup could not install it", false)
        }
        runCatching { commitSession(ready.release, ready.file, ready.sha256, silent = path == InstallPath.Silent) }
            .onFailure { fail(it.message ?: "The install failed", false) }
        return _state.value
    }

    /** Does the one thing the current state is waiting for, from a press in the UI. */
    fun interact(activity: Activity, scope: CoroutineScope) {
        when (val s = _state.value) {
            is UpdateState.NeedsConfirmation -> {
                returnAfterInstall()
                runCatching { activity.startActivity(s.confirm) }
            }
            is UpdateState.Ready -> if (currentPath() == InstallPath.NeedsPermission) {
                openInstallPermission(activity)
            } else {
                returnAfterInstall()
                scope.launch { install(interactive = true) }
            }
            is UpdateState.Available -> {
                returnAfterInstall()
                scope.launch { download(s.release); if (_state.value is UpdateState.Ready) install(interactive = true) }
            }
            is UpdateState.Failed -> scope.launch { if (s.release != null) download(s.release) else check() }
            else -> Unit
        }
    }

    /**
     * Android's "install unknown apps" switch for Lounge. TV settings show the whole list rather
     * than Lounge's own page, so a hint says what to switch on; coming back with it allowed
     * carries on with the install.
     */
    fun openInstallPermission(activity: Activity) {
        val intent = Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${context.packageName}"))
        runCatching { activity.startActivity(intent) }
            .onSuccess {
                awaitingPermission = true
                Toast.makeText(activity, "Switch on Lounge in this list, then press Back", Toast.LENGTH_LONG).show()
            }
            .onFailure { Log.w(TAG, "updates: no unknown-sources screen") }
    }

    /** The person started this install from Lounge, so it opens again once Android has replaced it. */
    private fun returnAfterInstall() {
        prefs.edit().putLong(KEY_RETURN, System.currentTimeMillis()).commit()
    }

    fun canRequestInstalls(): Boolean = context.packageManager.canRequestPackageInstalls()

    fun currentPath(): InstallPath = installPath(
        sdk = Build.VERSION.SDK_INT,
        // A block holds for the version it happened on; the next one may be allowed again.
        selfIsInstaller = selfIsInstaller() && prefs.getString(KEY_SILENT_BLOCKED_AT, null) != currentVersion,
        powerReady = power.status.value is PowerStatus.Ready,
        canRequestInstalls = canRequestInstalls(),
    )

    private fun selfIsInstaller(): Boolean = runCatching {
        if (Build.VERSION.SDK_INT >= 30) {
            context.packageManager.getInstallSourceInfo(context.packageName).installingPackageName == context.packageName
        } else {
            @Suppress("DEPRECATION")
            context.packageManager.getInstallerPackageName(context.packageName) == context.packageName
        }
    }.getOrDefault(false)

    /** The release's APK when it is already here and still matches its hash. Reads the whole file. */
    private fun readyIfDownloaded(release: Release): UpdateState.Ready? {
        val file = fileFor(release)
        val hash = release.sha256 ?: savedHash(release.apkName) ?: return null
        return if (file.exists() && file.length() == release.apkSize && sha256(file) == hash) UpdateState.Ready(release, file, hash, currentPath()) else null
    }

    /** The hash a download was checked against, for releases that list it only in SHA256SUMS. */
    private fun savedHash(apkName: String): String? =
        prefs.getString(KEY_APK_HASH, null)?.split(' ', limit = 2)?.takeIf { it.size == 2 && it[0] == apkName }?.get(1)

    private fun fileFor(release: Release): File = File(downloadDir(), release.apkName)

    private fun sha256(file: File): String = digest(file, "SHA-256")

    private fun digest(file: File, algorithm: String): String {
        val digest = MessageDigest.getInstance(algorithm)
        file.inputStream().use { input ->
            val buf = ByteArray(64 * 1024)
            while (true) {
                val n = input.read(buf)
                if (n < 0) break
                digest.update(buf, 0, n)
            }
        }
        return digest.digest().toHex()
    }

    /**
     * Throws unless [file] still has [sha256] and is a Lounge APK signed like the installed one,
     * newer than it and the version [release] names.
     */
    private fun verify(file: File, sha256: String, release: Release) {
        if (sha256(file) != sha256) throw IllegalStateException("The update changed after it was checked")
        checkPackage(file, release)
    }

    /** The file matches what was published, so anything wrong with it stays wrong for this release. */
    private fun checkPackage(file: File, release: Release) {
        val pm = context.packageManager
        val archive = archiveInfo(file) ?: throw NotInstallable("The update is not an app Android can read")
        if (archive.packageName != context.packageName) throw NotInstallable("The update is not Lounge")
        @Suppress("DEPRECATION")
        val installedInfo = pm.getPackageInfo(context.packageName, PackageManager.GET_SIGNING_CERTIFICATES or PackageManager.GET_SIGNATURES)
        if (!signedLike(signers(installedInfo, withHistory = false), signers(archive, withHistory = true))) throw NotInstallable("The update is not signed like this copy of Lounge")
        versionProblem(release.version, archive.versionName, archive.longVersionCode, installedInfo.longVersionCode)?.let { throw NotInstallable(it) }
    }

    /** A release that can never install here, whatever is tried again. */
    private class NotInstallable(message: String) : IllegalStateException(message)

    private var failures: Failures
        get() = Failures(prefs.getString(KEY_FAILED_TAG, null), prefs.getInt(KEY_FAILED_TRIES, 0))
        set(value) {
            prefs.edit().putString(KEY_FAILED_TAG, value.tag).putInt(KEY_FAILED_TRIES, value.tries).apply()
        }

    private fun noteFailure(release: Release?, final: Boolean) {
        if (release != null) failures = failures.after(release.tag, final)
    }

    private fun gaveUpMessage(release: Release): String = "Lounge ${release.version} did not install here, so it waits for the next release"

    private fun archiveInfo(file: File): PackageInfo? {
        // Signatures too: some versions only read an archive's certificates when asked for them.
        @Suppress("DEPRECATION")
        val flags = PackageManager.GET_SIGNING_CERTIFICATES or PackageManager.GET_SIGNATURES
        return if (Build.VERSION.SDK_INT >= 33) {
            context.packageManager.getPackageArchiveInfo(file.path, PackageManager.PackageInfoFlags.of(flags.toLong()))
        } else {
            @Suppress("DEPRECATION")
            context.packageManager.getPackageArchiveInfo(file.path, flags)
        }
    }

    /** SHA-256 digests of a package's signing certificates; with [withHistory], the ones it rotated from too. */
    private fun signers(info: PackageInfo, withHistory: Boolean): Set<String> {
        val signing = info.signingInfo
        val certificates: List<Signature> = when {
            signing == null -> @Suppress("DEPRECATION") info.signatures?.toList().orEmpty()
            signing.hasMultipleSigners() || !withHistory -> signing.apkContentsSigners?.toList().orEmpty()
            else -> signing.apkContentsSigners?.toList().orEmpty() + signing.signingCertificateHistory?.toList().orEmpty()
        }
        return certificates.map { MessageDigest.getInstance("SHA-256").digest(it.toByteArray()).toHex() }.toSet()
    }

    /** Private storage: nothing but Lounge can touch the APK between its checks. */
    private fun downloadDir(): File = File(context.noBackupFilesDir, "updates").apply { mkdirs() }

    /** Shared storage the shell user can read, used only for the moment of a Power Setup install. */
    private fun stagingDir(): File? = context.getExternalFilesDir(null)?.let { File(it, "updates") }

    /** Copies the APK where `pm` can read it and checks the copy, right before it is installed. */
    private fun stage(file: File, sha256: String, release: Release): File {
        val dir = stagingDir()?.apply { mkdirs() } ?: throw IllegalStateException("There is no shared storage for Power Setup to read from")
        val staged = File(dir, file.name)
        file.copyTo(staged, overwrite = true)
        try {
            verify(staged, sha256, release)
        } catch (e: Exception) {
            staged.delete()
            throw e
        }
        return staged
    }

    private fun installAction(file: File, sha256: String, sha1: String): PowerAction = PowerAction(
        "install_update",
        "Install an update",
        "Installs the downloaded Lounge update without a prompt.",
        installCommand(file.absolutePath, sha256, sha1, "pm install -r -i ${context.packageName}"),
        timeoutMs = 3 * 60_000,
    )

    // ----- network -----

    /** The latest release and the text it was read from, which is kept for the next start. */
    private suspend fun fetchLatest(): Pair<Release?, String?> = withContext(Dispatchers.IO) {
        val conn = open("https://api.github.com/repos/$repo/releases/latest").apply {
            setRequestProperty("Accept", "application/vnd.github+json")
            setRequestProperty("X-GitHub-Api-Version", "2022-11-28")
        }
        try {
            when (conn.responseCode) {
                200 -> conn.inputStream.bufferedReader().readText().let { Release.parse(it) to it }
                404 -> null to null
                403, 429 -> throw IllegalStateException("GitHub is limiting checks from this network; it will try again later")
                else -> throw IllegalStateException("GitHub answered HTTP ${conn.responseCode}")
            }
        } finally {
            conn.disconnect()
        }
    }

    /** The APK and the hash it was checked against. */
    private suspend fun fetchApk(release: Release): Pair<File, String> = withContext(Dispatchers.IO) {
        val expected = release.sha256 ?: release.sumsUrl?.let { url ->
            open(url).run { try { Release.hashFromSums(inputStream.bufferedReader().readText(), release.apkName) } finally { disconnect() } }
        } ?: throw IllegalStateException("The release has no checksum for ${release.apkName}")
        val target = fileFor(release)
        val partial = File(target.path + ".part")
        val conn = open(release.apkUrl)
        try {
            if (conn.responseCode != 200) throw IllegalStateException("The download answered HTTP ${conn.responseCode}")
            val total = conn.contentLengthLong.takeIf { it > 0 } ?: release.apkSize
            // A body that never ends must not fill the storage.
            val limit = 2 * release.apkSize
            val digest = MessageDigest.getInstance("SHA-256")
            conn.inputStream.use { input ->
                partial.outputStream().use { out ->
                    val buf = ByteArray(64 * 1024)
                    var done = 0L
                    var lastReported = 0f
                    while (true) {
                        val n = input.read(buf)
                        if (n < 0) break
                        if (done + n > limit) throw IllegalStateException("The download is larger than its release says")
                        out.write(buf, 0, n)
                        digest.update(buf, 0, n)
                        done += n
                        val p = if (total > 0) (done.toFloat() / total).coerceIn(0f, 1f) else 0f
                        if (p - lastReported >= 0.02f) {
                            lastReported = p
                            _state.value = UpdateState.Downloading(release, p)
                        }
                    }
                }
            }
            if (digest.digest().toHex() != expected) {
                partial.delete()
                throw IllegalStateException("The download did not match its checksum")
            }
            if (!partial.renameTo(target)) throw IllegalStateException("Could not keep the download")
            downloadDir().listFiles()?.filter { it != target && it.name.endsWith(".apk") }?.forEach { it.delete() }
            prefs.edit().putString(KEY_APK_HASH, "${release.apkName} $expected").apply()
            target to expected
        } finally {
            conn.disconnect()
        }
    }

    private fun open(url: String): HttpURLConnection = (URL(url).openConnection() as HttpURLConnection).apply {
        connectTimeout = 15_000
        readTimeout = 30_000
        instanceFollowRedirects = true
        setRequestProperty("User-Agent", "Lounge/$currentVersion (Android ${Build.VERSION.RELEASE})")
    }

    // ----- Android's installer -----

    /** Streams the APK into a session, hashing what is written, and commits only if it matches [sha256]. */
    private fun commitSession(release: Release, file: File, sha256: String, silent: Boolean) {
        val installer = context.packageManager.packageInstaller
        installer.mySessions.forEach { runCatching { installer.abandonSession(it.sessionId) } }
        val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL).apply {
            setAppPackageName(context.packageName)
            setSize(file.length())
            if (Build.VERSION.SDK_INT >= 31) {
                setRequireUserAction(if (silent) PackageInstaller.SessionParams.USER_ACTION_NOT_REQUIRED else PackageInstaller.SessionParams.USER_ACTION_REQUIRED)
            }
            if (Build.VERSION.SDK_INT >= 33) setPackageSource(PackageInstaller.PACKAGE_SOURCE_OTHER)
        }
        val id = installer.createSession(params)
        activeSession = id
        try {
            installer.openSession(id).use { session ->
                val digest = MessageDigest.getInstance("SHA-256")
                session.openWrite("lounge.apk", 0, file.length()).use { out ->
                    file.inputStream().use { input ->
                        val buf = ByteArray(64 * 1024)
                        while (true) {
                            val n = input.read(buf)
                            if (n < 0) break
                            out.write(buf, 0, n)
                            digest.update(buf, 0, n)
                        }
                    }
                    session.fsync(out)
                }
                if (digest.digest().toHex() != sha256) throw IllegalStateException("The update changed while it was being installed")
                val intent = Intent(context, InstallReceiver::class.java).setAction(ACTION_RESULT).putExtra(EXTRA_TAG, release.tag)
                val pending = PendingIntent.getBroadcast(context, id, intent, PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
                session.commit(pending.intentSender)
            }
        } catch (e: Exception) {
            runCatching { installer.abandonSession(id) }
            throw e
        }
        Log.i(TAG, "updates: session $id committed for ${release.tag}")
    }

    @Volatile private var workScope: CoroutineScope? = null
    @Volatile private var awaitingPermission = false
    @Volatile private var pendingInteractive = false
    @Volatile private var pendingFile: File? = null
    @Volatile private var pendingSilent = false
    @Volatile private var activeSession = -1

    /** Android's answer for a session; one from an earlier process is not ours to act on. */
    internal fun onInstallResult(sessionId: Int, status: Int, message: String?, confirm: Intent?) {
        if (sessionId != activeSession) {
            Log.i(TAG, "updates: ignoring status $status for session $sessionId")
            return
        }
        val release = _state.value.release
        when (status) {
            PackageInstaller.STATUS_PENDING_USER_ACTION -> {
                val file = pendingFile
                if (release != null && confirm != null && file != null) {
                    confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                    _state.value = UpdateState.NeedsConfirmation(release, file, confirm)
                    if (pendingInteractive) _confirmRequests.tryEmit(confirm)
                }
            }
            PackageInstaller.STATUS_SUCCESS -> Log.i(TAG, "updates: installed")
            else -> {
                val text = when (status) {
                    PackageInstaller.STATUS_FAILURE_ABORTED -> "The install was cancelled"
                    PackageInstaller.STATUS_FAILURE_BLOCKED -> "Android blocked the install"
                    PackageInstaller.STATUS_FAILURE_STORAGE -> "Not enough storage for the update"
                    PackageInstaller.STATUS_FAILURE_INCOMPATIBLE -> "This update does not fit this device"
                    else -> message ?: "The install failed"
                }
                Log.w(TAG, "updates: install status $status: $message")
                val final = status == PackageInstaller.STATUS_FAILURE_INCOMPATIBLE || status == PackageInstaller.STATUS_FAILURE_CONFLICT || status == PackageInstaller.STATUS_FAILURE_INVALID
                noteFailure(release, final)
                // Only Android refusing a quiet install says quiet installs do not work here; storage or a cancel does not.
                if (pendingSilent && status == PackageInstaller.STATUS_FAILURE_BLOCKED) prefs.edit().putString(KEY_SILENT_BLOCKED_AT, currentVersion).apply()
                // Cancelled or refused: a later quiet update must not pull Lounge to the front.
                prefs.edit().remove(KEY_RETURN).apply()
                _state.value = UpdateState.Failed(text, release, System.currentTimeMillis())
            }
        }
        pendingInteractive = false
    }

    companion object {
        internal const val TAG = "Lounge"
        internal const val ACTION_RESULT = "tv.lounge.update.RESULT"
        internal const val EXTRA_TAG = "tag"
        internal const val PREFS = "updates"
        internal const val KEY_RETURN = "return_after_install"
        /** How long after a person starts an install Lounge still opens itself when it lands. */
        internal const val RETURN_WITHIN_MS = 10 * 60_000L
        private const val KEY_AUTO = "auto"
        private const val KEY_LAST_CHECK = "last_check"
        private const val KEY_SEEN_VERSION = "seen_version"
        private const val KEY_RELEASE_JSON = "release_json"
        private const val KEY_SILENT_BLOCKED_AT = "silent_blocked_at"
        private const val KEY_APK_HASH = "apk_hash"
        private const val KEY_FAILED_TAG = "failed_tag"
        private const val KEY_FAILED_TRIES = "failed_tries"
        private const val FIRST_DELAY_MS = 45_000L
        private const val TICK_MS = 15 * 60_000L
        private const val CHECK_EVERY_MS = 12 * 3_600_000L
        private const val RETRY_AFTER_MS = 3_600_000L
        private const val INSTALL_TIMEOUT_MS = 10 * 60_000L

        /** The one manager in the process, for the install receiver. */
        @Volatile internal var current: UpdateManager? = null

        internal fun shellQuote(value: String): String = "'" + value.replace("'", "'\\''") + "'"

        /**
         * Copies the staged APK into the shell user's own directory and checks that copy before
         * [install] reads it: on Android 10 and earlier other apps can write to shared storage, but
         * not there, and the copy is the shell user's alone. A toybox without sha256sum checks [sha1]
         * instead.
         */
        internal fun installCommand(source: String, sha256: String, sha1: String, install: String, copy: String = "/data/local/tmp/lounge-update.apk"): String {
            val t = shellQuote(copy)
            return listOf(
                "umask 077",
                "rm -f $t",
                "cp ${shellQuote(source)} $t || exit 1",
                "if command -v sha256sum >/dev/null; then set -- \$(sha256sum $t); want=$sha256; else set -- \$(sha1sum $t); want=$sha1; fi",
                "if [ \"\$1\" != \"\$want\" ]; then rm -f $t; echo 'The update changed before it was installed'; exit 1; fi",
                "$install $t; r=\$?",
                "rm -f $t",
                "exit \$r",
            ).joinToString("\n")
        }

        /** Whether an update signed by [update] may replace an app signed by [installed]: every current signer must be among them. */
        internal fun signedLike(installed: Set<String>, update: Set<String>): Boolean =
            installed.isNotEmpty() && update.containsAll(installed)

        private fun ByteArray.toHex(): String = joinToString("") { "%02x".format(it) }
    }
}

/** Android's answer to an install session. */
class InstallReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != UpdateManager.ACTION_RESULT) return
        val sessionId = intent.getIntExtra(PackageInstaller.EXTRA_SESSION_ID, -1)
        val status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)
        val message = intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE)
        @Suppress("DEPRECATION")
        val confirm = intent.getParcelableExtra<Intent>(Intent.EXTRA_INTENT)
        UpdateManager.current?.onInstallResult(sessionId, status, message, confirm)
    }
}

/**
 * Android replaced Lounge with an update. When the person started it from Lounge, open Lounge again
 * rather than leaving them in whatever app was behind it: as the home screen where it is one.
 */
class UpdatedReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_MY_PACKAGE_REPLACED) return
        val prefs = context.getSharedPreferences(UpdateManager.PREFS, Context.MODE_PRIVATE)
        val askedAt = prefs.getLong(UpdateManager.KEY_RETURN, 0L)
        prefs.edit().remove(UpdateManager.KEY_RETURN).apply()
        if (System.currentTimeMillis() - askedAt > UpdateManager.RETURN_WITHIN_MS) return
        val pm = context.packageManager
        val home = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_HOME)
        val isHome = pm.resolveActivity(home, PackageManager.MATCH_DEFAULT_ONLY)?.activityInfo?.packageName == context.packageName
        val open = if (isHome) home else pm.getLeanbackLaunchIntentForPackage(context.packageName) ?: pm.getLaunchIntentForPackage(context.packageName) ?: return
        runCatching { context.startActivity(open.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
            .onFailure { Log.w(UpdateManager.TAG, "updates: could not reopen Lounge: ${it.message}") }
    }
}
