package tv.lounge.power

import android.content.pm.PackageManager
import java.io.InputStream
import kotlinx.coroutines.Dispatchers
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
            } catch (e: java.io.IOException) {
                throw PrivilegedException(e.message ?: "the ADB connection was lost", e)
            }
            AdbClient.Outcome.Refused -> throw PrivilegedException("ADB is not listening on this device")
            AdbClient.Outcome.Unknown, AdbClient.Outcome.Denied -> throw PrivilegedException("This device has not approved Lounge's key")
            is AdbClient.Outcome.TlsRequired -> throw PrivilegedException(o.detail)
            is AdbClient.Outcome.Error -> throw PrivilegedException(o.detail)
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
        if (!permitted) throw PrivilegedException("Shizuku has not granted Lounge access")
        val newProcess = Shizuku::class.java.getDeclaredMethod("newProcess", Array<String>::class.java, Array<String>::class.java, String::class.java)
        newProcess.isAccessible = true
        val process = newProcess.invoke(null, arrayOf("sh", "-c", command), null, null)
            ?: throw PrivilegedException("Shizuku could not start a shell")
        val cls = process.javaClass
        val stdout = (cls.getMethod("getInputStream").invoke(process) as InputStream).readBytes().toString(Charsets.UTF_8)
        val stderr = (cls.getMethod("getErrorStream").invoke(process) as InputStream).readBytes().toString(Charsets.UTF_8)
        val code = cls.getMethod("waitFor").invoke(process) as Int
        runCatching { cls.getMethod("destroy").invoke(process) }
        ShellResult(code, (stdout + stderr).trim())
    }
}
