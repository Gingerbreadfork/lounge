package tv.lounge.power

import java.io.File
import java.math.BigInteger
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.security.KeyFactory
import java.security.KeyPair
import java.security.KeyPairGenerator
import java.security.MessageDigest
import java.security.Signature
import java.security.interfaces.RSAPublicKey
import java.security.spec.PKCS8EncodedKeySpec
import java.security.spec.X509EncodedKeySpec
import android.util.Base64

/**
 * The launcher's own ADB key pair, stored in app-private files. ADB signs the daemon's 20-byte
 * token as an RSA PKCS#1 v1.5 signature with a SHA-1 DigestInfo prefix, and announces the public
 * key in Android's `RSAPublicKey` layout (little-endian modulus with Montgomery constants).
 */
class AdbKeys(private val dir: File) {
    private val privateFile = File(dir, "adbkey")
    private val publicFile = File(dir, "adbkey.pub")

    @Volatile private var cached: KeyPair? = null

    val exists: Boolean get() = privateFile.exists() && publicFile.exists()

    fun keyPair(): KeyPair = cached ?: synchronized(this) {
        cached ?: (load() ?: generate()).also { cached = it }
    }

    fun sign(token: ByteArray): ByteArray {
        val sig = Signature.getInstance("NONEwithRSA")
        sig.initSign(keyPair().private)
        sig.update(SHA1_DIGEST_INFO)
        sig.update(token)
        return sig.sign()
    }

    /** The AUTH payload adb sends: base64 key, a space, a name, and a terminating NUL. */
    fun publicKeyPayload(): ByteArray {
        val encoded = Base64.encodeToString(encodeAndroidPublicKey(keyPair().public as RSAPublicKey), Base64.NO_WRAP)
        return "$encoded lounge@tv\u0000".toByteArray(Charsets.US_ASCII)
    }

    /** What the device's prompt shows for this key: an MD5 over the decoded key bytes, colon separated. */
    fun fingerprint(): String {
        val md5 = MessageDigest.getInstance("MD5").digest(encodeAndroidPublicKey(keyPair().public as RSAPublicKey))
        return md5.joinToString(":") { "%02X".format(it) }
    }

    fun forget() {
        synchronized(this) {
            privateFile.delete()
            publicFile.delete()
            cached = null
        }
    }

    private fun load(): KeyPair? = runCatching {
        if (!exists) return null
        val kf = KeyFactory.getInstance("RSA")
        val priv = kf.generatePrivate(PKCS8EncodedKeySpec(privateFile.readBytes()))
        val pub = kf.generatePublic(X509EncodedKeySpec(publicFile.readBytes()))
        KeyPair(pub, priv)
    }.getOrNull()

    private fun generate(): KeyPair {
        val gen = KeyPairGenerator.getInstance("RSA")
        gen.initialize(2048)
        val pair = gen.generateKeyPair()
        dir.mkdirs()
        privateFile.writeBytes(pair.private.encoded)
        publicFile.writeBytes(pair.public.encoded)
        return pair
    }

    companion object {
        private val SHA1_DIGEST_INFO = byteArrayOf(
            0x30, 0x21, 0x30, 0x09, 0x06, 0x05, 0x2b, 0x0e, 0x03, 0x02, 0x1a, 0x05, 0x00, 0x04, 0x14,
        )
        private const val MODULUS_BYTES = 256

        fun encodeAndroidPublicKey(key: RSAPublicKey): ByteArray {
            val n = key.modulus
            val r32 = BigInteger.ONE.shiftLeft(32)
            val n0inv = r32.subtract(n.modInverse(r32))
            val rr = BigInteger.ONE.shiftLeft(MODULUS_BYTES * 8 * 2).mod(n)
            val buf = ByteBuffer.allocate(4 + 4 + MODULUS_BYTES + MODULUS_BYTES + 4).order(ByteOrder.LITTLE_ENDIAN)
            buf.putInt(MODULUS_BYTES / 4)
            buf.putInt(n0inv.toInt())
            buf.put(littleEndian(n, MODULUS_BYTES))
            buf.put(littleEndian(rr, MODULUS_BYTES))
            buf.putInt(key.publicExponent.toInt())
            return buf.array()
        }

        private fun littleEndian(value: BigInteger, size: Int): ByteArray {
            val be = value.toByteArray()
            val out = ByteArray(size)
            var i = be.size - 1
            var o = 0
            while (i >= 0 && o < size) {
                out[o++] = be[i--]
            }
            return out
        }
    }
}
