package org.soos.remote

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.soos.remote.net.ApiJson
import org.soos.remote.net.AuthStateView
import org.soos.remote.net.LoginOptions
import org.soos.remote.net.StatusView

class ApiJsonTest {
    private val challenge = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8"

    @Test
    fun authState() {
        assertEquals(
            AuthStateView("funnel", false),
            ApiJson.parseAuthState("""{"mode":"funnel","authenticated":false}"""),
        )
        assertEquals(
            AuthStateView("tailnet", true),
            ApiJson.parseAuthState("""{"mode":"tailnet","authenticated":true,"passkeys":true}"""),
        )
        assertNull(ApiJson.parseAuthState("""{"mode":"other","authenticated":true}"""))
        assertNull(ApiJson.parseAuthState("""{"mode":"tailnet","authenticated":"true"}"""))
        assertNull(ApiJson.parseAuthState("not json"))
    }

    @Test
    fun status() {
        assertEquals(
            StatusView("locked", true, false, 1700000000000),
            ApiJson.parseStatus(
                """{"state":"locked","active":true,"idle":false,"idle_since_unix_s":null,"checked_unix_ms":1700000000000}""",
            ),
        )
        assertNull(ApiJson.parseStatus("""{"state":"open","active":true,"idle":false,"checked_unix_ms":1}"""))
        assertNull(ApiJson.parseStatus("""{"state":"locked","active":1,"idle":false,"checked_unix_ms":1}"""))
        assertNull(ApiJson.parseStatus("""{"state":"locked","active":true,"idle":false,"checked_unix_ms":"1"}"""))
    }

    @Test
    fun result() {
        assertEquals("lock_requested", ApiJson.parseResult("""{"result":"lock_requested"}"""))
        assertNull(ApiJson.parseResult("""{"result":"<script>"}"""))
        assertNull(ApiJson.parseResult("""{"result":7}"""))
    }

    @Test
    fun loginOptions() {
        val host = "pc.tail1234.ts.net"
        assertEquals(
            LoginOptions(challenge, host, 120000),
            ApiJson.parseLoginOptions(
                """{"challenge":"$challenge","rp_id":"$host","timeout_ms":120000}""",
                host,
            ),
        )
        assertNull(
            ApiJson.parseLoginOptions(
                """{"challenge":"$challenge","rp_id":"evil.tail1234.ts.net","timeout_ms":120000}""",
                host,
            ),
        )
        assertNull(
            ApiJson.parseLoginOptions("""{"challenge":"short","rp_id":"$host","timeout_ms":1}""", host),
        )
        assertNull(
            ApiJson.parseLoginOptions(
                """{"challenge":"$challenge","rp_id":"$host","timeout_ms":0}""",
                host,
            ),
        )
    }
}
