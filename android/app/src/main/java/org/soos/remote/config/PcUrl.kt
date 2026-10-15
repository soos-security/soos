package org.soos.remote.config

/** Why a typed PC URL was refused (fixed reasons, the input is never echoed). */
enum class PcUrlError {
    EMPTY,
    TOO_LONG,
    NOT_HTTPS,
    USERINFO,
    PORT,
    PATH,
    QUERY_OR_FRAGMENT,
    INVALID_HOST,
    NOT_TS_NET,
}

/** Result of [PcUrl.parse]. */
sealed interface PcUrlResult {
    /** A usable PC address: [origin] is exactly `https://` + [host]. */
    data class Valid(val origin: String, val host: String) : PcUrlResult

    /** A refused input. */
    data class Invalid(val reason: PcUrlError) : PcUrlResult
}

/**
 * The PC address of `soos-remote`: `https://<pc>.<tailnet>.ts.net`, the same rule as the
 * server's `rp_id` (a valid DNS name ending with `.ts.net`, at least two labels before it),
 * HTTPS only, no user information, no port, no path, no query, no fragment. Pure.
 */
object PcUrl {
    /** Longest accepted input, after trimming. */
    const val MAX_INPUT_LENGTH = 300

    /** Suffix of every Tailscale node name. */
    const val TS_NET_SUFFIX = ".ts.net"

    private const val SCHEME = "https://"
    private const val MAX_HOST_LENGTH = 253
    private const val MAX_LABEL_LENGTH = 63

    fun parse(input: String): PcUrlResult {
        val text = input.trim()
        if (text.isEmpty()) return PcUrlResult.Invalid(PcUrlError.EMPTY)
        if (text.length > MAX_INPUT_LENGTH) return PcUrlResult.Invalid(PcUrlError.TOO_LONG)
        if (!text.regionMatches(0, SCHEME, 0, SCHEME.length, ignoreCase = true)) {
            return PcUrlResult.Invalid(PcUrlError.NOT_HTTPS)
        }
        var rest = text.substring(SCHEME.length)
        if (rest.endsWith("/")) rest = rest.substring(0, rest.length - 1)
        if (rest.contains('?') || rest.contains('#')) {
            return PcUrlResult.Invalid(PcUrlError.QUERY_OR_FRAGMENT)
        }
        if (rest.contains('/') || rest.contains('\\')) return PcUrlResult.Invalid(PcUrlError.PATH)
        if (rest.contains('@')) return PcUrlResult.Invalid(PcUrlError.USERINFO)
        if (rest.contains(':')) return PcUrlResult.Invalid(PcUrlError.PORT)
        val host = rest.lowercase()
        if (!isValidHostName(host)) return PcUrlResult.Invalid(PcUrlError.INVALID_HOST)
        if (!host.endsWith(TS_NET_SUFFIX)) return PcUrlResult.Invalid(PcUrlError.NOT_TS_NET)
        val labels = host.removeSuffix(TS_NET_SUFFIX).split('.').count { it.isNotEmpty() }
        if (labels < 2) return PcUrlResult.Invalid(PcUrlError.NOT_TS_NET)
        return PcUrlResult.Valid(origin = SCHEME + host, host = host)
    }

    /** Lower-case DNS name: 1..253 bytes, labels of 1..63 `[a-z0-9-]`, no edge hyphen. */
    private fun isValidHostName(host: String): Boolean {
        if (host.isEmpty() || host.length > MAX_HOST_LENGTH) return false
        return host.split('.').all { label ->
            label.isNotEmpty() &&
                label.length <= MAX_LABEL_LENGTH &&
                label.all { it in 'a'..'z' || it in '0'..'9' || it == '-' } &&
                !label.startsWith('-') &&
                !label.endsWith('-')
        }
    }
}
