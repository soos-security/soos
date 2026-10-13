package org.soos.remote

import java.io.IOException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.soos.remote.net.ApiResult
import org.soos.remote.net.HttpRequest
import org.soos.remote.net.HttpResponse
import org.soos.remote.net.HttpTransport
import org.soos.remote.net.SoosApi

class SoosApiTest {
    private val origin = "https://pc.tail1234.ts.net"
    private val token = "B".repeat(43)

    private class Fake(val answers: ArrayDeque<HttpResponse>) : HttpTransport {
        val requests = mutableListOf<HttpRequest>()

        override fun execute(request: HttpRequest): HttpResponse {
            requests += request
            return answers.removeFirstOrNull() ?: throw IOException("no answer")
        }
    }

    private fun answer(status: Int, body: String, cookies: List<String> = emptyList()) =
        HttpResponse(status, body.encodeToByteArray(), cookies)

    private fun HttpRequest.header(name: String): String? =
        headers.firstOrNull { it.first.equals(name, ignoreCase = true) }?.second

    @Test
    fun lockCarriesOriginAndAction() {
        val fake = Fake(ArrayDeque(listOf(answer(202, """{"result":"lock_requested"}"""))))
        val api = SoosApi(origin, fake)
        assertEquals(ApiResult.Ok("lock_requested"), api.lock())
        val request = fake.requests.single()
        assertEquals("POST", request.method)
        assertEquals("$origin/api/lock", request.url)
        assertEquals(origin, request.header("Origin"))
        assertEquals("lock", request.header("X-Soos-Action"))
        assertNull(request.header("Cookie"))
        assertNull(request.body)
    }

    @Test
    fun loginStoresTheCookieInMemoryAndSendsItBack() {
        val fake = Fake(
            ArrayDeque(
                listOf(
                    answer(200, """{"result":"logged_in"}""", listOf("__Host-soos_session=$token; Path=/; Secure")),
                    answer(200, """{"state":"unlocked","active":true,"idle":false,"checked_unix_ms":5}"""),
                    answer(403, """{"result":"login_required"}"""),
                ),
            ),
        )
        val api = SoosApi(origin, fake)
        assertEquals(ApiResult.Ok("logged_in"), api.loginVerify("""{"id":"x"}"""))
        val login = fake.requests[0]
        assertEquals("login", login.header("X-Soos-Action"))
        assertEquals("application/json", login.header("Content-Type"))
        assertEquals("""{"id":"x"}""", login.body!!.decodeToString())
        assertTrue(api.hasSession)
        assertTrue(api.status() is ApiResult.Ok)
        assertEquals("__Host-soos_session=$token", fake.requests[1].header("Cookie"))
        assertNull(fake.requests[1].header("Origin"))
        assertEquals(ApiResult.Refused(403, "login_required"), api.status())
        assertFalse("login_required drops the session", api.hasSession)
    }

    @Test
    fun loginOptionsCheckTheRelyingParty() {
        val challenge = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8"
        val fake = Fake(
            ArrayDeque(
                listOf(
                    answer(200, """{"challenge":"$challenge","rp_id":"other.tail1234.ts.net","timeout_ms":120000}"""),
                    answer(409, """{"result":"no_passkey"}"""),
                ),
            ),
        )
        val api = SoosApi(origin, fake)
        assertEquals(ApiResult.Unreachable, api.loginOptions("pc.tail1234.ts.net"))
        assertEquals("login-options", fake.requests[0].header("X-Soos-Action"))
        assertEquals(origin, fake.requests[0].header("Origin"))
        assertEquals(ApiResult.Refused(409, "no_passkey"), api.loginOptions("pc.tail1234.ts.net"))
    }

    @Test
    fun networkErrorsAreUnreachable() {
        val api = SoosApi(origin, Fake(ArrayDeque()))
        assertEquals(ApiResult.Unreachable, api.authState())
        assertEquals(ApiResult.Unreachable, api.lock())
    }
}
