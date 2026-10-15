package org.soos.remote.net

import java.io.ByteArrayOutputStream
import java.io.IOException
import java.io.InputStream
import java.net.URL
import javax.net.ssl.HttpsURLConnection

/**
 * [HttpTransport] over the platform `HttpsURLConnection`: HTTPS only (the network security
 * configuration also forbids cleartext), no redirect, no cache, no cookie handler, bounded
 * timeouts and a bounded response body.
 */
class UrlConnectionTransport : HttpTransport {
    override fun execute(request: HttpRequest): HttpResponse {
        val connection = URL(request.url).openConnection() as? HttpsURLConnection
            ?: throw IOException("HTTPS required")
        try {
            connection.instanceFollowRedirects = false
            connection.useCaches = false
            connection.connectTimeout = CONNECT_TIMEOUT_MS
            connection.readTimeout = READ_TIMEOUT_MS
            connection.requestMethod = request.method
            for ((name, value) in request.headers) {
                connection.setRequestProperty(name, value)
            }
            val body = request.body
            if (body != null) {
                connection.doOutput = true
                connection.setFixedLengthStreamingMode(body.size)
                connection.outputStream.use { it.write(body) }
            }
            val status = connection.responseCode
            val stream = if (status >= 400) connection.errorStream else connection.inputStream
            val bytes = stream?.use { readBounded(it) } ?: ByteArray(0)
            val cookies = connection.headerFields
                .filterKeys { it != null && it.equals("Set-Cookie", ignoreCase = true) }
                .values
                .flatten()
            return HttpResponse(status, bytes, cookies)
        } finally {
            connection.disconnect()
        }
    }

    companion object {
        const val CONNECT_TIMEOUT_MS = 5000
        const val READ_TIMEOUT_MS = 5000
        const val MAX_RESPONSE_BYTES = 16384

        /** Reads at most [MAX_RESPONSE_BYTES]; a longer body is an [IOException]. */
        fun readBounded(stream: InputStream): ByteArray {
            val out = ByteArrayOutputStream()
            val buffer = ByteArray(4096)
            while (true) {
                val n = stream.read(buffer)
                if (n < 0) break
                if (out.size() + n > MAX_RESPONSE_BYTES) throw IOException("response too large")
                out.write(buffer, 0, n)
            }
            return out.toByteArray()
        }
    }
}
