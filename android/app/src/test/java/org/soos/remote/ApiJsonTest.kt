package org.soos.remote

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Test
import org.soos.remote.net.AlertRecord
import org.soos.remote.net.AlertsView
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

    // --- failed-password alerts (GitHub #404, matrix ANA18) ---------------------------------

    private val epoch = "0123456789abcdef"

    private fun record(
        id: Long = 5,
        source: String = "\"lock_screen\"",
        account: String = "\"owner\"",
        kind: String = "\"wrong_password\"",
        count: String = "3",
    ) = """{"id":$id,"first_unix_ms":1699999990000,"last_unix_ms":1700000000000,""" +
        """"source":$source,"account":$account,"kind":$kind,"count":$count}"""

    private fun view(
        state: String = "\"active\"",
        reason: String = "null",
        epochValue: String = "\"$epoch\"",
        lockScreen: String = "\"monitored\"",
        wrong: String = "3",
        lockedOut: String = "1",
        last: String = "1700000000000",
        lastSource: String = "\"lock_screen\"",
        through: String = "7",
        history: String = "[${record()}]",
    ) = """{"state":$state,"reason":$reason,"epoch":$epochValue,"lock_screen":$lockScreen,""" +
        """"unacknowledged_wrong_password":$wrong,"unacknowledged_locked_out":$lockedOut,""" +
        """"last_unix_ms":$last,"last_source":$lastSource,"through":$through,"history":$history}"""

    @Test
    fun alertsActiveViewIsParsedExactly() {
        val sudo = record(id = 2, source = "\"sudo\"", account = "\"root\"", kind = "\"locked_out\"", count = "1")
        assertEquals(
            AlertsView(
                state = "active",
                reason = null,
                epoch = epoch,
                lockScreen = "monitored",
                unacknowledgedWrongPassword = 3,
                unacknowledgedLockedOut = 1,
                lastUnixMs = 1700000000000,
                lastSource = "lock_screen",
                through = 7,
                history = listOf(
                    AlertRecord(5, 1699999990000, 1700000000000, "lock_screen", "owner", "wrong_password", 3),
                    AlertRecord(2, 1699999990000, 1700000000000, "sudo", "root", "locked_out", 1),
                ),
            ),
            ApiJson.parseAlerts(view(history = "[${record()},$sudo]")),
        )
    }

    @Test
    fun alertsEveryStateAndEmptyView() {
        assertEquals(
            AlertsView("disabled", null, null, null, 0, 0, null, null, 0, emptyList()),
            ApiJson.parseAlerts(
                view(
                    state = "\"disabled\"",
                    epochValue = "null",
                    lockScreen = "null",
                    wrong = "0",
                    lockedOut = "0",
                    last = "null",
                    lastSource = "null",
                    through = "0",
                    history = "[]",
                ),
            ),
        )
        assertEquals("starting", ApiJson.parseAlerts(view(state = "\"starting\""))?.state)
        val unavailable = ApiJson.parseAlerts(
            view(state = "\"unavailable\"", reason = "\"no_journal_access\"", lockScreen = "\"not_configured\""),
        )
        assertEquals("unavailable", unavailable?.state)
        assertEquals("no_journal_access", unavailable?.reason)
        assertEquals("not_configured", unavailable?.lockScreen)
        for (reason in listOf("journal_reader_failed", "owner_unresolved", "overflow", "rng_failed")) {
            assertEquals(reason, ApiJson.parseAlerts(view(state = "\"unavailable\"", reason = "\"$reason\""))?.reason)
        }
        for (source in listOf("lock_screen", "sudo", "login", "other")) {
            assertNotNull(source, ApiJson.parseAlerts(view(lastSource = "\"$source\"", history = "[${record(source = "\"$source\"")}]")))
        }
        for (account in listOf("owner", "root", "other")) {
            assertNotNull(account, ApiJson.parseAlerts(view(history = "[${record(account = "\"$account\"")}]")))
        }
        // Absent nullable members read as null; unknown extra members are ignored.
        val minimal = """{"state":"active","unacknowledged_wrong_password":0,"unacknowledged_locked_out":0,""" +
            """"through":0,"history":[],"future":true}"""
        assertEquals(AlertsView("active", null, null, null, 0, 0, null, null, 0, emptyList()), ApiJson.parseAlerts(minimal))
    }

    @Test
    fun alertsUnknownEnumValuesAreRefused() {
        assertNull(ApiJson.parseAlerts(view(state = "\"open\"")))
        assertNull(ApiJson.parseAlerts(view(state = "\"Active\"")))
        assertNull(ApiJson.parseAlerts(view(state = "null")))
        assertNull(ApiJson.parseAlerts(view(state = "1")))
        assertNull(ApiJson.parseAlerts(view(state = "\"unavailable\"", reason = "\"disk_full\"")))
        assertNull(ApiJson.parseAlerts(view(lockScreen = "\"unknown\"")))
        assertNull(ApiJson.parseAlerts(view(lastSource = "\"ssh\"")))
        assertNull(ApiJson.parseAlerts(view(history = "[${record(source = "\"<script>\"")}]")))
        assertNull(ApiJson.parseAlerts(view(history = "[${record(account = "\"alice\"")}]")))
        assertNull(ApiJson.parseAlerts(view(history = "[${record(kind = "\"typed_password\"")}]")))
    }

    @Test
    fun alertsMalformedValuesAreRefused() {
        assertNull(ApiJson.parseAlerts("not json"))
        assertNull(ApiJson.parseAlerts("[]"))
        assertNull(ApiJson.parseAlerts(""))
        // epoch: exactly 16 lowercase hex characters
        assertNull(ApiJson.parseAlerts(view(epochValue = "\"0123456789ABCDEF\"")))
        assertNull(ApiJson.parseAlerts(view(epochValue = "\"0123456789abcde\"")))
        assertNull(ApiJson.parseAlerts(view(epochValue = "\"0123456789abcdef0\"")))
        assertNull(ApiJson.parseAlerts(view(epochValue = "\"0123456789abcdeg\"")))
        assertNull(ApiJson.parseAlerts(view(epochValue = "5")))
        // counters, times and through: non-negative integers in range
        assertNull(ApiJson.parseAlerts(view(wrong = "\"3\"")))
        assertNull(ApiJson.parseAlerts(view(wrong = "-1")))
        assertNull(ApiJson.parseAlerts(view(wrong = "1.5")))
        assertNull(ApiJson.parseAlerts(view(wrong = "4294967296")))
        assertEquals(4294967295, ApiJson.parseAlerts(view(wrong = "4294967295"))?.unacknowledgedWrongPassword)
        assertNull(ApiJson.parseAlerts(view(lockedOut = "true")))
        assertNull(ApiJson.parseAlerts(view(last = "-5")))
        assertNull(ApiJson.parseAlerts(view(last = "\"14:07\"")))
        assertNull(ApiJson.parseAlerts(view(through = "-1")))
        assertNull(ApiJson.parseAlerts(view(through = "null")))
        assertNull(ApiJson.parseAlerts(view(through = "18446744073709551615")))
        assertNull(ApiJson.parseAlerts(view(reason = "7")))
        // history: an array of valid record objects
        assertNull(ApiJson.parseAlerts(view(history = "null")))
        assertNull(ApiJson.parseAlerts(view(history = "{}")))
        assertNull(ApiJson.parseAlerts(view(history = "[1]")))
        assertNull(ApiJson.parseAlerts(view(history = "[${record(count = "0")}]")))
        assertNull(ApiJson.parseAlerts(view(history = "[${record(count = "\"3\"")}]")))
        assertNull(ApiJson.parseAlerts(view(history = "[${record(id = -1)}]")))
        assertNull(ApiJson.parseAlerts(view(history = "[{\"id\":1}]")))
    }

    @Test
    fun alertsOversizedInputIsRefused() {
        val full = (1..ApiJson.MAX_ALERT_HISTORY).joinToString(",", "[", "]") { record(id = it.toLong()) }
        assertEquals(32, ApiJson.MAX_ALERT_HISTORY)
        assertEquals(32, ApiJson.parseAlerts(view(history = full))?.history?.size)
        val tooMany = (1..ApiJson.MAX_ALERT_HISTORY + 1).joinToString(",", "[", "]") { record(id = it.toLong()) }
        assertNull(ApiJson.parseAlerts(view(history = tooMany)))
        // The parser keeps the 16 KiB bound of the transport (an unknown member pads the body).
        val prefix = view().dropLast(1) + ",\"pad\":\""
        val suffix = "\"}"
        val padding = 16384 - prefix.length - suffix.length
        val atLimit = prefix + "a".repeat(padding) + suffix
        assertEquals(16384, atLimit.length)
        assertNotNull(ApiJson.parseAlerts(atLimit))
        assertNull(ApiJson.parseAlerts(prefix + "a".repeat(padding + 1) + suffix))
    }
}
