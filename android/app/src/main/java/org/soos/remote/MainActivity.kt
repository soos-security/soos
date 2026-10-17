package org.soos.remote

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import org.soos.remote.data.SettingsStore
import org.soos.remote.heartbeat.HeartbeatController
import org.soos.remote.heartbeat.PlatformHeartbeatScheduler
import org.soos.remote.ui.SoosApp
import org.soos.remote.ui.theme.SoosTheme

/** The only exported component: the launcher activity. */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val settings = SettingsStore(applicationContext)
        val heartbeat = HeartbeatController(settings, PlatformHeartbeatScheduler(applicationContext))
        // A force stop removes the persisted job: opening the app schedules it again.
        heartbeat.ensureScheduled()
        setContent {
            SoosTheme {
                SoosApp(activity = this, settings = settings, heartbeat = heartbeat)
            }
        }
    }
}
