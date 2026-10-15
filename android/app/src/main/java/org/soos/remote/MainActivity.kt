package org.soos.remote

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import org.soos.remote.data.SettingsStore
import org.soos.remote.ui.SoosApp
import org.soos.remote.ui.theme.SoosTheme

/** The only exported component: the launcher activity. */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val settings = SettingsStore(applicationContext)
        setContent {
            SoosTheme {
                SoosApp(activity = this, settings = settings)
            }
        }
    }
}
