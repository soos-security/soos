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
