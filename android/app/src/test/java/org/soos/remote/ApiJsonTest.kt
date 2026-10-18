package org.soos.remote

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Test
import org.soos.remote.net.AlertRecord
import org.soos.remote.net.AlertsView
import org.soos.remote.net.ApiJson
import org.soos.remote.net.AuthStateView
import org.soos.remote.net.BatteryView
import org.soos.remote.net.LoginOptions
import org.soos.remote.net.OfflineInterval
import org.soos.remote.net.OfflineView
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

    // --- battery level (GitHub #439, matrix ANA22) ------------------------------------------

    private fun battery(
        state: String = "\"present\"",
        percent: String = "81",
        charge: String = "\"discharging\"",
        externalPower: String = "false",
    ) = """{"state":$state,"percent":$percent,"charge":$charge,"external_power":$externalPower}"""

    @Test
    fun batteryPresentViewIsParsedExactly() {
        assertEquals(BatteryView("present", 81, "discharging", false), ApiJson.parseBattery(battery()))
        assertEquals(
            BatteryView("present", null, "unknown", null),
            ApiJson.parseBattery(battery(percent = "null", charge = "\"unknown\"", externalPower = "null")),
        )
        assertEquals(0, ApiJson.parseBattery(battery(percent = "0"))?.percent)
        assertEquals(100, ApiJson.parseBattery(battery(percent = "100"))?.percent)
        for (charge in listOf("charging", "discharging", "full", "not_charging", "unknown")) {
            assertEquals(charge, ApiJson.parseBattery(battery(charge = "\"$charge\""))?.charge)
        }
        assertEquals(true, ApiJson.parseBattery(battery(externalPower = "true"))?.externalPower)
        // Unknown extra members are ignored; an absent nullable member reads as null.
        assertEquals(
            BatteryView("present", null, "full", null),
            ApiJson.parseBattery("""{"state":"present","charge":"full","future":1}"""),
        )
    }

    @Test
    fun batteryEveryStateWithoutValues() {
        for (state in listOf("disabled", "unavailable", "no_battery")) {
            assertEquals(
                BatteryView(state, null, null, null),
                ApiJson.parseBattery(battery(state = "\"$state\"", percent = "null", charge = "null", externalPower = "null")),
            )
            assertEquals(BatteryView(state, null, null, null), ApiJson.parseBattery("""{"state":"$state"}"""))
            // Values are only ever sent with `present` (battery.rs `without_values`).
            assertNull(ApiJson.parseBattery(battery(state = "\"$state\"", percent = "50", charge = "null", externalPower = "null")))
            assertNull(ApiJson.parseBattery(battery(state = "\"$state\"", percent = "null", charge = "\"full\"", externalPower = "null")))
            assertNull(ApiJson.parseBattery(battery(state = "\"$state\"", percent = "null", charge = "null", externalPower = "true")))
        }
        // `charge` is set whenever a battery is present.
        assertNull(ApiJson.parseBattery(battery(charge = "null")))
        assertNull(ApiJson.parseBattery("""{"state":"present","percent":5}"""))
    }

    @Test
    fun batteryUnknownOrMalformedValuesAreRefused() {
        assertNull(ApiJson.parseBattery("not json"))
        assertNull(ApiJson.parseBattery("[]"))
        assertNull(ApiJson.parseBattery(""))
        assertNull(ApiJson.parseBattery("""{"percent":5}"""))
        assertNull(ApiJson.parseBattery(battery(state = "\"Present\"")))
        assertNull(ApiJson.parseBattery(battery(state = "\"charging\"")))
        assertNull(ApiJson.parseBattery(battery(state = "null")))
        assertNull(ApiJson.parseBattery(battery(state = "1")))
        assertNull(ApiJson.parseBattery(battery(charge = "\"Charging\"")))
        assertNull(ApiJson.parseBattery(battery(charge = "\"other\"")))
        assertNull(ApiJson.parseBattery(battery(charge = "1")))
        assertNull(ApiJson.parseBattery(battery(percent = "-1")))
        assertNull(ApiJson.parseBattery(battery(percent = "101")))
        assertNull(ApiJson.parseBattery(battery(percent = "50.5")))
        assertNull(ApiJson.parseBattery(battery(percent = "\"50\"")))
        assertNull(ApiJson.parseBattery(battery(percent = "4294967296")))
        assertNull(ApiJson.parseBattery(battery(externalPower = "\"true\"")))
        assertNull(ApiJson.parseBattery(battery(externalPower = "1")))
    }

    @Test
    fun batteryOversizedInputIsRefused() {
        assertEquals(16384, ApiJson.MAX_VIEW_BODY_CHARS)
        val prefix = battery().dropLast(1) + ",\"pad\":\""
        val suffix = "\"}"
        val padding = ApiJson.MAX_VIEW_BODY_CHARS - prefix.length - suffix.length
        val atLimit = prefix + "a".repeat(padding) + suffix
        assertEquals(16384, atLimit.length)
        assertNotNull(ApiJson.parseBattery(atLimit))
        assertNull(ApiJson.parseBattery(prefix + "a".repeat(padding + 1) + suffix))
    }

    // --- offline history (GitHub #439, matrix ANA23) ----------------------------------------

    private fun interval(
        reason: String = "\"network_lost\"",
        start: String = "1700000000000",
        end: String = "1700000600000",
        duration: String = "600000",
    ) = """{"reason":$reason,"start_unix_ms":$start,"end_unix_ms":$end,"duration_ms":$duration}"""

    private fun offline(
        state: String = "\"active\"",
        reason: String = "null",
        monitor: String = "\"active\"",
        intervals: String = "[${interval()}]",
    ) = """{"state":$state,"reason":$reason,"network_monitor":$monitor,"intervals":$intervals}"""

    @Test
    fun offlineActiveViewIsParsedExactly() {
        val sleep = interval(reason = "\"sleep\"", start = "1690000000000", end = "1690003600000", duration = "3500000")
        assertEquals(
            OfflineView(
                state = "active",
                reason = null,
                networkMonitor = "active",
                intervals = listOf(
                    OfflineInterval("network_lost", 1700000000000, 1700000600000, 600000),
                    OfflineInterval("sleep", 1690000000000, 1690003600000, 3500000),
                ),
            ),
            ApiJson.parseOffline(offline(intervals = "[${interval()},$sleep]")),
        )
        for (reason in listOf("network_lost", "wifi_disabled", "sleep", "shutdown_or_crash")) {
            assertEquals(
                reason,
                ApiJson.parseOffline(offline(intervals = "[${interval(reason = "\"$reason\"")}]"))?.intervals?.single()?.reason,
            )
        }
        for (monitor in listOf("unknown", "active", "unavailable")) {
            assertEquals(monitor, ApiJson.parseOffline(offline(monitor = "\"$monitor\""))?.networkMonitor)
        }
        assertEquals(
            OfflineView("active", null, "unknown", emptyList()),
            ApiJson.parseOffline(offline(monitor = "\"unknown\"", intervals = "[]")),
        )
        // An empty wall span is valid (start == end); extra members are ignored.
        assertEquals(
            OfflineInterval("sleep", 5, 5, 0),
            ApiJson.parseOffline(
                offline(intervals = """[{"reason":"sleep","start_unix_ms":5,"end_unix_ms":5,"duration_ms":0,"x":1}]"""),
            )?.intervals?.single(),
        )
        assertEquals(
            OfflineView("active", null, "active", emptyList()),
            ApiJson.parseOffline("""{"state":"active","network_monitor":"active","intervals":[],"future":true}"""),
        )
    }

    @Test
    fun offlineDisabledAndUnavailableViews() {
        assertEquals(
            OfflineView("disabled", null, "unknown", emptyList()),
            ApiJson.parseOffline(offline(state = "\"disabled\"", monitor = "\"unknown\"", intervals = "[]")),
        )
        assertEquals(
            OfflineView("unavailable", "store_failed", "unknown", emptyList()),
            ApiJson.parseOffline(
                offline(state = "\"unavailable\"", reason = "\"store_failed\"", monitor = "\"unknown\"", intervals = "[]"),
            ),
        )
        // `reason` is set iff unavailable; the monitor is unknown and the list empty unless active.
        assertNull(ApiJson.parseOffline(offline(state = "\"unavailable\"", monitor = "\"unknown\"", intervals = "[]")))
        assertNull(ApiJson.parseOffline(offline(reason = "\"store_failed\"")))
        assertNull(
            ApiJson.parseOffline(offline(state = "\"disabled\"", reason = "\"store_failed\"", monitor = "\"unknown\"", intervals = "[]")),
        )
        assertNull(ApiJson.parseOffline(offline(state = "\"disabled\"", monitor = "\"active\"", intervals = "[]")))
        assertNull(ApiJson.parseOffline(offline(state = "\"disabled\"", monitor = "\"unknown\"")))
        assertNull(
            ApiJson.parseOffline(offline(state = "\"unavailable\"", reason = "\"store_failed\"", monitor = "\"unavailable\"", intervals = "[]")),
        )
        assertNull(
            ApiJson.parseOffline(offline(state = "\"unavailable\"", reason = "\"store_failed\"", monitor = "\"unknown\"")),
        )
    }

    @Test
    fun offlineUnknownOrMalformedValuesAreRefused() {
        assertNull(ApiJson.parseOffline("not json"))
        assertNull(ApiJson.parseOffline("[]"))
        assertNull(ApiJson.parseOffline(""))
        assertNull(ApiJson.parseOffline(offline(state = "\"Active\"")))
        assertNull(ApiJson.parseOffline(offline(state = "\"starting\"")))
        assertNull(ApiJson.parseOffline(offline(state = "null")))
        assertNull(ApiJson.parseOffline(offline(state = "\"unavailable\"", reason = "\"disk_full\"", monitor = "\"unknown\"", intervals = "[]")))
        assertNull(ApiJson.parseOffline(offline(state = "\"unavailable\"", reason = "7", monitor = "\"unknown\"", intervals = "[]")))
        assertNull(ApiJson.parseOffline(offline(monitor = "\"other\"")))
        assertNull(ApiJson.parseOffline(offline(monitor = "null")))
        assertNull(ApiJson.parseOffline("""{"state":"active","intervals":[]}"""))
        assertNull(ApiJson.parseOffline("""{"state":"active","network_monitor":"active"}"""))
        assertNull(ApiJson.parseOffline(offline(intervals = "null")))
        assertNull(ApiJson.parseOffline(offline(intervals = "{}")))
        assertNull(ApiJson.parseOffline(offline(intervals = "[1]")))
        assertNull(ApiJson.parseOffline(offline(intervals = "[${interval(reason = "\"other\"")}]")))
        assertNull(ApiJson.parseOffline(offline(intervals = "[${interval(reason = "null")}]")))
        assertNull(ApiJson.parseOffline(offline(intervals = "[${interval(start = "-1")}]")))
        assertNull(ApiJson.parseOffline(offline(intervals = "[${interval(end = "-1")}]")))
        assertNull(ApiJson.parseOffline(offline(intervals = "[${interval(duration = "-1")}]")))
        assertNull(ApiJson.parseOffline(offline(intervals = "[${interval(duration = "1.5")}]")))
        assertNull(ApiJson.parseOffline(offline(intervals = "[${interval(start = "\"1\"")}]")))
        assertNull(ApiJson.parseOffline(offline(intervals = "[${interval(duration = "null")}]")))
        assertNull(ApiJson.parseOffline(offline(intervals = "[${interval(start = "18446744073709551615")}]")))
        // The wall span is never inverted (offline.rs `OfflineInterval::new`, store loader).
        assertNull(ApiJson.parseOffline(offline(intervals = "[${interval(start = "10", end = "9")}]")))
        assertNull(ApiJson.parseOffline(offline(intervals = """[{"reason":"sleep"}]""")))
    }

    @Test
    fun offlineOversizedInputIsRefused() {
        assertEquals(32, ApiJson.MAX_OFFLINE_INTERVALS)
        val full = (1..ApiJson.MAX_OFFLINE_INTERVALS).joinToString(",", "[", "]") { interval() }
        assertEquals(32, ApiJson.parseOffline(offline(intervals = full))?.intervals?.size)
        val tooMany = (1..ApiJson.MAX_OFFLINE_INTERVALS + 1).joinToString(",", "[", "]") { interval() }
        assertNull(ApiJson.parseOffline(offline(intervals = tooMany)))
        val prefix = offline().dropLast(1) + ",\"pad\":\""
        val suffix = "\"}"
        val padding = ApiJson.MAX_VIEW_BODY_CHARS - prefix.length - suffix.length
        val atLimit = prefix + "a".repeat(padding) + suffix
        assertEquals(16384, atLimit.length)
        assertNotNull(ApiJson.parseOffline(atLimit))
        assertNull(ApiJson.parseOffline(prefix + "a".repeat(padding + 1) + suffix))
    }
}
