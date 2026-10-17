package org.soos.remote

import java.io.IOException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotSame
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import org.soos.remote.net.ApiHolder
import org.soos.remote.net.ApiResult
import org.soos.remote.net.HttpRequest
import org.soos.remote.net.HttpResponse
import org.soos.remote.net.HttpTransport

class SharedApiTest {
    private val origin = "https://pc.tail1234.ts.net"
    private val other = "https://other.tail1234.ts.net"
    private val token = "C".repeat(43)

    private class Fake : HttpTransport {
        val requests = mutableListOf<HttpRequest>()
        val answers = ArrayDeque<HttpResponse>()

        override fun execute(request: HttpRequest): HttpResponse {
            requests += request
            return answers.removeFirstOrNull() ?: throw IOException("no answer")
        }
    }

    @Test
    fun oneInstancePerOriginSharesTheInMemorySession() {
        val fake = Fake()
        val holder = ApiHolder { fake }
        val ui = holder.forOrigin(origin)
        fake.answers += HttpResponse(
            200,
            """{"result":"logged_in"}""".encodeToByteArray(),
            listOf("__Host-soos_session=$token; Path=/; Secure"),
        )
        assertEquals(ApiResult.Ok("logged_in"), ui.loginVerify("""{"id":"x"}"""))
        val job = holder.forOrigin(origin)
        assertSame(ui, job)
        assertTrue(job.hasSession)
        fake.answers += HttpResponse(
            200,
            """{"state":"locked","active":true,"idle":false,"checked_unix_ms":9}""".encodeToByteArray(),
            emptyList(),
        )
        assertTrue(job.status() is ApiResult.Ok)
        val status = fake.requests.last()
        assertEquals("GET", status.method)
        assertEquals("$origin/api/status", status.url)
        assertTrue(status.headers.contains("Cookie" to "__Host-soos_session=$token"))
    }

    @Test
    fun aLateRefusalOfARequestSentWithoutASessionKeepsANewSession() {
        // The job sends a status request without a session; before its 403 arrives, the
        // screen signs in. The late refusal must not sign the user out.
        lateinit var ui: org.soos.remote.net.SoosApi
        val login = HttpResponse(
            200,
            """{"result":"logged_in"}""".encodeToByteArray(),
            listOf("__Host-soos_session=$token"),
        )
        val racing = HttpTransport { request ->
            if (request.url.endsWith("/api/status")) {
                // The sign-in completes while the status request is in flight.
                assertEquals(ApiResult.Ok("logged_in"), ui.loginVerify("{}"))
                HttpResponse(403, """{"result":"login_required"}""".encodeToByteArray(), emptyList())
            } else {
                login
            }
        }
        ui = ApiHolder { racing }.forOrigin(origin)
        assertFalse(ui.hasSession)
        assertEquals(ApiResult.Refused(403, "login_required"), ui.status())
        assertTrue("the session stored meanwhile is kept", ui.hasSession)
    }

    @Test
    fun anotherOriginDropsTheSession() {
        val fake = Fake()
        val holder = ApiHolder { fake }
        val first = holder.forOrigin(origin)
        fake.answers += HttpResponse(
            200,
            """{"result":"logged_in"}""".encodeToByteArray(),
            listOf("__Host-soos_session=$token"),
        )
        first.loginVerify("{}")
        assertTrue(first.hasSession)
        val second = holder.forOrigin(other)
        assertNotSame(first, second)
        assertFalse(second.hasSession)
        // Coming back to the first address starts without the old session too.
        val back = holder.forOrigin(origin)
        assertNotSame(first, back)
        assertFalse(back.hasSession)
    }
}
