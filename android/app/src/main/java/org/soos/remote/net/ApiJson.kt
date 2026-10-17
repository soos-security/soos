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

    /** A wrong type or value inside [parseAlerts] (never escapes it). */
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
