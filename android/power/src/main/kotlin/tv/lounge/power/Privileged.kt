package tv.lounge.power

/** Where the shell-level helper stands right now. */
sealed interface PowerStatus {
    /** Neither the device's ADB daemon nor Shizuku can be reached. */
    data class Unavailable(val reason: String) : PowerStatus
    /** ADB is listening but has not accepted this launcher's key yet. */
    data object NotPaired : PowerStatus
    /** The device is showing its "Allow debugging?" prompt for our key. */
    data object WaitingForApproval : PowerStatus
    data class Ready(val backend: String) : PowerStatus
    data class Failed(val reason: String) : PowerStatus
}

data class ShellResult(val exitCode: Int, val output: String) {
    val ok: Boolean get() = exitCode == 0
}

open class PrivilegedException(message: String, cause: Throwable? = null) : Exception(message, cause)

/** The daemon answered but does not know Lounge's key: the one failure that means pairing again. */
class KeyNotApprovedException : PrivilegedException("This device has not approved Lounge's key")

/** One way of running a shell command with shell-user rights. */
interface Privileged {
    val name: String
    suspend fun available(): Boolean
    suspend fun run(command: String, timeoutMs: Int = DEFAULT_TIMEOUT_MS): ShellResult

    companion object {
        const val DEFAULT_TIMEOUT_MS = 20_000
    }
}
