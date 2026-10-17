package org.soos.remote.heartbeat

import android.app.job.JobParameters
import android.app.job.JobService
import android.provider.Settings
import java.util.concurrent.atomic.AtomicBoolean
import org.soos.remote.config.PcUrl
import org.soos.remote.config.PcUrlResult
import org.soos.remote.data.SettingsStore
import org.soos.remote.net.SharedApi

/**
 * The opt-in heartbeat job (not exported, bound only by the system through
 * `BIND_JOB_SERVICE`): one `GET /api/status` on the saved PC, then the state machine and at
 * most one local notification. A stopped run is discarded, never counted and never retried
 * with backoff; the periodic schedule continues. Nothing is logged.
 */
class HeartbeatJobService : JobService() {
    /** The stop flag of the current run; each run captures its own. */
    @Volatile
    private var current: AtomicBoolean? = null

    override fun onStartJob(params: JobParameters): Boolean {
        val stopped = AtomicBoolean(false)
        current = stopped
        val app = applicationContext
        // Everything, the storage reads and writes included, runs off the main thread.
        Thread {
            try {
                val settings = SettingsStore(app)
                val controller = HeartbeatController(settings, PlatformHeartbeatScheduler(app))
                val saved = settings.pcUrl
                val pc = saved?.let { PcUrl.parse(it) } as? PcUrlResult.Valid
                val airplane =
                    Settings.Global.getInt(app.contentResolver, Settings.Global.AIRPLANE_MODE_ON, 0) != 0
                // A job left over from an older opt-in is cancelled (one locked step, no write).
                if (!controller.cancelIfDisabled() && saved != null && pc != null && !airplane) {
                    // Without a usable address, or in airplane mode, nothing is counted.
                    val outcome = HeartbeatPolicy.classify(SharedApi.forOrigin(pc.origin).status())
                    // A run the system stopped is discarded; once a state is stored, its notice
                    // is always shown.
                    if (!stopped.get()) {
                        controller.onCheck(saved, outcome)?.let { HeartbeatNotifier(app).show(it) }
                    }
                }
            } catch (ignored: RuntimeException) {
                // An unexpected failure discards this check; the next period runs normally.
            } finally {
                jobFinished(params, false)
            }
        }.start()
        return true
    }

    override fun onStopJob(params: JobParameters): Boolean {
        current?.set(true)
        return false
    }
}
