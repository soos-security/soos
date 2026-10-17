package org.soos.remote.heartbeat

/** The platform job scheduler seam (a fake in the unit tests). */
interface HeartbeatScheduler {
    /** Schedules (or replaces) the job; `false` when the platform refused it. */
    fun schedule(spec: HeartbeatJobSpec): Boolean

    fun cancel(jobId: Int)

    /** Whether the platform still holds the job (a force stop removes it). */
    fun isScheduled(jobId: Int): Boolean
}

/** The private app storage seam of the heartbeat (a fake in the unit tests). */
interface HeartbeatStore {
    /** The saved PC address (an origin), or `null`. */
    var pcUrl: String?

    var heartbeatEnabled: Boolean

    fun loadHeartbeatState(): HeartbeatState

    /** Writes [state] synchronously; `false` when the write failed. */
    fun saveHeartbeatState(state: HeartbeatState): Boolean
}

/**
 * Turns the opt-in heartbeat on and off and applies each check to the stored state. Off by
 * default; turning it off cancels the job. One process-wide lock serializes the settings
 * screen and a finishing job. Pure.
 */
class HeartbeatController(
    private val store: HeartbeatStore,
    private val scheduler: HeartbeatScheduler,
) {
    val enabled: Boolean
        get() = synchronized(LOCK) { store.heartbeatEnabled }

    /** The platform refused the job at the last [enable] or [ensureScheduled] (memory only). */
    var scheduleRefused: Boolean = false
        private set

    /** Starts from a fresh state and schedules the job; `false` (and off) when refused. */
    fun enable(): Boolean = synchronized(LOCK) {
        store.saveHeartbeatState(HeartbeatState())
        store.heartbeatEnabled = true
        scheduleOrTurnOff()
    }

    /**
     * Called when the app opens: an opted-in heartbeat whose job the platform dropped (a force
     * stop) is scheduled again, keeping its state; a pending job keeps its period. `true` when
     * the heartbeat is on afterwards.
     */
    fun ensureScheduled(): Boolean = synchronized(LOCK) {
        when {
            !store.heartbeatEnabled -> false
            scheduler.isScheduled(HeartbeatPolicy.JOB_ID) -> true
            else -> scheduleOrTurnOff()
        }
    }

    /** Schedules the job; when refused, the heartbeat is turned off. Caller holds [LOCK]. */
    private fun scheduleOrTurnOff(): Boolean {
        if (scheduler.schedule(HeartbeatPolicy.JOB)) {
            scheduleRefused = false
            return true
        }
        scheduleRefused = true
        store.heartbeatEnabled = false
        scheduler.cancel(HeartbeatPolicy.JOB_ID)
        return false
    }

    /** Cancels the job and forgets the state. */
    fun disable() {
        synchronized(LOCK) {
            scheduler.cancel(HeartbeatPolicy.JOB_ID)
            store.heartbeatEnabled = false
            store.saveHeartbeatState(HeartbeatState())
        }
    }

    /**
     * Cancels a job left over from an older opt-in, in one locked step (no write); `true` when
     * the heartbeat was off.
     */
    fun cancelIfDisabled(): Boolean {
        synchronized(LOCK) {
            if (store.heartbeatEnabled) return false
            scheduler.cancel(HeartbeatPolicy.JOB_ID)
            return true
        }
    }

    /**
     * Saves a PC address; another address starts the heartbeat over. Under the same lock as
     * [onCheck], which drops the result of a check made for another address, so a check of the
     * previous PC never counts for the new one.
     */
    fun savePcUrl(url: String) {
        synchronized(LOCK) {
            if (store.pcUrl != url) store.saveHeartbeatState(HeartbeatState())
            store.pcUrl = url
        }
    }

    /**
     * Applies one check of [checkedUrl]; the notice to show, if any. A stale job is cancelled;
     * a check of an address that is no longer the saved one is dropped (no write).
     */
    fun onCheck(checkedUrl: String, outcome: CheckOutcome): Notice? {
        synchronized(LOCK) {
            if (!store.heartbeatEnabled) {
                scheduler.cancel(HeartbeatPolicy.JOB_ID)
                return null
            }
            if (store.pcUrl != checkedUrl) return null
            val step = HeartbeatPolicy.next(store.loadHeartbeatState(), outcome)
            // A notice is shown only once its state is stored, so a failed write can never
            // repeat a notice at every period.
            return if (store.saveHeartbeatState(step.state)) step.notice else null
        }
    }

    private companion object {
        val LOCK = Any()
    }
}
