package org.soos.remote.data

import android.content.Context
import android.content.SharedPreferences
import org.soos.remote.heartbeat.HeartbeatPolicy
import org.soos.remote.heartbeat.HeartbeatState
import org.soos.remote.heartbeat.HeartbeatStore

/**
 * App settings in private storage (no backup, no device transfer): the validated PC address,
 * the id of the passkey used for the last sign-in (public data, never a secret) and the opt-in
 * heartbeat (a switch, a failure count and two flags). The web session cookie is never stored.
 */
class SettingsStore(context: Context) : HeartbeatStore {
    private val prefs: SharedPreferences =
        context.getSharedPreferences(FILE_NAME, Context.MODE_PRIVATE)

    /** Saved through HeartbeatController.savePcUrl, which also resets the heartbeat state. */
    override var pcUrl: String?
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

    /** Off by default. */
    override var heartbeatEnabled: Boolean
        get() = prefs.getBoolean(KEY_HEARTBEAT_ENABLED, false)
        set(value) {
            prefs.edit().putBoolean(KEY_HEARTBEAT_ENABLED, value).apply()
        }

    override fun loadHeartbeatState(): HeartbeatState = HeartbeatPolicy.sanitize(
        HeartbeatState(
            failures = prefs.getInt(KEY_HEARTBEAT_FAILURES, 0),
            offlineNotified = prefs.getBoolean(KEY_HEARTBEAT_OFFLINE_NOTIFIED, false),
            signInNotified = prefs.getBoolean(KEY_HEARTBEAT_SIGN_IN_NOTIFIED, false),
        ),
    )

    /** Synchronous, so a state survives the job process ending right after the check. */
    override fun saveHeartbeatState(state: HeartbeatState): Boolean =
        prefs.edit()
            .putInt(KEY_HEARTBEAT_FAILURES, state.failures)
            .putBoolean(KEY_HEARTBEAT_OFFLINE_NOTIFIED, state.offlineNotified)
            .putBoolean(KEY_HEARTBEAT_SIGN_IN_NOTIFIED, state.signInNotified)
            .commit()

    private companion object {
        const val FILE_NAME = "soos_settings"
        const val KEY_PC_URL = "pc_url"
        const val KEY_CREDENTIAL_ID = "credential_id"
        const val KEY_HEARTBEAT_ENABLED = "heartbeat_enabled"
        const val KEY_HEARTBEAT_FAILURES = "heartbeat_failures"
        const val KEY_HEARTBEAT_OFFLINE_NOTIFIED = "heartbeat_offline_notified"
        const val KEY_HEARTBEAT_SIGN_IN_NOTIFIED = "heartbeat_sign_in_notified"
    }
}
