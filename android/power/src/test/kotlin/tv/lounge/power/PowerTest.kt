package tv.lounge.power

import java.math.BigInteger
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.security.KeyPairGenerator
import java.security.interfaces.RSAPublicKey
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class PowerTest {
    @Test
    fun androidPublicKeyLayoutMatchesAdb() {
        val gen = KeyPairGenerator.getInstance("RSA")
        gen.initialize(2048)
        val pub = gen.generateKeyPair().public as RSAPublicKey
        val bytes = AdbKeys.encodeAndroidPublicKey(pub)
        assertEquals(4 + 4 + 256 + 256 + 4, bytes.size)

        val b = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        assertEquals(64, b.int)
        val n0inv = b.int.toLong() and 0xffffffffL
        val modulus = ByteArray(256).also { b.get(it) }
        val rr = ByteArray(256).also { b.get(it) }
        assertEquals(65537, b.int)

        val n = BigInteger(1, modulus.reversedArray())
        assertEquals(pub.modulus, n)
        // n0inv is -n^-1 mod 2^32: n * n0inv + 1 must be divisible by 2^32.
        val r32 = BigInteger.ONE.shiftLeft(32)
        assertEquals(BigInteger.ZERO, n.multiply(BigInteger.valueOf(n0inv)).add(BigInteger.ONE).mod(r32))
        assertEquals(BigInteger.ONE.shiftLeft(4096).mod(n), BigInteger(1, rr.reversedArray()))
    }

    @Test
    fun exitCodeMarkerIsSplitOff() {
        val r = AdbBackend.parseResult("Success\r\n__rc=0\r\n")
        assertTrue(r.ok)
        assertEquals("Success", r.output)
        val f = AdbBackend.parseResult("Error: unknown command\n__rc=255\n")
        assertEquals(255, f.exitCode)
        assertEquals("Error: unknown command", f.output)
        assertEquals(-1, AdbBackend.parseResult("no marker").exitCode)
    }

    @Test
    fun onlyThisDevicesDaemonIsTrusted() {
        val banner = "device::ro.product.name=kirkwood;ro.product.model=Google TV Streamer;ro.product.device=kirkwood;features=shell_v2,cmd"
        assertTrue(AdbClient.bannerIsThisDevice(banner, "kirkwood", "Google TV Streamer", "kirkwood"))
        assertFalse(AdbClient.bannerIsThisDevice(banner, "kirkwood", "Other TV", "kirkwood"))
        assertFalse(AdbClient.bannerIsThisDevice("device::features=shell_v2", "kirkwood", "Google TV Streamer", "kirkwood"))
        assertFalse(AdbClient.bannerIsThisDevice("host::ro.product.name=kirkwood", "kirkwood", "Google TV Streamer", "kirkwood"))

        assertEquals(true, AdbClient.tcpPortMatches("5555", null, 5555))
        assertEquals(true, AdbClient.tcpPortMatches(null, "5555", 5555))
        assertEquals(false, AdbClient.tcpPortMatches("-1", "5555", 5555))
        assertEquals(false, AdbClient.tcpPortMatches("7000", null, 5555))
        assertNull(AdbClient.tcpPortMatches(null, null, 5555))
    }

    @Test
    fun theScreensaverFlagIsReadFromTheActionOutput() {
        assertEquals("0", PowerSetup.previousScreensaverEnabled("0"))
        assertEquals("null", PowerSetup.previousScreensaverEnabled("null\n"))
        assertNull(PowerSetup.previousScreensaverEnabled("Error: permission denied"))
        assertNull(PowerSetup.previousScreensaverEnabled(""))
    }

    @Test
    fun cecDevicesComeOutOfDumpsys() {
        val dump = """
            mDeviceInfos:
                CEC: logical_address: 0x00 device_type: 0 cec_version: 5 vendor_id: 7173227 display_name: Living Room TV power_status: 1 physical_address: 0x0000 port_id: -1
                CEC: logical_address: 0x08 device_type: 4 cec_version: 6 vendor_id: 6673 display_name: Living Room TV power_status: 0 physical_address: 0x4000 port_id: 0
                CEC: logical_address: 0x04 device_type: 4 cec_version: 5 vendor_id: 1 display_name: PlayStation 5 power_status: 1 physical_address: 0x1000 port_id: 1
        """.trimIndent()
        val devices = PowerSetup.parseCecDevices(dump)
        assertEquals(listOf(0, 4, 8), devices.map { it.logicalAddress })
        assertTrue(devices[0].isTv)
        assertEquals("PlayStation 5", devices[1].name)
        assertEquals(0x1000, devices[1].physicalAddress)
        assertTrue(devices[2].isOn)
    }

    @Test
    fun googleTvsOwnHomesComeBeforeLounge() {
        val homes = listOf(
            HomeApp("com.google.android.apps.tv.launcherx", "Google TV Home", priority = 2, enabled = true, appEnabled = true),
            HomeApp("com.google.android.tungsten.setupwraith", "Setup", priority = 1, enabled = true, appEnabled = true),
            HomeApp("com.example.launcher", "Another launcher", priority = 0, enabled = true, appEnabled = true),
            HomeApp("com.google.android.tvrecommendations", "Old home", priority = 3, enabled = false, appEnabled = false),
        )
        assertEquals(listOf("com.google.android.apps.tv.launcherx", "com.google.android.tungsten.setupwraith"), PowerSetup.outranking(homes, ownPriority = 0).map { it.packageName })
        assertEquals(emptyList<HomeApp>(), PowerSetup.outranking(homes, ownPriority = 3))
    }

    @Test
    fun onlyWholeAppsSwitchedOffAreOfferedBack() {
        val homes = listOf(
            HomeApp("com.google.android.apps.tv.launcherx", "Google TV Home", priority = 2, enabled = false, appEnabled = false),
            HomeApp("com.example.setup", "Setup", priority = 1, enabled = false, appEnabled = true),
        )
        assertEquals(listOf("com.google.android.apps.tv.launcherx"), PowerSetup.switchedOff(homes).map { it.packageName })
    }

    @Test
    fun homeCommandsNameTheUserAndQuoteThePackage() {
        assertEquals("pm disable-user --user 0 'com.google.android.apps.tv.launcherx'", PowerSetup.disableCommand("com.google.android.apps.tv.launcherx", 0))
        assertEquals("pm enable --user 10 'a'\\''b'", PowerSetup.enableCommand("a'b", 10))
    }
}
