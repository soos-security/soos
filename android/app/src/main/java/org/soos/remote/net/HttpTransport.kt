package org.soos.remote.net

import java.io.IOException

/** One HTTPS request to the PC. */
class HttpRequest(
    val method: String,
    val url: String,
    val headers: List<Pair<String, String>>,
    val body: ByteArray?,
)

/** One bounded response. */
class HttpResponse(
    val status: Int,
    val body: ByteArray,
    val setCookies: List<String>,
)

/** The network seam of [SoosApi] (a fake in the unit tests). */
fun interface HttpTransport {
    @Throws(IOException::class)
    fun execute(request: HttpRequest): HttpResponse
}
