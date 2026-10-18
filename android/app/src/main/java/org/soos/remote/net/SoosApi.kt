package org.soos.remote.net

import java.io.IOException

/** Outcome of one call. */
sealed interface ApiResult<out T> {
    data class Ok<T>(val value: T) : ApiResult<T>

    /** The PC answered with a refusal (`status`, fixed `result` token when present). */
    data class Refused(val status: Int, val result: String?) : ApiResult<Nothing>

    /** No usable answer (network error, timeout, malformed body). */
    data object Unreachable : ApiResult<Nothing>
}

/**
 * Client of the `soos-remote` HTTP surface for the native app: status, lock, the
 * failed-password alerts, the battery level, the offline history and the Funnel passkey login.
 * Every POST carries `Origin: <origin>` and the route's exact `X-Soos-Action`; the web session
 * cookie is kept in memory only and dropped on sign-out or `login_required`.
 */
class SoosApi(private val origin: String, private val transport: HttpTransport) {
    @Volatile
    private var session: String? = null

    val hasSession: Boolean
        get() = session != null

    fun authState(): ApiResult<AuthStateView> =
        get(AUTH_STATE_PATH) { ApiJson.parseAuthState(it) }

    fun status(): ApiResult<StatusView> = get(STATUS_PATH) { ApiJson.parseStatus(it) }

    fun lock(): ApiResult<String> = post(LOCK_PATH, ACTION_LOCK, null)

    /** `GET /api/alerts` (tailnet identity or web session). */
    fun alerts(): ApiResult<AlertsView> = get(ALERTS_PATH) { ApiJson.parseAlerts(it) }

    /** `GET /api/battery` (tailnet identity or web session; read-only). */
    fun battery(): ApiResult<BatteryView> = get(BATTERY_PATH) { ApiJson.parseBattery(it) }

    /** `GET /api/offline` (tailnet identity or web session; read-only). */
    fun offline(): ApiResult<OfflineView> = get(OFFLINE_PATH) { ApiJson.parseOffline(it) }

    /**
     * `POST /api/alerts/ack` of the displayed view: no body, `X-Soos-Action: alerts-ack` and
     * exactly one epoch and one through header (`Docs/REMOTE_COMPANION.md` §2c). The `200`
     * answer is the fresh view. A snapshot the server would refuse is never sent.
     */
    fun ackAlerts(epoch: String, through: Long): ApiResult<AlertsView> {
        if (!ApiJson.isAlertsEpoch(epoch) || through < 0) {
            return ApiResult.Refused(BAD_REQUEST, "bad_request")
        }
        val snapshot = listOf(
            ALERTS_EPOCH_HEADER to epoch,
            ALERTS_THROUGH_HEADER to through.toString(),
        )
        val response = send("POST", ALERTS_ACK_PATH, ACTION_ALERTS_ACK, null, snapshot)
            ?: return ApiResult.Unreachable
        val body = response.body.decodeToString()
        if (response.status != 200) return ApiResult.Refused(response.status, ApiJson.parseResult(body))
        val view = ApiJson.parseAlerts(body) ?: return ApiResult.Unreachable
        return ApiResult.Ok(view)
    }

    fun loginOptions(expectedRpId: String): ApiResult<LoginOptions> {
        val response = send("POST", LOGIN_OPTIONS_PATH, ACTION_LOGIN_OPTIONS, null)
            ?: return ApiResult.Unreachable
        val body = response.body.decodeToString()
        if (response.status != 200) return ApiResult.Refused(response.status, ApiJson.parseResult(body))
        val options = ApiJson.parseLoginOptions(body, expectedRpId) ?: return ApiResult.Unreachable
        return ApiResult.Ok(options)
    }

    fun loginVerify(assertionJson: String): ApiResult<String> =
        post(LOGIN_VERIFY_PATH, ACTION_LOGIN, assertionJson)

    fun logout(): ApiResult<String> {
        val result = post(LOGOUT_PATH, ACTION_LOGOUT, null)
        session = null
        return result
    }

    fun forgetSession() {
        session = null
    }

    private fun <T> get(path: String, parse: (String) -> T?): ApiResult<T> {
        val response = send("GET", path, null, null) ?: return ApiResult.Unreachable
        val body = response.body.decodeToString()
        if (response.status != 200) return ApiResult.Refused(response.status, ApiJson.parseResult(body))
        val value = parse(body) ?: return ApiResult.Unreachable
        return ApiResult.Ok(value)
    }

    private fun post(path: String, action: String, json: String?): ApiResult<String> {
        val response = send("POST", path, action, json) ?: return ApiResult.Unreachable
        val result = ApiJson.parseResult(response.body.decodeToString())
        return if (response.status in 200..299 && result != null) {
            ApiResult.Ok(result)
        } else {
            ApiResult.Refused(response.status, result)
        }
    }

    private fun send(
        method: String,
        path: String,
        action: String?,
        json: String?,
        extra: List<Pair<String, String>> = emptyList(),
    ): HttpResponse? {
        val headers = mutableListOf("Accept" to "application/json")
        if (method == "POST") {
            headers += "Origin" to origin
        }
        if (action != null) headers += ACTION_HEADER to action
        headers += extra
        if (json != null) headers += "Content-Type" to "application/json"
        val sent = session
        sent?.let { headers += "Cookie" to SessionCookie.header(it) }
        val request = HttpRequest(method, origin + path, headers, json?.encodeToByteArray())
        val response = try {
            transport.execute(request)
        } catch (ignored: IOException) {
            return null
        }
        synchronized(this) {
            when (val update = SessionCookie.fromSetCookie(response.setCookies)) {
                is CookieUpdate.Set -> session = update.value
                CookieUpdate.Cleared -> session = null
                CookieUpdate.None -> {}
            }
            // The screens and the heartbeat share this client: a refusal of a request sent
            // with another (or no) session never drops a session stored meanwhile.
            if (response.status == 403 &&
                session == sent &&
                ApiJson.parseResult(response.body.decodeToString()) == "login_required"
            ) {
                session = null
            }
        }
        return response
    }

    companion object {
        const val ACTION_HEADER = "X-Soos-Action"
        const val ACTION_LOCK = "lock"
        const val ACTION_LOGIN_OPTIONS = "login-options"
        const val ACTION_LOGIN = "login"
        const val ACTION_LOGOUT = "logout"
        const val STATUS_PATH = "/api/status"
        const val LOCK_PATH = "/api/lock"
        const val AUTH_STATE_PATH = "/api/auth/state"
        const val LOGIN_OPTIONS_PATH = "/api/auth/login/options"
        const val LOGIN_VERIFY_PATH = "/api/auth/login/verify"
        const val LOGOUT_PATH = "/api/auth/logout"

        /** Equals `ACTION_ALERTS_ACK` of `crates/remote/src/lib.rs`. */
        const val ACTION_ALERTS_ACK = "alerts-ack"
        const val ALERTS_PATH = "/api/alerts"
        const val ALERTS_ACK_PATH = "/api/alerts/ack"
        const val ALERTS_EPOCH_HEADER = "X-Soos-Alerts-Epoch"
        const val ALERTS_THROUGH_HEADER = "X-Soos-Alerts-Through"

        /** Equal `BATTERY_PATH` and `OFFLINE_PATH` of `crates/remote/src/routes.rs`. */
        const val BATTERY_PATH = "/api/battery"
        const val OFFLINE_PATH = "/api/offline"

        /** Status of a snapshot refused locally (never sent). */
        private const val BAD_REQUEST = 400
    }
}
