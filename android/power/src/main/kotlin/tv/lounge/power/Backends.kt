package tv.lounge.power

import android.content.Context
import android.content.pm.PackageManager
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.withContext
import rikka.shizuku.Shizuku

/** The device's own ADB daemon over localhost, authorised once by the person on screen. */
class AdbBackend(private val client: AdbClient, private val context: Context) : Privileged {
    override val name: String = "adb"

    override suspend fun available(): Boolean = withContext(Dispatchers.IO) {
        when (val o = client.connect(presentKey = false, connectTimeoutMs = 1_500)) {
            is AdbClient.Outcome.Online -> { o.connection.close(); true }
            else -> false
        }
    }

    override suspend fun run(command: String, timeoutMs: Int): ShellResult = withContext(Dispatchers.IO) {
        when (val o = client.connect(presentKey = false, connectTimeoutMs = 2_000)) {
            is AdbClient.Outcome.Online -> try {
                o.connection.use { c -> parseResult(c.shell("($command); echo __rc=$?", timeoutMs)) }
            } catch (e: java.net.SocketTimeoutException) {
                throw PrivilegedException(context.getString(R.string.backends_took_too_long), e)
            } catch (e: java.io.IOException) {
                throw PrivilegedException(context.getString(R.string.backends_dropped), e)
            }
            AdbClient.Outcome.Refused -> throw PrivilegedException(context.getString(R.string.backends_unreachable))
            AdbClient.Outcome.Unknown, AdbClient.Outcome.Denied -> throw KeyNotApprovedException(context.getString(R.string.backends_key_not_approved))
            is AdbClient.Outcome.TlsRequired -> throw PrivilegedException(context.getString(R.string.backends_tls), IllegalStateException(o.detail))
            is AdbClient.Outcome.NotThisDevice -> throw PrivilegedException(context.getString(R.string.backends_not_this_tv), IllegalStateException(o.detail))
            is AdbClient.Outcome.Error -> throw PrivilegedException(context.getString(R.string.backends_dropped), IllegalStateException(o.detail))
        }
    }

    companion object {
        /** Splits the trailing exit-code marker off a shell transcript. */
        fun parseResult(raw: String): ShellResult {
            val text = raw.replace("\r\n", "\n")
            val marker = text.lastIndexOf("__rc=")
            if (marker < 0) return ShellResult(-1, text.trim())
            val code = text.substring(marker + 5).trim().toIntOrNull() ?: -1
            return ShellResult(code, text.substring(0, marker).trim())
        }
    }
}

/** Shizuku, when the person already runs it: the same commands through its binder. */
class ShizukuBackend(private val context: Context) : Privileged {
    override val name: String = "shizuku"

    val installed: Boolean get() = runCatching { Shizuku.pingBinder() }.getOrDefault(false)

    val permitted: Boolean
        get() = runCatching { Shizuku.pingBinder() && Shizuku.checkSelfPermission() == PackageManager.PERMISSION_GRANTED }.getOrDefault(false)

    override suspend fun available(): Boolean = permitted

    fun requestPermission(requestCode: Int) {
        runCatching { Shizuku.requestPermission(requestCode) }
    }

    override suspend fun run(command: String, timeoutMs: Int): ShellResult = withContext(Dispatchers.IO) {
        if (!permitted) throw PrivilegedException(context.getString(R.string.backends_shizuku_not_allowed))
        val newProcess = Shizuku::class.java.getDeclaredMethod("newProcess", Array<String>::class.java, Array<String>::class.java, String::class.java)
        newProcess.isAccessible = true
        val process = newProcess.invoke(null, arrayOf("sh", "-c", command), null, null) as? Process
            ?: throw PrivilegedException(context.getString(R.string.backends_shizuku_not_running))
        try {
            coroutineScope {
                // Both streams drain at once, so a chatty stderr cannot fill its pipe and stall it.
                val stdout = async { process.inputStream.readBytes() }
                val stderr = async { process.errorStream.readBytes() }
                if (!process.waitFor(timeoutMs.toLong(), java.util.concurrent.TimeUnit.MILLISECONDS)) {
                    process.destroy()
                    throw PrivilegedException(context.getString(R.string.backends_took_too_long))
                }
                val text = (stdout.await().toString(Charsets.UTF_8) + stderr.await().toString(Charsets.UTF_8)).trim()
                ShellResult(process.exitValue(), text)
            }
        } finally {
            runCatching { process.destroy() }
        }
    }
}
