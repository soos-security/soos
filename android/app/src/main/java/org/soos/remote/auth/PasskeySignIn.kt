package org.soos.remote.auth

import android.app.Activity
import androidx.credentials.CredentialManager
import androidx.credentials.GetCredentialRequest
import androidx.credentials.GetPublicKeyCredentialOption
import androidx.credentials.PublicKeyCredential
import androidx.credentials.exceptions.GetCredentialCancellationException
import androidx.credentials.exceptions.GetCredentialException
import androidx.credentials.exceptions.NoCredentialException

/** Outcome of the Credential Manager prompt. */
sealed interface PasskeyOutcome {
    data class Response(val json: String) : PasskeyOutcome

    data object Cancelled : PasskeyOutcome

    data object NoCredential : PasskeyOutcome

    data object Failed : PasskeyOutcome
}

/**
 * The passkey prompt through Android Credential Manager (platform implementation on
 * Android 14+, no Google Play services library). The biometric or screen-lock check happens
 * inside the provider; the app only sees the signed WebAuthn response.
 */
class PasskeySignIn(private val activity: Activity) {
    suspend fun getAssertion(requestJson: String): PasskeyOutcome {
        val manager = CredentialManager.create(activity)
        val request = GetCredentialRequest(listOf(GetPublicKeyCredentialOption(requestJson)))
        return try {
            val result = manager.getCredential(activity, request)
            val credential = result.credential
            if (credential is PublicKeyCredential) {
                PasskeyOutcome.Response(credential.authenticationResponseJson)
            } else {
                PasskeyOutcome.Failed
            }
        } catch (ignored: GetCredentialCancellationException) {
            PasskeyOutcome.Cancelled
        } catch (ignored: NoCredentialException) {
            PasskeyOutcome.NoCredential
        } catch (ignored: GetCredentialException) {
            PasskeyOutcome.Failed
        }
    }
}
