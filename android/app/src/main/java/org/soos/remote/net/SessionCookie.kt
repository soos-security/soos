package org.soos.remote.net

/** What a response did to the web session cookie. */
sealed interface CookieUpdate {
    /** No session cookie in the response. */
    data object None : CookieUpdate

    /** The server cleared the cookie (sign-out). */
    data object Cleared : CookieUpdate

    /** A new session value. */
    data class Set(val value: String) : CookieUpdate
}

/**
 * The `__Host-soos_session` cookie of a Funnel web session. The value lives in memory only
 * (never written to storage) and is sent back verbatim. Pure.
 */
object SessionCookie {
    const val NAME = "__Host-soos_session"

    /** Base64url of the 32-byte session token, without padding. */
    const val VALUE_LENGTH = 43

    /** The update carried by the `Set-Cookie` header values of one response. */
    fun fromSetCookie(headers: List<String>): CookieUpdate {
        var update: CookieUpdate = CookieUpdate.None
        for (header in headers) {
            val pair = header.substringBefore(';').trim()
            val eq = pair.indexOf('=')
            if (eq <= 0 || pair.substring(0, eq) != NAME) continue
            val value = pair.substring(eq + 1)
            update = when {
                value.isEmpty() -> CookieUpdate.Cleared
                isTokenValue(value) -> CookieUpdate.Set(value)
                else -> CookieUpdate.None
            }
        }
        return update
    }

    /** The `Cookie` request header value for [value]. */
    fun header(value: String): String = "$NAME=$value"

    private fun isTokenValue(value: String): Boolean =
        value.length == VALUE_LENGTH && value.all(::isBase64Url)

    internal fun isBase64Url(c: Char): Boolean =
        c in 'A'..'Z' || c in 'a'..'z' || c in '0'..'9' || c == '-' || c == '_'
}
