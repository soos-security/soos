package org.soos.remote.auth

import org.json.JSONArray
import org.json.JSONObject
import org.soos.remote.net.ApiJson
import org.soos.remote.net.LoginOptions
import org.soos.remote.net.SessionCookie

/** The login verify body and the credential id that produced it (public data). */
data class AssertionBody(val json: String, val credentialId: String)

/**
 * Pure mapping between the `soos-remote` login ceremony and Android Credential Manager's
 * WebAuthn JSON (GitHub #382, #354 part 2).
 */
object PasskeyJson {
    /** Longest accepted Credential Manager response. */
    const val MAX_RESPONSE_JSON_CHARS = 16384

    /** Longest accepted credential id (base64url of 1023 bytes). */
    private const val MAX_ID_CHARS = 1368

    /**
     * The `GetPublicKeyCredentialOption` request: user verification required. When the app
     * remembers the credential of its last sign-in, it names it (`allowCredentials`), so
     * Credential Manager goes straight to the biometric prompt; the server never discloses
     * a credential id to an anonymous client (owner decision 2026-10-12 on #354).
     */
    fun requestJson(options: LoginOptions, rememberedCredentialId: String?): String {
        val json = JSONObject()
        json.put("challenge", options.challenge)
        json.put("rpId", options.rpId)
        json.put("timeout", options.timeoutMs)
        json.put("userVerification", "required")
        val remembered = rememberedCredentialId?.takeIf(::isCredentialId)
        if (remembered != null) {
            val credential = JSONObject()
            credential.put("type", "public-key")
            credential.put("id", remembered)
            credential.put("transports", JSONArray(listOf("internal", "hybrid")))
            json.put("allowCredentials", JSONArray().put(credential))
        }
        return json.toString()
    }

    /** Maps `PublicKeyCredential.authenticationResponseJson` to the server's body. */
    fun assertionBody(responseJson: String): AssertionBody? {
        if (responseJson.length > MAX_RESPONSE_JSON_CHARS) return null
        val json = ApiJson.objectOf(responseJson) ?: return null
        if (ApiJson.string(json, "type") != "public-key") return null
        val id = field(ApiJson.string(json, "id")) ?: return null
        if (!isCredentialId(id)) return null
        val response = if (json.has("response")) json.get("response") as? JSONObject else null
        response ?: return null
        val clientData = field(ApiJson.string(response, "clientDataJSON")) ?: return null
        val authenticatorData = field(ApiJson.string(response, "authenticatorData")) ?: return null
        val signature = field(ApiJson.string(response, "signature")) ?: return null
        val body = JSONObject()
        body.put("id", id)
        body.put("client_data_json", clientData)
        body.put("authenticator_data", authenticatorData)
        body.put("signature", signature)
        if (response.has("userHandle")) {
            val raw = response.get("userHandle")
            if (raw != JSONObject.NULL) {
                val handle = field(raw as? String) ?: return null
                body.put("user_handle", handle)
            }
        }
        return AssertionBody(body.toString(), id)
    }

    /** A non-empty base64url field; trailing padding is stripped (the server refuses it). */
    private fun field(value: String?): String? {
        val trimmed = value?.trimEnd('=') ?: return null
        if (trimmed.isEmpty() || !trimmed.all(SessionCookie::isBase64Url)) return null
        return trimmed
    }

    private fun isCredentialId(value: String): Boolean =
        value.isNotEmpty() && value.length <= MAX_ID_CHARS && value.all(SessionCookie::isBase64Url)
}
