package tv.lounge.feature.home

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The ways to give. Money sent to a mistyped address is gone, so each address is held to the
 * checksum its own coin builds into it, and to the scheme wallets read.
 */
class DonateTest {
    private val letters = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"

    /** Whether a bech32 address, as Bitcoin and DigiByte write theirs, passes its checksum. */
    private fun bech32(address: String): Boolean {
        val prefix = address.substringBeforeLast('1')
        val data = address.substringAfterLast('1').map { letters.indexOf(it) }
        if (data.any { it < 0 }) return false
        val values = prefix.map { it.code shr 5 } + 0 + prefix.map { it.code and 31 } + data
        val generators = intArrayOf(0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3)
        var check = 1
        for (v in values) {
            val top = check ushr 25
            check = ((check and 0x1ffffff) shl 5) xor v
            for (i in 0 until 5) if ((top ushr i) and 1 == 1) check = check xor generators[i]
        }
        return check == 1
    }

    /** Whether a Bitcoin Cash address passes its checksum, taken with the prefix it is written under. */
    private fun cashAddress(prefix: String, address: String): Boolean {
        val data = address.map { letters.indexOf(it) }
        if (data.any { it < 0 }) return false
        val generators = longArrayOf(0x98f2bc8e61, 0x79b76d99e2, 0xf33e5fb3c4, 0xae2eabe2a8, 0x1e4f43e470)
        var check = 1L
        for (v in prefix.map { it.code and 31 } + 0 + data) {
            val top = check ushr 35
            check = ((check and 0x07ffffffffL) shl 5) xor v.toLong()
            for (i in 0 until 5) if ((top ushr i) and 1L == 1L) check = check xor generators[i]
        }
        return check xor 1L == 0L
    }

    private fun gift(name: String) = Gifts.single { it.name == name }

    @Test
    fun everyAddressPassesItsOwnChecksum() {
        // The way most people can use comes first, so it is the code the page opens on.
        assertEquals(listOf("PayPal", "Bitcoin", "DigiByte", "Bitcoin Cash"), Gifts.map { it.name })
        assertEquals("bitcoin:bc1qlezapstmtq6jm4cj6ahn0ylyyfnpz79l2zegjv", gift("Bitcoin").code)
        assertEquals("digibyte:dgb1qrf5m26fjs57rs4lehdm95ycz5h8py94n3hcw2h", gift("DigiByte").code)
        assertEquals("bitcoincash:qqn2ymq3qwp7987cjlllgyp56dk3m7t89vu8u48sxk", gift("Bitcoin Cash").code)
        assertEquals("https://paypal.me/gingerbreadfork", gift("PayPal").code)

        assertTrue(bech32(gift("Bitcoin").code.substringAfter(':')))
        assertTrue(bech32(gift("DigiByte").code.substringAfter(':')))
        assertTrue(cashAddress("bitcoincash", gift("Bitcoin Cash").code.substringAfter(':')))
        // The checks really do catch one wrong character.
        assertFalse(bech32("bc1qlezapstmtq6jm4cj6ahn0ylyyfnpz79l2zegjx"))
        assertFalse(cashAddress("bitcoincash", "qqn2ymq3qwp7987cjlllgyp56dk3m7t89vu8u48sxq"))
    }

    @Test
    fun whatIsWrittenUnderACodeIsTheAddressInTheCode() {
        assertEquals("bc1q leza pstm tq6j m4cj 6ahn\n0yly yfnp z79l 2zeg jv", gift("Bitcoin").shown)
        for (coin in Gifts.filter { it.name != "PayPal" }) {
            assertEquals(coin.code.substringAfter(':'), coin.shown.filterNot { it.isWhitespace() })
            // Each says which coin alone the address takes.
            assertEquals("Scan with a wallet.\nSend only ${coin.name} to this address.", coin.note)
        }
        assertEquals("https://${gift("PayPal").shown}", gift("PayPal").code)
        assertEquals("abcd efgh\nij", spaced("abcdefghij"))
        assertEquals("abc", spaced("abc"))
    }

    @Test
    fun theAskSaysHowLongLoungeHasBeenHereOnlyOnceItHasBeenAWhile() {
        val day = 86_400_000L
        assertEquals(null, hadFor(null, 0))
        assertEquals(null, hadFor(0, 6 * day))
        assertEquals("1 week", hadFor(0, 7 * day))
        assertEquals("3 weeks", hadFor(0, 24 * day))
        assertEquals("2 months", hadFor(0, 60 * day))
        assertEquals("8 months", hadFor(0, 250 * day))
        assertEquals("over a year", hadFor(0, 400 * day))
        assertEquals("over 2 years", hadFor(0, 800 * day))

        assertTrue(donateAsk("8 months").startsWith("You've had Lounge on this TV for 8 months. If it has earned its place,"))
        // A TV that has only just met Lounge is asked without being told it owes anything.
        assertTrue(donateAsk(null).startsWith("If it has earned its place,"))
        assertTrue(donateAsk(null).endsWith("absolutely anything helps."))
    }
}
