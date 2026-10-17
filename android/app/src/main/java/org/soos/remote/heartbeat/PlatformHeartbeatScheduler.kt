package org.soos.remote.heartbeat

import android.app.job.JobInfo
import android.app.job.JobScheduler
import android.content.ComponentName
import android.content.Context

/**
 * [HeartbeatScheduler] over the platform `JobScheduler`: one periodic job, persisted across
 * reboots (needs only the boot permission of the manifest, no receiver). No network constraint: on
 * Android 14 and later it needs `ACCESS_NETWORK_STATE`, which the app does not request.
 */
class PlatformHeartbeatScheduler(private val context: Context) : HeartbeatScheduler {
    override fun schedule(spec: HeartbeatJobSpec): Boolean {
        val scheduler = context.getSystemService(JobScheduler::class.java) ?: return false
        return try {
            val job = JobInfo.Builder(spec.jobId, ComponentName(context, HeartbeatJobService::class.java))
                .setPeriodic(spec.intervalMs, spec.flexMs)
                .setPersisted(spec.persisted)
                .build()
            scheduler.schedule(job) == JobScheduler.RESULT_SUCCESS
        } catch (ignored: IllegalArgumentException) {
            false
        } catch (ignored: SecurityException) {
            false
        } catch (ignored: IllegalStateException) {
            // The platform's job-count limit.
            false
        }
    }

    override fun cancel(jobId: Int) {
        context.getSystemService(JobScheduler::class.java)?.cancel(jobId)
    }

    override fun isScheduled(jobId: Int): Boolean =
        context.getSystemService(JobScheduler::class.java)?.getPendingJob(jobId) != null
}
