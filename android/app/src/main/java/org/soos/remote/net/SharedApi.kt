package org.soos.remote.net

/**
 * One [SoosApi] per PC origin for the whole process, so the screens and the heartbeat job share
 * the in-memory web session cookie (never written to storage; lost when the process ends).
 * Another origin replaces the client and drops its session. Pure.
 */
class ApiHolder(private val transport: () -> HttpTransport) {
    private var origin: String? = null
    private var api: SoosApi? = null

    @Synchronized
    fun forOrigin(origin: String): SoosApi {
        val current = api
        if (current != null && this.origin == origin) return current
        val created = SoosApi(origin, transport())
        this.origin = origin
        api = created
        return created
    }
}

/** The process-wide [ApiHolder] over the HTTPS transport. */
object SharedApi {
    private val holder = ApiHolder { UrlConnectionTransport() }

    fun forOrigin(origin: String): SoosApi = holder.forOrigin(origin)
}
