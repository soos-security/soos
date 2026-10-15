package org.soos.remote

import org.junit.Assert.assertEquals
import org.junit.Test
import org.soos.remote.net.CookieUpdate
import org.soos.remote.net.SessionCookie

class SessionCookieTest {
    private val token = "A".repeat(42) + "_"

    @Test
    fun setClearedAndIgnored() {
        assertEquals(
            CookieUpdate.Set(token),
            SessionCookie.fromSetCookie(
                listOf("__Host-soos_session=$token; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=28800"),
            ),
        )
        assertEquals(
            CookieUpdate.Cleared,
            SessionCookie.fromSetCookie(listOf("__Host-soos_session=; Path=/; Max-Age=0")),
        )
        assertEquals(CookieUpdate.None, SessionCookie.fromSetCookie(emptyList()))
        assertEquals(CookieUpdate.None, SessionCookie.fromSetCookie(listOf("other=$token")))
        assertEquals(
            CookieUpdate.None,
            SessionCookie.fromSetCookie(listOf("__Host-soos_session=short; Path=/")),
        )
        assertEquals(
            CookieUpdate.None,
            SessionCookie.fromSetCookie(listOf("__Host-soos_session=${"A".repeat(42)}+; Path=/")),
        )
        assertEquals("__Host-soos_session=$token", SessionCookie.header(token))
    }
}
