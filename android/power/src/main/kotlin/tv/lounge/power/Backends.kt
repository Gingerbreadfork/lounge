package tv.lounge.power

import android.content.pm.PackageManager
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.withContext
import rikka.shizuku.Shizuku

/** The device's own ADB daemon over localhost, authorised once by the person on screen. */
class AdbBackend(private val client: AdbClient) : Privileged {
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
                throw PrivilegedException("The TV took too long to answer. Try again.", e)
            } catch (e: java.io.IOException) {
                throw PrivilegedException(DROPPED, e)
            }
            AdbClient.Outcome.Refused -> throw PrivilegedException(UNREACHABLE)
            AdbClient.Outcome.Unknown, AdbClient.Outcome.Denied -> throw KeyNotApprovedException()
            is AdbClient.Outcome.TlsRequired -> throw PrivilegedException(TLS, IllegalStateException(o.detail))
            is AdbClient.Outcome.NotThisDevice -> throw PrivilegedException(NOT_THIS_TV, IllegalStateException(o.detail))
            is AdbClient.Outcome.Error -> throw PrivilegedException(DROPPED, IllegalStateException(o.detail))
        }
    }

    companion object {
        const val UNREACHABLE = "Lounge couldn't reach USB debugging. Check that it is still on in Developer options, then try again."
        const val DROPPED = "The connection to USB debugging dropped. Try again."
        const val TLS = "This TV asks for wireless debugging's pairing code, which Power Setup can't use. Turn on USB debugging in Developer options instead, then try again."
        const val NOT_THIS_TV = "Something other than this TV's debugging service answered, so Lounge didn't connect. Restart the TV, then try again."

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
class ShizukuBackend : Privileged {
    override val name: String = "shizuku"

    val installed: Boolean get() = runCatching { Shizuku.pingBinder() }.getOrDefault(false)

    val permitted: Boolean
        get() = runCatching { Shizuku.pingBinder() && Shizuku.checkSelfPermission() == PackageManager.PERMISSION_GRANTED }.getOrDefault(false)

    override suspend fun available(): Boolean = permitted

    fun requestPermission(requestCode: Int) {
        runCatching { Shizuku.requestPermission(requestCode) }
    }

    override suspend fun run(command: String, timeoutMs: Int): ShellResult = withContext(Dispatchers.IO) {
        if (!permitted) throw PrivilegedException("Shizuku hasn't allowed Lounge. Open Shizuku, allow Lounge, then try again.")
        val newProcess = Shizuku::class.java.getDeclaredMethod("newProcess", Array<String>::class.java, Array<String>::class.java, String::class.java)
        newProcess.isAccessible = true
        val process = newProcess.invoke(null, arrayOf("sh", "-c", command), null, null) as? Process
            ?: throw PrivilegedException("Shizuku couldn't run that. Open Shizuku and check that it is running.")
        try {
            coroutineScope {
                // Both streams drain at once, so a chatty stderr cannot fill its pipe and stall it.
                val stdout = async { process.inputStream.readBytes() }
                val stderr = async { process.errorStream.readBytes() }
                if (!process.waitFor(timeoutMs.toLong(), java.util.concurrent.TimeUnit.MILLISECONDS)) {
                    process.destroy()
                    throw PrivilegedException("The TV took too long to answer. Try again.")
                }
                val text = (stdout.await().toString(Charsets.UTF_8) + stderr.await().toString(Charsets.UTF_8)).trim()
                ShellResult(process.exitValue(), text)
            }
        } finally {
            runCatching { process.destroy() }
        }
    }
}
