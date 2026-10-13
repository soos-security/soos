package org.soos.remote.data

import android.content.Context
import android.content.SharedPreferences

/**
 * App settings in private storage (no backup, no device transfer): the validated PC address
 * and the id of the passkey used for the last sign-in (public data, never a secret). The web
 * session cookie is never stored.
 */
class SettingsStore(context: Context) {
    private val prefs: SharedPreferences =
        context.getSharedPreferences(FILE_NAME, Context.MODE_PRIVATE)

    var pcUrl: String?
        get() = prefs.getString(KEY_PC_URL, null)
        set(value) {
            // A new PC address forgets the credential remembered for the previous one.
            prefs.edit().putString(KEY_PC_URL, value).remove(KEY_CREDENTIAL_ID).apply()
        }

    var credentialId: String?
        get() = prefs.getString(KEY_CREDENTIAL_ID, null)
        set(value) {
            prefs.edit().putString(KEY_CREDENTIAL_ID, value).apply()
        }

    private companion object {
        const val FILE_NAME = "soos_settings"
        const val KEY_PC_URL = "pc_url"
        const val KEY_CREDENTIAL_ID = "credential_id"
    }
}
