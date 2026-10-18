package org.soos.remote.net

import org.json.JSONArray
import org.json.JSONException
import org.json.JSONObject

/** `GET /api/auth/state`. */
data class AuthStateView(val mode: String, val authenticated: Boolean)

/** `GET /api/status` (`state` is one of [ApiJson.STATES]). */
data class StatusView(
    val state: String,
    val active: Boolean,
    val idle: Boolean,
    val checkedUnixMs: Long,
)

/** `POST /api/auth/login/options`. */
data class LoginOptions(val challenge: String, val rpId: String, val timeoutMs: Long)

/** One history record of `GET /api/alerts` (no text: classes, times and a count only). */
data class AlertRecord(
    val id: Long,
    val firstUnixMs: Long,
    val lastUnixMs: Long,
    /** In [ApiJson.ALERT_SOURCES]. */
    val source: String,
    /** In [ApiJson.ALERT_ACCOUNTS]. */
    val account: String,
    /** In [ApiJson.ALERT_KINDS]. */
    val kind: String,
    val count: Long,
)

/** `GET /api/alerts` and the `200` body of `POST /api/alerts/ack`. */
data class AlertsView(
    /** In [ApiJson.ALERT_STATES]. */
    val state: String,
    /** `null` or in [ApiJson.ALERT_REASONS]. */
    val reason: String?,
    /** `null` or [ApiJson.ALERTS_EPOCH_HEX_LEN] lowercase hex characters. */
    val epoch: String?,
    /** `null` or in [ApiJson.LOCK_SCREEN_COVERAGE]. */
    val lockScreen: String?,
    val unacknowledgedWrongPassword: Long,
    val unacknowledgedLockedOut: Long,
    val lastUnixMs: Long?,
    /** `null` or in [ApiJson.ALERT_SOURCES]. */
    val lastSource: String?,
    /** Highest attempt seq of this view (the `X-Soos-Alerts-Through` of an acknowledgement). */
    val through: Long,
    /** Newest first, at most [ApiJson.MAX_ALERT_HISTORY]. */
    val history: List<AlertRecord>,
)

/** `GET /api/battery` (`crates/remote/src/battery.rs` `BatteryView`). */
data class BatteryView(
    /** In [ApiJson.BATTERY_STATES]. */
    val state: String,
    /** `null` or `0..`[ApiJson.MAX_BATTERY_PERCENT], only with `present`. */
    val percent: Int?,
    /** In [ApiJson.CHARGE_STATUSES] with `present`, else `null`. */
    val charge: String?,
    /** `null`, or whether mains power is connected, only with `present`. */
    val externalPower: Boolean?,
)

/** One offline interval of `GET /api/offline` (reason and times only). */
data class OfflineInterval(
    /** In [ApiJson.OFFLINE_REASONS]. */
    val reason: String,
    val startUnixMs: Long,
    /** Never before [startUnixMs]. */
    val endUnixMs: Long,
    val durationMs: Long,
)

/** `GET /api/offline` (`crates/remote/src/offline.rs` `OfflineView`). */
data class OfflineView(
    /** In [ApiJson.OFFLINE_STATES]. */
    val state: String,
    /** In [ApiJson.OFFLINE_UNAVAILABLE_REASONS] iff `unavailable`, else `null`. */
    val reason: String?,
    /** In [ApiJson.NETWORK_MONITORS]; `unknown` unless `active`. */
    val networkMonitor: String,
    /** Newest first, at most [ApiJson.MAX_OFFLINE_INTERVALS], empty unless `active`. */
    val intervals: List<OfflineInterval>,
)

/**
 * Pure parsers of the `soos-remote` JSON answers. Every value is type-checked explicitly
 * (no `opt*` coercion: `org.json` differs between the JVM and Android there); anything
 * unexpected is `null`.
 */
object ApiJson {
    val STATES = setOf("locked", "unlocked", "no_session", "unavailable")
    private val MODES = setOf("tailnet", "funnel")
    private val RESULT = Regex("[a-z_]{1,64}")
    private const val CHALLENGE_LENGTH = 43
    private const val MAX_TIMEOUT_MS = 600_000L

    val ALERT_STATES = setOf("disabled", "starting", "active", "unavailable")
    val ALERT_REASONS = setOf(
        "no_journal_access",
        "journal_reader_failed",
        "owner_unresolved",
        "overflow",
        "rng_failed",
    )
    val LOCK_SCREEN_COVERAGE = setOf("monitored", "not_configured")
    val ALERT_SOURCES = setOf("lock_screen", "sudo", "login", "other")
    val ALERT_ACCOUNTS = setOf("owner", "root", "other")
    val ALERT_KINDS = setOf("wrong_password", "locked_out")

    /** Equals `MAX_ALERT_HISTORY` of `crates/remote/src/lib.rs`. */
    const val MAX_ALERT_HISTORY = 32

    /** Equals `ALERTS_EPOCH_HEX_LEN` of `crates/remote/src/lib.rs`. */
    const val ALERTS_EPOCH_HEX_LEN = 16

    /** The saturating `u32` counters of the server. */
    const val MAX_ALERT_COUNT = 4_294_967_295L

    /** Equals `UrlConnectionTransport.MAX_RESPONSE_BYTES` (a char is at least one byte). */
    const val MAX_ALERTS_BODY_CHARS = 16384

    /** `BatteryState` of `crates/remote/src/battery.rs`. */
    val BATTERY_STATES = setOf("disabled", "unavailable", "no_battery", "present")

    /** `ChargeStatus` of `crates/remote/src/battery.rs`. */
    val CHARGE_STATUSES = setOf("charging", "discharging", "full", "not_charging", "unknown")

    /** The server percent is at most 100 (`parse_capacity`). */
    const val MAX_BATTERY_PERCENT = 100

    /** `OfflineState` of `crates/remote/src/offline.rs`. */
    val OFFLINE_STATES = setOf("disabled", "active", "unavailable")

    /** `OfflineUnavailableReason` of `crates/remote/src/offline.rs`. */
    val OFFLINE_UNAVAILABLE_REASONS = setOf("store_failed")

    /** `NetworkMonitor` of `crates/remote/src/offline.rs`. */
    val NETWORK_MONITORS = setOf("unknown", "active", "unavailable")

    /** `OfflineReason` of `crates/remote/src/offline.rs`. */
    val OFFLINE_REASONS = setOf("network_lost", "wifi_disabled", "sleep", "shutdown_or_crash")

    /** Equals `MAX_OFFLINE_INTERVALS` of `crates/remote/src/lib.rs`. */
    const val MAX_OFFLINE_INTERVALS = 32

    /** Equals `UrlConnectionTransport.MAX_RESPONSE_BYTES` (a char is at least one byte). */
    const val MAX_VIEW_BODY_CHARS = 16384

    fun parseAuthState(body: String): AuthStateView? {
        val json = objectOf(body) ?: return null
        val mode = string(json, "mode")?.takeIf { it in MODES } ?: return null
        val authenticated = bool(json, "authenticated") ?: return null
        return AuthStateView(mode, authenticated)
    }

    fun parseStatus(body: String): StatusView? {
        val json = objectOf(body) ?: return null
        val state = string(json, "state")?.takeIf { it in STATES } ?: return null
        val active = bool(json, "active") ?: return null
        val idle = bool(json, "idle") ?: return null
        val checked = long(json, "checked_unix_ms") ?: return null
        return StatusView(state, active, idle, checked)
    }

    /** The `result` token of `{"result": "..."}`. */
    fun parseResult(body: String): String? {
        val json = objectOf(body) ?: return null
        return string(json, "result")?.takeIf { RESULT.matches(it) }
    }

    /** Login options; `rp_id` must be the configured host (never a passkey of another site). */
    fun parseLoginOptions(body: String, expectedRpId: String): LoginOptions? {
        val json = objectOf(body) ?: return null
        val challenge = string(json, "challenge")
            ?.takeIf { it.length == CHALLENGE_LENGTH && it.all(SessionCookie::isBase64Url) }
            ?: return null
        val rpId = string(json, "rp_id")?.takeIf { it == expectedRpId } ?: return null
        val timeout = long(json, "timeout_ms")?.takeIf { it in 1..MAX_TIMEOUT_MS } ?: return null
        return LoginOptions(challenge, rpId, timeout)
    }

    /** Exactly [ALERTS_EPOCH_HEX_LEN] characters of `[0-9a-f]`. */
    fun isAlertsEpoch(value: String): Boolean =
        value.length == ALERTS_EPOCH_HEX_LEN && value.all { it in '0'..'9' || it in 'a'..'f' }

    /**
     * Strict, bounded parser of the alerts view: an unknown state, reason, coverage, source,
     * account or kind, a wrong type, a value out of range, more than [MAX_ALERT_HISTORY]
     * entries or a body over [MAX_ALERTS_BODY_CHARS] makes the whole view `null`. Unknown
     * extra members are ignored.
     */
    fun parseAlerts(body: String): AlertsView? {
        if (body.length > MAX_ALERTS_BODY_CHARS) return null
        val json = objectOf(body) ?: return null
        return try {
            val history = array(json, "history") ?: return null
            if (history.length() > MAX_ALERT_HISTORY) return null
            val records = ArrayList<AlertRecord>(history.length())
            for (index in 0 until history.length()) {
                val entry = history.get(index) as? JSONObject ?: return null
                records += parseAlertRecord(entry) ?: return null
            }
            AlertsView(
                state = string(json, "state")?.takeIf { it in ALERT_STATES } ?: return null,
                reason = nullableMember(json, "reason", ALERT_REASONS),
                epoch = nullableString(json, "epoch")?.also { if (!isAlertsEpoch(it)) throw Malformed() },
                lockScreen = nullableMember(json, "lock_screen", LOCK_SCREEN_COVERAGE),
                unacknowledgedWrongPassword = counter(json, "unacknowledged_wrong_password", 0) ?: return null,
                unacknowledgedLockedOut = counter(json, "unacknowledged_locked_out", 0) ?: return null,
                lastUnixMs = nullableNonNegative(json, "last_unix_ms"),
                lastSource = nullableMember(json, "last_source", ALERT_SOURCES),
                through = long(json, "through")?.takeIf { it >= 0 } ?: return null,
                history = records,
            )
        } catch (ignored: Malformed) {
            null
        } catch (ignored: JSONException) {
            null
        }
    }

    /**
     * Strict, bounded parser of the battery view: an unknown state or charge status, a wrong
     * type, a percent outside `0..`[MAX_BATTERY_PERCENT], a value without `present`, `present`
     * without a charge status, or a body over [MAX_VIEW_BODY_CHARS] makes the whole view `null`.
     * Unknown extra members are ignored; an absent nullable member reads as `null`.
     */
    fun parseBattery(body: String): BatteryView? {
        if (body.length > MAX_VIEW_BODY_CHARS) return null
        val json = objectOf(body) ?: return null
        return try {
            val state = string(json, "state")?.takeIf { it in BATTERY_STATES } ?: return null
            val percent = nullableNonNegative(json, "percent")
                ?.also { if (it > MAX_BATTERY_PERCENT) throw Malformed() }
                ?.toInt()
            val charge = nullableMember(json, "charge", CHARGE_STATUSES)
            val externalPower = nullableBool(json, "external_power")
            val present = state == "present"
            if (present && charge == null) return null
            if (!present && (percent != null || charge != null || externalPower != null)) return null
            BatteryView(state, percent, charge, externalPower)
        } catch (ignored: Malformed) {
            null
        } catch (ignored: JSONException) {
            null
        }
    }

    /**
     * Strict, bounded parser of the offline history: an unknown state, reason, monitor or
     * interval reason, a wrong type, a negative time or duration, an interval ending before it
     * starts, more than [MAX_OFFLINE_INTERVALS] intervals, a view that breaks the server's rules
     * (a reason iff `unavailable`; monitor `unknown` and no interval unless `active`) or a body
     * over [MAX_VIEW_BODY_CHARS] makes the whole view `null`. Unknown extra members are ignored.
     */
    fun parseOffline(body: String): OfflineView? {
        if (body.length > MAX_VIEW_BODY_CHARS) return null
        val json = objectOf(body) ?: return null
        return try {
            val state = string(json, "state")?.takeIf { it in OFFLINE_STATES } ?: return null
            val reason = nullableMember(json, "reason", OFFLINE_UNAVAILABLE_REASONS)
            val monitor = string(json, "network_monitor")?.takeIf { it in NETWORK_MONITORS } ?: return null
            val intervals = array(json, "intervals") ?: return null
            if (intervals.length() > MAX_OFFLINE_INTERVALS) return null
            val active = state == "active"
            if ((state == "unavailable") != (reason != null)) return null
            if (!active && (monitor != "unknown" || intervals.length() != 0)) return null
            val parsed = ArrayList<OfflineInterval>(intervals.length())
            for (index in 0 until intervals.length()) {
                val entry = intervals.get(index) as? JSONObject ?: return null
                parsed += parseOfflineInterval(entry) ?: return null
            }
            OfflineView(state, reason, monitor, parsed)
        } catch (ignored: Malformed) {
            null
        } catch (ignored: JSONException) {
            null
        }
    }

    private fun parseOfflineInterval(json: JSONObject): OfflineInterval? {
        val reason = string(json, "reason")?.takeIf { it in OFFLINE_REASONS } ?: return null
        val start = long(json, "start_unix_ms")?.takeIf { it >= 0 } ?: return null
        val end = long(json, "end_unix_ms")?.takeIf { it >= start } ?: return null
        val duration = long(json, "duration_ms")?.takeIf { it >= 0 } ?: return null
        return OfflineInterval(reason, start, end, duration)
    }

    private fun parseAlertRecord(json: JSONObject): AlertRecord? {
        val id = long(json, "id")?.takeIf { it >= 0 } ?: return null
        val first = long(json, "first_unix_ms")?.takeIf { it >= 0 } ?: return null
        val last = long(json, "last_unix_ms")?.takeIf { it >= 0 } ?: return null
        val source = string(json, "source")?.takeIf { it in ALERT_SOURCES } ?: return null
        val account = string(json, "account")?.takeIf { it in ALERT_ACCOUNTS } ?: return null
        val kind = string(json, "kind")?.takeIf { it in ALERT_KINDS } ?: return null
        val count = counter(json, "count", 1) ?: return null
        return AlertRecord(id, first, last, source, account, kind, count)
    }

    /** A wrong type or value inside [parseAlerts], [parseBattery] or [parseOffline] (never escapes). */
    private class Malformed : Exception()

    /** An integer in `min..MAX_ALERT_COUNT`, else `null`. */
    private fun counter(json: JSONObject, key: String, min: Long): Long? =
        long(json, key)?.takeIf { it in min..MAX_ALERT_COUNT }

    /** Absent or JSON `null` → `null`; a string → it; anything else → [Malformed]. */
    private fun nullableString(json: JSONObject, key: String): String? {
        if (!json.has(key) || json.isNull(key)) return null
        return json.get(key) as? String ?: throw Malformed()
    }

    /** Like [nullableString], and the value must be in [allowed]. */
    private fun nullableMember(json: JSONObject, key: String, allowed: Set<String>): String? =
        nullableString(json, key)?.also { if (it !in allowed) throw Malformed() }

    /** Absent or JSON `null` → `null`; a boolean → it; anything else → [Malformed]. */
    private fun nullableBool(json: JSONObject, key: String): Boolean? {
        if (!json.has(key) || json.isNull(key)) return null
        return json.get(key) as? Boolean ?: throw Malformed()
    }

    /** Absent or JSON `null` → `null`; an integer ≥ 0 → it; anything else → [Malformed]. */
    private fun nullableNonNegative(json: JSONObject, key: String): Long? {
        if (!json.has(key) || json.isNull(key)) return null
        return long(json, key)?.takeIf { it >= 0 } ?: throw Malformed()
    }

    internal fun objectOf(body: String): JSONObject? =
        try {
            JSONObject(body)
        } catch (ignored: JSONException) {
            null
        }

    internal fun string(json: JSONObject, key: String): String? =
        if (json.has(key)) json.get(key) as? String else null

    internal fun bool(json: JSONObject, key: String): Boolean? =
        if (json.has(key)) json.get(key) as? Boolean else null

    internal fun long(json: JSONObject, key: String): Long? {
        if (!json.has(key)) return null
        return when (val value = json.get(key)) {
            is Int -> value.toLong()
            is Long -> value
            else -> null
        }
    }

    internal fun array(json: JSONObject, key: String): JSONArray? =
        if (json.has(key)) json.get(key) as? JSONArray else null
}
