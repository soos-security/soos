package org.soos.remote

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.soos.remote.heartbeat.CheckOutcome
import org.soos.remote.heartbeat.HeartbeatController
import org.soos.remote.heartbeat.HeartbeatJobSpec
import org.soos.remote.heartbeat.HeartbeatPolicy
import org.soos.remote.heartbeat.HeartbeatScheduler
import org.soos.remote.heartbeat.HeartbeatState
import org.soos.remote.heartbeat.HeartbeatStore
import org.soos.remote.heartbeat.Notice

class HeartbeatControllerTest {
    private companion object {
        const val PC = "https://pc.tail1234.ts.net"
        const val OTHER = "https://other.tail1234.ts.net"
    }

    private class FakeStore : HeartbeatStore {
        override var pcUrl: String? = PC
        override var heartbeatEnabled: Boolean = false
        var state = HeartbeatState()
        var saves = 0

        override fun loadHeartbeatState(): HeartbeatState = state

        override fun saveHeartbeatState(state: HeartbeatState): Boolean {
            this.state = state
            saves += 1
            return true
        }
    }

    private class FakeScheduler(var accept: Boolean = true) : HeartbeatScheduler {
        val scheduled = mutableListOf<HeartbeatJobSpec>()
        val cancelled = mutableListOf<Int>()
        val pending = mutableSetOf<Int>()

        override fun schedule(spec: HeartbeatJobSpec): Boolean {
            scheduled += spec
            if (accept) pending += spec.jobId
            return accept
        }

        override fun cancel(jobId: Int) {
            cancelled += jobId
            pending -= jobId
        }

        override fun isScheduled(jobId: Int): Boolean = jobId in pending
    }

    @Test
    fun offByDefault() {
        val store = FakeStore()
        val scheduler = FakeScheduler()
        val controller = HeartbeatController(store, scheduler)
        assertFalse(controller.enabled)
        assertTrue(scheduler.scheduled.isEmpty())
        assertTrue(scheduler.cancelled.isEmpty())
    }

    @Test
    fun optInSchedulesThePersistedFifteenMinuteJobWithAFreshState() {
        val store = FakeStore()
        store.state = HeartbeatState(failures = 2, offlineNotified = true, signInNotified = true)
        val scheduler = FakeScheduler()
        val controller = HeartbeatController(store, scheduler)
        assertTrue(controller.enable())
        assertTrue(controller.enabled)
        assertTrue(store.heartbeatEnabled)
        assertEquals(listOf(HeartbeatPolicy.JOB), scheduler.scheduled)
        val spec = scheduler.scheduled.single()
        assertEquals(900_000L, spec.intervalMs)
        assertTrue(spec.persisted)
        assertEquals(HeartbeatState(), store.state)
    }

    @Test
    fun optOutCancelsTheJobAndResetsTheState() {
        val store = FakeStore()
        val scheduler = FakeScheduler()
        val controller = HeartbeatController(store, scheduler)
        assertTrue(controller.enable())
        controller.onCheck(PC, CheckOutcome.UNREACHABLE)
        assertEquals(1, store.state.failures)
        controller.disable()
        assertFalse(controller.enabled)
        assertFalse(store.heartbeatEnabled)
        assertEquals(listOf(HeartbeatPolicy.JOB_ID), scheduler.cancelled)
        assertEquals(HeartbeatState(), store.state)
    }

    @Test
    fun aCheckWhileDisabledCancelsTheStaleJobAndChangesNothing() {
        val store = FakeStore()
        val scheduler = FakeScheduler()
        val controller = HeartbeatController(store, scheduler)
        assertNull(controller.onCheck(PC, CheckOutcome.UNREACHABLE))
        assertNull(controller.onCheck(PC, CheckOutcome.UNREACHABLE))
        assertEquals(listOf(HeartbeatPolicy.JOB_ID, HeartbeatPolicy.JOB_ID), scheduler.cancelled)
        assertEquals(0, store.saves)
        assertEquals(HeartbeatState(), store.state)
    }

    @Test
    fun cancelIfDisabledCancelsOnlyAStaleJobAndNeverWrites() {
        val store = FakeStore()
        val scheduler = FakeScheduler()
        val controller = HeartbeatController(store, scheduler)
        assertTrue(controller.cancelIfDisabled())
        assertEquals(listOf(HeartbeatPolicy.JOB_ID), scheduler.cancelled)
        assertEquals(0, store.saves)
        assertTrue(controller.enable())
        val saves = store.saves
        assertFalse("an opted-in heartbeat is left alone", controller.cancelIfDisabled())
        assertEquals(listOf(HeartbeatPolicy.JOB_ID), scheduler.cancelled)
        assertTrue(controller.enabled)
        assertEquals(saves, store.saves)
    }

    @Test
    fun aNewPcAddressStartsOverWithoutABackOnlineNotice() {
        val store = FakeStore()
        val scheduler = FakeScheduler()
        val controller = HeartbeatController(store, scheduler)
        assertTrue(controller.enable())
        controller.onCheck(PC, CheckOutcome.UNREACHABLE)
        assertEquals(Notice.PC_OFFLINE, controller.onCheck(PC, CheckOutcome.UNREACHABLE))
        controller.savePcUrl(PC)
        assertTrue("the same address keeps the state", store.state.offlineNotified)
        controller.savePcUrl(OTHER)
        assertEquals(OTHER, store.pcUrl)
        assertEquals(HeartbeatState(), store.state)
        assertTrue("the heartbeat stays on", controller.enabled)
        assertEquals(listOf(HeartbeatPolicy.JOB), scheduler.scheduled)
        assertNull("no back-online notice for the new PC", controller.onCheck(OTHER, CheckOutcome.REACHABLE))
    }

    @Test
    fun aLateCheckOfThePreviousPcIsDropped() {
        val store = FakeStore()
        val controller = HeartbeatController(store, FakeScheduler())
        assertTrue(controller.enable())
        controller.onCheck(PC, CheckOutcome.UNREACHABLE)
        controller.savePcUrl(OTHER)
        val saves = store.saves
        // The job read the old address before the change; its result arrives afterwards.
        assertNull(controller.onCheck(PC, CheckOutcome.UNREACHABLE))
        assertNull(controller.onCheck(PC, CheckOutcome.SIGN_IN_NEEDED))
        assertEquals("nothing is written", saves, store.saves)
        assertEquals(HeartbeatState(), store.state)
        // One failure of the new PC alone does not alert.
        assertNull(controller.onCheck(OTHER, CheckOutcome.UNREACHABLE))
        assertEquals(Notice.PC_OFFLINE, controller.onCheck(OTHER, CheckOutcome.UNREACHABLE))
    }

    @Test
    fun aRefusedScheduleLeavesTheHeartbeatOff() {
        val store = FakeStore()
        val scheduler = FakeScheduler(accept = false)
        val controller = HeartbeatController(store, scheduler)
        assertFalse(controller.enable())
        assertFalse(controller.enabled)
        assertFalse(store.heartbeatEnabled)
        assertTrue(controller.scheduleRefused)
        assertEquals(listOf(HeartbeatPolicy.JOB_ID), scheduler.cancelled)
    }

    @Test
    fun checksDriveTheStateMachineAndPersistIt() {
        val store = FakeStore()
        val controller = HeartbeatController(store, FakeScheduler())
        assertTrue(controller.enable())
        val notices = listOf(
            CheckOutcome.UNREACHABLE,
            CheckOutcome.UNREACHABLE,
            CheckOutcome.UNREACHABLE,
            CheckOutcome.REACHABLE,
            CheckOutcome.REACHABLE,
        ).map { controller.onCheck(PC, it) }
        assertEquals(listOf(null, Notice.PC_OFFLINE, null, Notice.PC_BACK_ONLINE, null), notices)
        assertEquals(HeartbeatState(), store.state)
        assertTrue(store.saves >= 5)
    }

    @Test
    fun aStateRestoredAfterAProcessRestartIsHonoured() {
        // The process dies between two job runs: the next controller reads the stored state.
        val store = FakeStore()
        val scheduler = FakeScheduler()
        assertTrue(HeartbeatController(store, scheduler).enable())
        assertNull(HeartbeatController(store, scheduler).onCheck(PC, CheckOutcome.UNREACHABLE))
        assertEquals(Notice.PC_OFFLINE, HeartbeatController(store, scheduler).onCheck(PC, CheckOutcome.UNREACHABLE))
        assertNull(HeartbeatController(store, scheduler).onCheck(PC, CheckOutcome.UNREACHABLE))
        assertEquals(Notice.PC_BACK_ONLINE, HeartbeatController(store, scheduler).onCheck(PC, CheckOutcome.REACHABLE))
    }

    @Test
    fun openingTheAppReschedulesAJobDroppedByAForceStop() {
        val store = FakeStore()
        val scheduler = FakeScheduler()
        val controller = HeartbeatController(store, scheduler)
        assertTrue(controller.enable())
        controller.onCheck(PC, CheckOutcome.UNREACHABLE)
        scheduler.pending.clear() // force stop: the platform dropped the job
        assertTrue(controller.ensureScheduled())
        assertEquals(listOf(HeartbeatPolicy.JOB, HeartbeatPolicy.JOB), scheduler.scheduled)
        assertTrue(scheduler.isScheduled(HeartbeatPolicy.JOB_ID))
        assertTrue(controller.enabled)
        assertEquals("the outage tracking is kept", 1, store.state.failures)
        assertFalse(controller.scheduleRefused)
    }

    @Test
    fun aPendingJobKeepsItsPeriod() {
        val store = FakeStore()
        val scheduler = FakeScheduler()
        val controller = HeartbeatController(store, scheduler)
        assertTrue(controller.enable())
        assertTrue(controller.ensureScheduled())
        assertTrue(controller.ensureScheduled())
        assertEquals("never rescheduled while pending", listOf(HeartbeatPolicy.JOB), scheduler.scheduled)
        assertTrue(scheduler.cancelled.isEmpty())
    }

    @Test
    fun anOptedOutHeartbeatIsNeverScheduledOnOpen() {
        val store = FakeStore()
        val scheduler = FakeScheduler()
        val controller = HeartbeatController(store, scheduler)
        assertFalse(controller.ensureScheduled())
        assertTrue(controller.enable())
        controller.disable()
        assertFalse(controller.ensureScheduled())
        assertEquals(listOf(HeartbeatPolicy.JOB), scheduler.scheduled)
        assertFalse(controller.enabled)
    }

    @Test
    fun aRefusedRescheduleTurnsTheSwitchOff() {
        val store = FakeStore()
        val scheduler = FakeScheduler()
        val controller = HeartbeatController(store, scheduler)
        assertTrue(controller.enable())
        scheduler.pending.clear()
        scheduler.accept = false
        assertFalse(controller.ensureScheduled())
        assertFalse(controller.enabled)
        assertFalse(store.heartbeatEnabled)
        assertTrue(controller.scheduleRefused)
        scheduler.accept = true
        assertTrue(controller.enable())
        assertFalse(controller.scheduleRefused)
    }

    @Test
    fun reEnablingStartsOver() {
        val store = FakeStore()
        val scheduler = FakeScheduler()
        val controller = HeartbeatController(store, scheduler)
        assertTrue(controller.enable())
        controller.onCheck(PC, CheckOutcome.UNREACHABLE)
        controller.onCheck(PC, CheckOutcome.UNREACHABLE)
        assertTrue(store.state.offlineNotified)
        assertTrue(controller.enable())
        assertEquals(HeartbeatState(), store.state)
        // No back-online notice for an outage reported before the restart.
        assertNull(controller.onCheck(PC, CheckOutcome.REACHABLE))
        // The sign-in notice is given once per opt-in; turning the heartbeat on again re-arms it.
        assertEquals(Notice.SIGN_IN_NEEDED, controller.onCheck(PC, CheckOutcome.SIGN_IN_NEEDED))
        assertNull(controller.onCheck(PC, CheckOutcome.SIGN_IN_NEEDED))
        assertNull(controller.onCheck(PC, CheckOutcome.REACHABLE))
        assertNull(controller.onCheck(PC, CheckOutcome.SIGN_IN_NEEDED))
        assertTrue(controller.enable())
        assertEquals(Notice.SIGN_IN_NEEDED, controller.onCheck(PC, CheckOutcome.SIGN_IN_NEEDED))
    }
}
