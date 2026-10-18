package org.soos.remote

import java.io.IOException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.soos.remote.net.AlertRecord
import org.soos.remote.net.AlertsView
import org.soos.remote.net.ApiResult
import org.soos.remote.net.BatteryView
import org.soos.remote.net.HttpRequest
import org.soos.remote.net.HttpResponse
import org.soos.remote.net.HttpTransport
import org.soos.remote.net.OfflineInterval
import org.soos.remote.net.OfflineView
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

    // --- failed-password alerts (GitHub #404, matrix ANA19) ---------------------------------

    private val epoch = "0123456789abcdef"
    private val alertsJson =
        """{"state":"active","reason":null,"epoch":"$epoch","lock_screen":"monitored",""" +
            """"unacknowledged_wrong_password":2,"unacknowledged_locked_out":0,""" +
            """"last_unix_ms":1700000000000,"last_source":"sudo","through":9,"history":[""" +
            """{"id":8,"first_unix_ms":1699999999000,"last_unix_ms":1700000000000,""" +
            """"source":"sudo","account":"root","kind":"wrong_password","count":2}]}"""
    private val alertsView = AlertsView(
        "active", null, epoch, "monitored", 2, 0, 1700000000000, "sudo", 9,
        listOf(AlertRecord(8, 1699999999000, 1700000000000, "sudo", "root", "wrong_password", 2)),
    )
    private val clearedJson =
        """{"state":"active","reason":null,"epoch":"$epoch","lock_screen":"monitored",""" +
            """"unacknowledged_wrong_password":0,"unacknowledged_locked_out":0,""" +
            """"last_unix_ms":null,"last_source":null,"through":9,"history":[]}"""

    private fun HttpRequest.headerCount(name: String): Int =
        headers.count { it.first.equals(name, ignoreCase = true) }

    @Test
    fun alertsIsAPlainGet() {
        val fake = Fake(ArrayDeque(listOf(answer(200, alertsJson))))
        val api = SoosApi(origin, fake)
        assertEquals(ApiResult.Ok(alertsView), api.alerts())
        val request = fake.requests.single()
        assertEquals("GET", request.method)
        assertEquals("$origin/api/alerts", request.url)
        assertEquals("application/json", request.header("Accept"))
        assertNull(request.header("Origin"))
        assertNull(request.header("X-Soos-Action"))
        assertNull(request.header("X-Soos-Alerts-Epoch"))
        assertNull(request.header("X-Soos-Alerts-Through"))
        assertNull(request.header("Cookie"))
        assertNull(request.body)
    }

    @Test
    fun ackCarriesExactlyTheSnapshotHeadersAndNoBody() {
        val fake = Fake(ArrayDeque(listOf(answer(200, clearedJson))))
        val api = SoosApi(origin, fake)
        val cleared = api.ackAlerts(epoch, 9)
        assertEquals(
            ApiResult.Ok(AlertsView("active", null, epoch, "monitored", 0, 0, null, null, 9, emptyList())),
            cleared,
        )
        val request = fake.requests.single()
        assertEquals("POST", request.method)
        assertEquals("$origin/api/alerts/ack", request.url)
        assertNull("the acknowledgement has no body", request.body)
        assertNull(request.header("Content-Type"))
        assertEquals(1, request.headerCount("Origin"))
        assertEquals(origin, request.header("Origin"))
        assertEquals(1, request.headerCount("X-Soos-Action"))
        assertEquals("alerts-ack", request.header("X-Soos-Action"))
        assertEquals(SoosApi.ACTION_ALERTS_ACK, "alerts-ack")
        assertEquals(1, request.headerCount("X-Soos-Alerts-Epoch"))
        assertEquals(epoch, request.header("X-Soos-Alerts-Epoch"))
        assertEquals(1, request.headerCount("X-Soos-Alerts-Through"))
        assertEquals("9", request.header("X-Soos-Alerts-Through"))
        assertEquals("application/json", request.header("Accept"))
        assertNull(request.header("Cookie"))
        assertEquals(
            listOf("Accept", "Origin", "X-Soos-Action", "X-Soos-Alerts-Epoch", "X-Soos-Alerts-Through"),
            request.headers.map { it.first },
        )
    }

    @Test
    fun alertsAndAckSendTheSessionCookie() {
        val fake = Fake(
            ArrayDeque(
                listOf(
                    answer(200, """{"result":"logged_in"}""", listOf("__Host-soos_session=$token; Path=/; Secure")),
                    answer(200, alertsJson),
                    answer(200, clearedJson),
                ),
            ),
        )
        val api = SoosApi(origin, fake)
        assertEquals(ApiResult.Ok("logged_in"), api.loginVerify("""{"id":"x"}"""))
        assertEquals(ApiResult.Ok(alertsView), api.alerts())
        assertEquals("__Host-soos_session=$token", fake.requests[1].header("Cookie"))
        assertTrue(api.ackAlerts(epoch, 9) is ApiResult.Ok)
        assertEquals("__Host-soos_session=$token", fake.requests[2].header("Cookie"))
        assertEquals(1, fake.requests[2].headerCount("Cookie"))
    }

    @Test
    fun ackRefusalsAreReportedWithTheirFixedToken() {
        val fake = Fake(
            ArrayDeque(
                listOf(
                    answer(409, """{"result":"stale_view"}"""),
                    answer(429, """{"result":"rate_limited"}"""),
                    answer(403, """{"result":"alerts_disabled"}"""),
                    answer(200, """{"result":"acknowledged"}"""),
                    answer(403, """{"result":"login_required"}"""),
                ),
            ),
        )
        val api = SoosApi(origin, fake)
        assertEquals(ApiResult.Refused(409, "stale_view"), api.ackAlerts(epoch, 9))
        assertEquals(ApiResult.Refused(429, "rate_limited"), api.ackAlerts(epoch, 9))
        assertEquals(ApiResult.Refused(403, "alerts_disabled"), api.ackAlerts(epoch, 9))
        assertEquals("a 200 without a view is unreadable", ApiResult.Unreachable, api.ackAlerts(epoch, 9))
        assertEquals(ApiResult.Refused(403, "login_required"), api.alerts())
        assertEquals(ApiResult.Unreachable, api.ackAlerts(epoch, 9))
    }

    @Test
    fun ackWithAnInvalidSnapshotSendsNothing() {
        val fake = Fake(ArrayDeque(listOf(answer(200, clearedJson))))
        val api = SoosApi(origin, fake)
        for (bad in listOf("", "0123456789ABCDEF", "0123456789abcde", "0123456789abcdef0", "0123456789abcdeg", "0123456789abcde\n")) {
            assertEquals(ApiResult.Refused(400, "bad_request"), api.ackAlerts(bad, 9))
        }
        assertEquals(ApiResult.Refused(400, "bad_request"), api.ackAlerts(epoch, -1))
        assertTrue("nothing reached the network", fake.requests.isEmpty())
    }

    @Test
    fun alertsWithAMalformedBodyAreUnreachable() {
        val fake = Fake(
            ArrayDeque(
                listOf(
                    answer(200, """{"state":"exploded"}"""),
                    answer(200, "<html>"),
                ),
            ),
        )
        val api = SoosApi(origin, fake)
        assertEquals(ApiResult.Unreachable, api.alerts())
        assertEquals(ApiResult.Unreachable, api.alerts())
        assertEquals("network error", ApiResult.Unreachable, api.alerts())
    }

    // --- battery and offline history (GitHub #439, matrix ANA24) ----------------------------

    private val batteryJson = """{"state":"present","percent":64,"charge":"charging","external_power":true}"""
    private val offlineJson =
        """{"state":"active","reason":null,"network_monitor":"active","intervals":[""" +
            """{"reason":"wifi_disabled","start_unix_ms":1700000000000,"end_unix_ms":1700000900000,"duration_ms":900000}]}"""

    @Test
    fun batteryAndOfflineArePlainGets() {
        val fake = Fake(ArrayDeque(listOf(answer(200, batteryJson), answer(200, offlineJson))))
        val api = SoosApi(origin, fake)
        assertEquals(ApiResult.Ok(BatteryView("present", 64, "charging", true)), api.battery())
        assertEquals(
            ApiResult.Ok(
                OfflineView(
                    "active",
                    null,
                    "active",
                    listOf(OfflineInterval("wifi_disabled", 1700000000000, 1700000900000, 900000)),
                ),
            ),
            api.offline(),
        )
        assertEquals(
            listOf("$origin/api/battery", "$origin/api/offline"),
            fake.requests.map { it.url },
        )
        assertEquals("/api/battery", SoosApi.BATTERY_PATH)
        assertEquals("/api/offline", SoosApi.OFFLINE_PATH)
        for (request in fake.requests) {
            assertEquals("GET", request.method)
            assertEquals(listOf("Accept"), request.headers.map { it.first })
            assertEquals("application/json", request.header("Accept"))
            assertNull(request.header("Origin"))
            assertNull(request.header("X-Soos-Action"))
            assertNull(request.header("Cookie"))
            assertNull(request.body)
        }
    }

    @Test
    fun batteryAndOfflineSendTheSessionCookieAndDropItOnLoginRequired() {
        val fake = Fake(
            ArrayDeque(
                listOf(
                    answer(200, """{"result":"logged_in"}""", listOf("__Host-soos_session=$token; Path=/; Secure")),
                    answer(200, batteryJson),
                    answer(200, offlineJson),
                    answer(403, """{"result":"login_required"}"""),
                    answer(403, """{"result":"login_required"}"""),
                ),
            ),
        )
        val api = SoosApi(origin, fake)
        assertEquals(ApiResult.Ok("logged_in"), api.loginVerify("""{"id":"x"}"""))
        assertTrue(api.battery() is ApiResult.Ok)
        assertTrue(api.offline() is ApiResult.Ok)
        assertEquals("__Host-soos_session=$token", fake.requests[1].header("Cookie"))
        assertEquals("__Host-soos_session=$token", fake.requests[2].header("Cookie"))
        assertEquals(1, fake.requests[2].headerCount("Cookie"))
        assertEquals(ApiResult.Refused(403, "login_required"), api.battery())
        assertFalse("login_required drops the session", api.hasSession)
        assertEquals(ApiResult.Refused(403, "login_required"), api.offline())
        assertNull("no session is sent once dropped", fake.requests[4].header("Cookie"))
    }

    @Test
    fun batteryAndOfflineRefusalsAndUnreadableBodies() {
        val fake = Fake(
            ArrayDeque(
                listOf(
                    answer(503, """{"result":"unavailable"}"""),
                    answer(200, """{"state":"exploded","percent":null,"charge":null,"external_power":null}"""),
                    answer(200, "<html>"),
                    answer(200, """{"state":"active","reason":null,"network_monitor":"active","intervals":"none"}"""),
                    answer(500, "<html>"),
                ),
            ),
        )
        val api = SoosApi(origin, fake)
        assertEquals(ApiResult.Refused(503, "unavailable"), api.battery())
        assertEquals(ApiResult.Unreachable, api.battery())
        assertEquals(ApiResult.Unreachable, api.offline())
        assertEquals(ApiResult.Unreachable, api.offline())
        assertEquals(ApiResult.Refused(500, null), api.offline())
        assertEquals("network error", ApiResult.Unreachable, api.battery())
        assertEquals("network error", ApiResult.Unreachable, api.offline())
    }
}
