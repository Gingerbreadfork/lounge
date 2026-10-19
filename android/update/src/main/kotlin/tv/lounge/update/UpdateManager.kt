package tv.lounge.update

import android.app.Activity
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.net.Uri
import android.os.Build
import android.provider.Settings
import android.util.Log
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
    /** Downloaded and verified, waiting for a quiet moment or a press. */
    data class Ready(override val release: Release, val file: File, val path: InstallPath) : UpdateState {
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
 * the background; the Settings sheet and the home notice only show where it stands.
 */
class UpdateManager(
    private val context: Context,
    private val power: PowerSetup,
    val currentVersion: String,
    private val repo: String = "Gingerbreadfork/lounge",
) {
    private val prefs = context.getSharedPreferences("updates", Context.MODE_PRIVATE)
    private val _state = MutableStateFlow<UpdateState>(restoredState())
    private val _autoCheck = MutableStateFlow(prefs.getBoolean(KEY_AUTO, true))
    private val _confirmRequests = MutableSharedFlow<Intent>(extraBufferCapacity = 1)
    private val work = Mutex()
    @Volatile private var inForeground = false
    private var loop: Job? = null

    val state: StateFlow<UpdateState> = _state.asStateFlow()
    val autoCheck: StateFlow<Boolean> = _autoCheck.asStateFlow()
    /** Android's confirmation dialogs, to be started by the activity. */
    val confirmRequests: SharedFlow<Intent> = _confirmRequests.asSharedFlow()
    val lastCheckedMs: Long get() = prefs.getLong(KEY_LAST_CHECK, 0L)

    /** The release found last time, when it is still newer than this build. */
    private fun restoredState(): UpdateState {
        val release = prefs.getString(KEY_RELEASE_JSON, null)?.let { runCatching { Release.parse(it) }.getOrNull() }
        val installed = Version.parse(currentVersion)
        val checked = lastCheckedMs
        return when {
            checked == 0L -> UpdateState.Idle(0L)
            release == null || installed == null || release.version <= installed -> UpdateState.UpToDate(checked)
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
    }

    fun start(scope: CoroutineScope) {
        current = this
        if (loop != null) return
        runCatching { context.packageManager.packageInstaller.let { pi -> pi.mySessions.forEach { pi.abandonSession(it.sessionId) } } }
        scope.launch {
            var last = ""
            _state.collect {
                val line = "${it.label}${it.release?.let { r -> " ${r.tag}" } ?: ""}"
                if (line != last) Log.i(TAG, "updates: $line")
                last = line
            }
        }
        loop = scope.launch {
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
            val s = _state.value
            val now = System.currentTimeMillis()
            val due = now - lastCheckedMs > CHECK_EVERY_MS || (s is UpdateState.Failed && now - s.atMs > RETRY_AFTER_MS)
            when {
                !_autoCheck.value -> return
                due && (s is UpdateState.Idle || s is UpdateState.UpToDate || s is UpdateState.Failed) -> if (s is UpdateState.Failed && s.release != null) download(s.release) else check()
                s is UpdateState.Available -> download(s.release)
                // A quiet install waits until the home is off screen; one that needs a confirmation
                // only prepares the session, so Android has the dialog ready for the notice.
                s is UpdateState.Ready && currentPath().let { it == InstallPath.Confirm || (!inForeground && it != InstallPath.NeedsPermission) } -> install(interactive = false)
                else -> return
            }
        }
    }

    /** Looks for a newer release right now. */
    suspend fun check(): UpdateState = work.withLock {
        _state.value = UpdateState.Checking
        val result = runCatching { fetchLatest() }
        val now = System.currentTimeMillis()
        val next = result.fold(
            onSuccess = { (release, json) ->
                prefs.edit().putLong(KEY_LAST_CHECK, now).putString(KEY_RELEASE_JSON, json).apply()
                val installed = Version.parse(currentVersion)
                when {
                    release == null -> UpdateState.UpToDate(now)
                    installed != null && release.version <= installed -> UpdateState.UpToDate(now)
                    else -> readyIfDownloaded(release) ?: UpdateState.Available(release)
                }
            },
            onFailure = { UpdateState.Failed(it.message ?: "Could not reach GitHub", null, now) },
        )
        _state.value = next
        next
    }

    /** Fetches and verifies the APK; the state ends at [UpdateState.Ready] or [UpdateState.Failed]. */
    suspend fun download(release: Release): UpdateState = work.withLock {
        readyIfDownloaded(release)?.let { _state.value = it; return it }
        _state.value = UpdateState.Downloading(release, 0f)
        val result = runCatching { fetchApk(release) }
        val next = result.fold(
            onSuccess = { file -> UpdateState.Ready(release, file, currentPath()) },
            onFailure = { UpdateState.Failed(it.message ?: "The download failed", release, System.currentTimeMillis()) },
        )
        _state.value = next
        next
    }

    /**
     * Installs the downloaded update. Without a prompt where Android or Power Setup allow it;
     * otherwise Android's dialog is opened at once when [interactive], or kept for the notice.
     */
    suspend fun install(interactive: Boolean): UpdateState = work.withLock {
        val ready = _state.value as? UpdateState.Ready ?: return _state.value
        val path = currentPath()
        if (path == InstallPath.NeedsPermission) {
            _state.value = ready.copy(path = path)
            return _state.value
        }
        _state.value = UpdateState.Installing(ready.release)
        pendingInteractive = interactive
        pendingFile = ready.file
        pendingSilent = path == InstallPath.Silent
        if (path == InstallPath.PowerSetup) {
            val r = runCatching { power.run(installAction(ready.file)) }
            if (r.isSuccess && r.getOrThrow().ok) {
                Log.i(TAG, "updates: installed ${ready.release.tag} through Power Setup")
                return _state.value
            }
            Log.w(TAG, "updates: Power Setup install failed: ${r.exceptionOrNull()?.message ?: r.getOrNull()?.output}")
            if (!canRequestInstalls()) {
                _state.value = UpdateState.Failed("Power Setup could not install it", ready.release, System.currentTimeMillis())
                return _state.value
            }
        }
        runCatching { commitSession(ready.release, ready.file, silent = path == InstallPath.Silent) }
            .onFailure { _state.value = UpdateState.Failed(it.message ?: "The install failed", ready.release, System.currentTimeMillis()) }
        _state.value
    }

    /** Does the one thing the current state is waiting for, from a press in the UI. */
    fun interact(activity: Activity, scope: CoroutineScope) {
        when (val s = _state.value) {
            is UpdateState.NeedsConfirmation -> runCatching { activity.startActivity(s.confirm) }
            is UpdateState.Ready -> if (currentPath() == InstallPath.NeedsPermission) openInstallPermission(activity) else scope.launch { install(interactive = true) }
            is UpdateState.Available -> scope.launch { download(s.release); if (_state.value is UpdateState.Ready) install(interactive = true) }
            is UpdateState.Failed -> scope.launch { if (s.release != null) download(s.release) else check() }
            else -> Unit
        }
    }

    /** Android's "install unknown apps" switch for Lounge. */
    fun openInstallPermission(activity: Activity) {
        val intent = Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${context.packageName}"))
        runCatching { activity.startActivity(intent) }.onFailure { Log.w(TAG, "updates: no unknown-sources screen") }
    }

    fun canRequestInstalls(): Boolean = context.packageManager.canRequestPackageInstalls()

    fun currentPath(): InstallPath = installPath(
        sdk = Build.VERSION.SDK_INT,
        selfIsInstaller = selfIsInstaller() && !prefs.getBoolean(KEY_SILENT_BLOCKED, false),
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

    private fun readyIfDownloaded(release: Release): UpdateState.Ready? {
        val file = fileFor(release)
        val hash = release.sha256 ?: return null
        return if (file.exists() && file.length() == release.apkSize && sha256(file) == hash) UpdateState.Ready(release, file, currentPath()) else null
    }

    private fun fileFor(release: Release): File = File(downloadDir(), release.apkName)

    private fun sha256(file: File): String {
        val digest = MessageDigest.getInstance("SHA-256")
        file.inputStream().use { input ->
            val buf = ByteArray(64 * 1024)
            while (true) {
                val n = input.read(buf)
                if (n < 0) break
                digest.update(buf, 0, n)
            }
        }
        return digest.digest().joinToString("") { "%02x".format(it) }
    }

    /** External storage when there is any: the shell user can read it, so Power Setup can install from it. */
    private fun downloadDir(): File = File(context.getExternalFilesDir(null) ?: context.cacheDir, "updates").apply { mkdirs() }

    private fun installAction(file: File): PowerAction = PowerAction(
        "install_update",
        "Install an update",
        "Installs the downloaded Lounge update without a prompt.",
        "pm install -r -i ${context.packageName} ${shellQuote(file.absolutePath)}",
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

    private suspend fun fetchApk(release: Release): File = withContext(Dispatchers.IO) {
        val expected = release.sha256 ?: release.sumsUrl?.let { url ->
            open(url).run { try { Release.hashFromSums(inputStream.bufferedReader().readText(), release.apkName) } finally { disconnect() } }
        } ?: throw IllegalStateException("The release has no checksum for ${release.apkName}")
        val target = fileFor(release)
        val partial = File(target.path + ".part")
        val conn = open(release.apkUrl)
        try {
            if (conn.responseCode != 200) throw IllegalStateException("The download answered HTTP ${conn.responseCode}")
            val total = conn.contentLengthLong.takeIf { it > 0 } ?: release.apkSize
            val digest = MessageDigest.getInstance("SHA-256")
            conn.inputStream.use { input ->
                partial.outputStream().use { out ->
                    val buf = ByteArray(64 * 1024)
                    var done = 0L
                    var lastReported = 0f
                    while (true) {
                        val n = input.read(buf)
                        if (n < 0) break
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
            val actual = digest.digest().joinToString("") { "%02x".format(it) }
            if (actual != expected) {
                partial.delete()
                throw IllegalStateException("The download did not match its checksum")
            }
            if (!partial.renameTo(target)) throw IllegalStateException("Could not keep the download")
            downloadDir().listFiles()?.filter { it != target && it.name.endsWith(".apk") }?.forEach { it.delete() }
            target
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

    private fun commitSession(release: Release, file: File, silent: Boolean) {
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
        installer.openSession(id).use { session ->
            session.openWrite("lounge.apk", 0, file.length()).use { out ->
                file.inputStream().use { it.copyTo(out) }
                session.fsync(out)
            }
            val intent = Intent(context, InstallReceiver::class.java).setAction(ACTION_RESULT).putExtra(EXTRA_TAG, release.tag)
            val pending = PendingIntent.getBroadcast(context, id, intent, PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
            session.commit(pending.intentSender)
        }
        Log.i(TAG, "updates: session $id committed for ${release.tag}")
    }

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
                if (pendingSilent) prefs.edit().putBoolean(KEY_SILENT_BLOCKED, true).apply()
                _state.value = UpdateState.Failed(text, release, System.currentTimeMillis())
            }
        }
        pendingInteractive = false
    }

    companion object {
        internal const val TAG = "Lounge"
        internal const val ACTION_RESULT = "tv.lounge.update.RESULT"
        internal const val EXTRA_TAG = "tag"
        private const val KEY_AUTO = "auto"
        private const val KEY_LAST_CHECK = "last_check"
        private const val KEY_SEEN_VERSION = "seen_version"
        private const val KEY_RELEASE_JSON = "release_json"
        private const val KEY_SILENT_BLOCKED = "silent_blocked"
        private const val FIRST_DELAY_MS = 45_000L
        private const val TICK_MS = 15 * 60_000L
        private const val CHECK_EVERY_MS = 12 * 3_600_000L
        private const val RETRY_AFTER_MS = 3_600_000L

        /** The one manager in the process, for the install receiver. */
        @Volatile internal var current: UpdateManager? = null

        internal fun shellQuote(value: String): String = "'" + value.replace("'", "'\\''") + "'"
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
