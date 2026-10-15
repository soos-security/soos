package org.soos.remote

import org.junit.Assert.assertEquals
import org.junit.Test
import org.soos.remote.config.PcUrl
import org.soos.remote.config.PcUrlError
import org.soos.remote.config.PcUrlResult

class PcUrlTest {
    private fun reason(input: String): PcUrlError? =
        (PcUrl.parse(input) as? PcUrlResult.Invalid)?.reason

    @Test
    fun validAddressesAreNormalised() {
        for (input in listOf(
            "https://pc.tail1234.ts.net",
            "  https://pc.tail1234.ts.net/  ",
            "HTTPS://PC.Tail1234.TS.NET",
        )) {
            assertEquals(
                input,
                PcUrlResult.Valid("https://pc.tail1234.ts.net", "pc.tail1234.ts.net"),
                PcUrl.parse(input),
            )
        }
    }

    @Test
    fun everyRuleRefusesWithItsReason() {
        assertEquals(PcUrlError.EMPTY, reason("   "))
        assertEquals(PcUrlError.TOO_LONG, reason("https://" + "a".repeat(PcUrl.MAX_INPUT_LENGTH)))
        assertEquals(PcUrlError.NOT_HTTPS, reason("http://pc.tail1234.ts.net"))
        assertEquals(PcUrlError.NOT_HTTPS, reason("pc.tail1234.ts.net"))
        assertEquals(PcUrlError.USERINFO, reason("https://user@pc.tail1234.ts.net"))
        assertEquals(PcUrlError.PORT, reason("https://pc.tail1234.ts.net:443"))
        assertEquals(PcUrlError.PATH, reason("https://pc.tail1234.ts.net/api"))
        assertEquals(PcUrlError.PATH, reason("https://pc.tail1234.ts.net//"))
        assertEquals(PcUrlError.QUERY_OR_FRAGMENT, reason("https://pc.tail1234.ts.net?x=1"))
        assertEquals(PcUrlError.QUERY_OR_FRAGMENT, reason("https://pc.tail1234.ts.net#x"))
        assertEquals(PcUrlError.INVALID_HOST, reason("https://pc..ts.net"))
        assertEquals(PcUrlError.INVALID_HOST, reason("https://-pc.tail.ts.net"))
        assertEquals(PcUrlError.INVALID_HOST, reason("https://p_c.tail.ts.net"))
        assertEquals(PcUrlError.INVALID_HOST, reason("https://" + "a".repeat(64) + ".tail.ts.net"))
        assertEquals(PcUrlError.NOT_TS_NET, reason("https://pc.example.com"))
        assertEquals(PcUrlError.NOT_TS_NET, reason("https://tail.ts.net"))
        assertEquals(PcUrlError.NOT_TS_NET, reason("https://pc.tail.ts.net.evil.com"))
    }
}
