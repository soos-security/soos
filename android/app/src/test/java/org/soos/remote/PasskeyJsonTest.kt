package org.soos.remote

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.soos.remote.auth.PasskeyJson
import org.soos.remote.net.LoginOptions

class PasskeyJsonTest {
    private val options = LoginOptions("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8", "pc.tail1234.ts.net", 120000)

    @Test
    fun discoverableRequestWithoutRememberedCredential() {
        val json = JSONObject(PasskeyJson.requestJson(options, null))
        assertEquals(options.challenge, json.getString("challenge"))
        assertEquals("pc.tail1234.ts.net", json.getString("rpId"))
        assertEquals(120000L, json.getLong("timeout"))
        assertEquals("required", json.getString("userVerification"))
        assertFalse(json.has("allowCredentials"))
    }

    @Test
    fun rememberedCredentialIsNamed() {
        val json = JSONObject(PasskeyJson.requestJson(options, "Y3JlZC1pZA"))
        val allow = json.getJSONArray("allowCredentials")
        assertEquals(1, allow.length())
        val entry = allow.getJSONObject(0)
        assertEquals("public-key", entry.getString("type"))
        assertEquals("Y3JlZC1pZA", entry.getString("id"))
        assertEquals("internal", entry.getJSONArray("transports").getString(0))
        // A malformed remembered id is ignored (discoverable flow).
        assertFalse(JSONObject(PasskeyJson.requestJson(options, "not base64!")).has("allowCredentials"))
    }

    @Test
    fun assertionBodyMapsCredentialManagerJson() {
        val response = """
            {"id":"Y3JlZC1pZA","rawId":"Y3JlZC1pZA","type":"public-key",
             "response":{"clientDataJSON":"e30","authenticatorData":"YWQ=","signature":"c2ln","userHandle":"aGFuZGxl"}}
        """.trimIndent()
        val body = PasskeyJson.assertionBody(response)!!
        assertEquals("Y3JlZC1pZA", body.credentialId)
        val json = JSONObject(body.json)
        assertEquals("Y3JlZC1pZA", json.getString("id"))
        assertEquals("e30", json.getString("client_data_json"))
        assertEquals("YWQ", json.getString("authenticator_data"))
        assertEquals("c2ln", json.getString("signature"))
        assertEquals("aGFuZGxl", json.getString("user_handle"))
        assertEquals(5, json.length())
    }

    @Test
    fun malformedResponsesAreRefused() {
        assertNull(PasskeyJson.assertionBody("""{"id":"x","type":"password","response":{}}"""))
        assertNull(PasskeyJson.assertionBody("""{"id":"Y3","type":"public-key"}"""))
        assertNull(
            PasskeyJson.assertionBody(
                """{"id":"Y3","type":"public-key","response":{"clientDataJSON":"e30","authenticatorData":"a+b","signature":"c2ln"}}""",
            ),
        )
        assertNull(PasskeyJson.assertionBody("x".repeat(PasskeyJson.MAX_RESPONSE_JSON_CHARS + 1)))
        val noHandle = PasskeyJson.assertionBody(
            """{"id":"Y3","type":"public-key","response":{"clientDataJSON":"e30","authenticatorData":"YWQ","signature":"c2ln"}}""",
        )
        assertTrue(noHandle != null && !JSONObject(noHandle.json).has("user_handle"))
    }
}
