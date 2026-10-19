package tv.lounge.power

import java.io.DataInputStream
import java.io.EOFException
import java.io.IOException
import java.io.OutputStream
import java.net.InetSocketAddress
import java.net.Socket
import java.net.SocketTimeoutException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.atomic.AtomicLong

/**
 * A minimal ADB transport client that talks to the device's own daemon over TCP. It speaks the
 * classic protocol: CNXN, RSA AUTH (signature, then public key when the key is unknown), and
 * the legacy `shell:` service over one OPEN/WRTE/OKAY/CLSE stream per command.
 */
class AdbClient(private val keys: AdbKeys, private val host: String = "127.0.0.1", private val port: Int = 5555) {

    sealed interface Outcome {
        data class Online(val connection: Connection) : Outcome
        data object Refused : Outcome
        /** The daemon does not know our key and we were asked not to present it. */
        data object Unknown : Outcome
        data object Denied : Outcome
        data class TlsRequired(val detail: String) : Outcome
        data class Error(val detail: String) : Outcome
    }

    private data class Message(val command: Int, val arg0: Int, val arg1: Int, val data: ByteArray)

    class Connection internal constructor(private val socket: Socket, private val input: DataInputStream, private val output: OutputStream) : AutoCloseable {
        private var nextLocalId = 1

        /** Runs `command` with the daemon's shell user and returns everything it printed. */
        fun shell(command: String, timeoutMs: Int = 20_000): String {
            socket.soTimeout = timeoutMs
            val localId = nextLocalId++
            write(output, A_OPEN, localId, 0, "shell:$command\u0000".toByteArray(Charsets.UTF_8))
            var remoteId = 0
            val out = java.io.ByteArrayOutputStream()
            while (true) {
                val m = read(input)
                when (m.command) {
                    A_OKAY -> remoteId = m.arg0
                    A_WRTE -> {
                        if (m.arg1 == localId) {
                            out.write(m.data)
                            write(output, A_OKAY, localId, m.arg0, EMPTY)
                        }
                    }
                    A_CLSE -> {
                        if (m.arg1 == localId || m.arg0 == remoteId) {
                            write(output, A_CLSE, localId, m.arg0, EMPTY)
                            return out.toString(Charsets.UTF_8.name())
                        }
                    }
                    else -> Unit
                }
            }
        }

        override fun close() {
            runCatching { socket.close() }
        }
    }

    /**
     * Connects and authenticates. When the daemon does not know our key it sends a second token;
     * with `presentKey` we then send the public key, `onWaitingForUser` fires, and the device
     * shows its prompt for up to `approvalTimeoutMs`. Without it the attempt ends as [Outcome.Unknown].
     */
    fun connect(
        presentKey: Boolean = true,
        onWaitingForUser: () -> Unit = {},
        connectTimeoutMs: Int = 3_000,
        approvalTimeoutMs: Int = 120_000,
        giveUpAt: AtomicLong = AtomicLong(Long.MAX_VALUE),
    ): Outcome {
        val socket = Socket()
        try {
            socket.tcpNoDelay = true
            socket.connect(InetSocketAddress(host, port), connectTimeoutMs)
        } catch (e: IOException) {
            runCatching { socket.close() }
            return Outcome.Refused
        }
        val input = DataInputStream(socket.getInputStream().buffered())
        val output = socket.getOutputStream()
        try {
            socket.soTimeout = 8_000
            write(output, A_CNXN, A_VERSION, MAX_PAYLOAD, "host::\u0000".toByteArray(Charsets.US_ASCII))
            var tokens = 0
            val deadline = System.currentTimeMillis() + approvalTimeoutMs
            while (true) {
                val m = try {
                    read(input)
                } catch (e: SocketTimeoutException) {
                    // Past the second token we are waiting on the person; silence until the
                    // deadline means they never answered (or declined), not a broken link.
                    if (tokens >= 2) {
                        val now = System.currentTimeMillis()
                        if (now < deadline && now < giveUpAt.get()) continue
                        socket.close()
                        return Outcome.Denied
                    }
                    throw e
                }
                when (m.command) {
                    A_AUTH -> {
                        if (m.arg0 != AUTH_TOKEN) continue
                        tokens++
                        when (tokens) {
                            1 -> write(output, A_AUTH, AUTH_SIGNATURE, 0, keys.sign(m.data))
                            2 -> {
                                if (!presentKey) {
                                    socket.close()
                                    return Outcome.Unknown
                                }
                                write(output, A_AUTH, AUTH_RSAPUBLICKEY, 0, keys.publicKeyPayload())
                                socket.soTimeout = 1_000
                                onWaitingForUser()
                            }
                            else -> Unit
                        }
                    }
                    A_CNXN -> {
                        socket.soTimeout = 20_000
                        return Outcome.Online(Connection(socket, input, output))
                    }
                    A_STLS -> {
                        socket.close()
                        return Outcome.TlsRequired("the daemon asked for TLS; turn on plain network debugging")
                    }
                    A_CLSE -> {
                        socket.close()
                        return Outcome.Denied
                    }
                    else -> Unit
                }
            }
        } catch (e: EOFException) {
            runCatching { socket.close() }
            return Outcome.Denied
        } catch (e: IOException) {
            runCatching { socket.close() }
            return Outcome.Error(e.message ?: e.javaClass.simpleName)
        }
    }

    companion object {
        private const val A_CNXN = 0x4e584e43
        private const val A_AUTH = 0x48545541
        private const val A_OPEN = 0x4e45504f
        private const val A_OKAY = 0x59414b4f
        private const val A_CLSE = 0x45534c43
        private const val A_WRTE = 0x45545257
        private const val A_STLS = 0x534c5453
        private const val A_VERSION = 0x01000001
        private const val MAX_PAYLOAD = 256 * 1024
        private const val AUTH_TOKEN = 1
        private const val AUTH_SIGNATURE = 2
        private const val AUTH_RSAPUBLICKEY = 3
        private val EMPTY = ByteArray(0)

        private fun write(out: OutputStream, command: Int, arg0: Int, arg1: Int, data: ByteArray) {
            var check = 0
            for (b in data) check += b.toInt() and 0xff
            val header = ByteBuffer.allocate(24).order(ByteOrder.LITTLE_ENDIAN)
            header.putInt(command).putInt(arg0).putInt(arg1).putInt(data.size).putInt(check).putInt(command xor -1)
            synchronized(out) {
                out.write(header.array())
                if (data.isNotEmpty()) out.write(data)
                out.flush()
            }
        }

        private fun read(input: DataInputStream): Message {
            val header = ByteArray(24)
            input.readFully(header)
            val b = ByteBuffer.wrap(header).order(ByteOrder.LITTLE_ENDIAN)
            val command = b.int
            val arg0 = b.int
            val arg1 = b.int
            val length = b.int
            b.int
            val magic = b.int
            if (magic != (command xor -1)) throw IOException("bad adb packet")
            if (length < 0 || length > MAX_PAYLOAD) throw IOException("adb packet too large: $length")
            val data = ByteArray(length)
            if (length > 0) input.readFully(data)
            return Message(command, arg0, arg1, data)
        }
    }
}
