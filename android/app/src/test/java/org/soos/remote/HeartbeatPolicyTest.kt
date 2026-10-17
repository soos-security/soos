package org.soos.remote

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.soos.remote.heartbeat.CheckOutcome
import org.soos.remote.heartbeat.HeartbeatPolicy
import org.soos.remote.heartbeat.HeartbeatState
import org.soos.remote.heartbeat.Notice
import org.soos.remote.net.ApiResult
import org.soos.remote.net.StatusView

class HeartbeatPolicyTest {
    private val status = StatusView("unlocked", active = true, idle = false, checkedUnixMs = 5)

    /** Feeds [outcomes] from a fresh state; returns every notice (nulls included) and the end state. */
    private fun feed(vararg outcomes: CheckOutcome): Pair<List<Notice?>, HeartbeatState> {
        var state = HeartbeatState()
        val notices = mutableListOf<Notice?>()
        for (outcome in outcomes) {
            val step = HeartbeatPolicy.next(state, outcome)
            notices += step.notice
            state = step.state
        }
        return notices to state
    }

    @Test
    fun defaultsAreTheOwnerDecision() {
        assertEquals(900_000L, HeartbeatPolicy.INTERVAL_MS)
        assertEquals(300_000L, HeartbeatPolicy.FLEX_MS)
        assertEquals(2, HeartbeatPolicy.FAILURES_BEFORE_ALERT)
        val job = HeartbeatPolicy.JOB
        assertEquals(HeartbeatPolicy.JOB_ID, job.jobId)
        assertEquals(15L * 60L * 1000L, job.intervalMs)
        assertEquals(5L * 60L * 1000L, job.flexMs)
        assertTrue("the job survives a reboot", job.persisted)
        assertEquals(HeartbeatState(0, offlineNotified = false, signInNotified = false), HeartbeatState())
    }

    @Test
    fun classifyMapsEveryApiResult() {
        assertEquals(CheckOutcome.REACHABLE, HeartbeatPolicy.classify(ApiResult.Ok(status)))
        assertEquals(
            CheckOutcome.SIGN_IN_NEEDED,
            HeartbeatPolicy.classify(ApiResult.Refused(403, "login_required")),
        )
        // Any other soos-remote refusal proves that the PC answered.
        for ((code, result) in listOf(403 to "forbidden", 429 to "rate_limited", 503 to "busy", 503 to "unavailable", 421 to "misdirected_request")) {
            assertEquals(
                "$code $result",
                CheckOutcome.REACHABLE,
                HeartbeatPolicy.classify(ApiResult.Refused(code, result)),
            )
        }
        // No soos-remote answer: a gateway error without a result token, or no answer at all.
        assertEquals(CheckOutcome.UNREACHABLE, HeartbeatPolicy.classify(ApiResult.Refused(502, null)))
        assertEquals(CheckOutcome.UNREACHABLE, HeartbeatPolicy.classify(ApiResult.Refused(403, null)))
        assertEquals(CheckOutcome.UNREACHABLE, HeartbeatPolicy.classify(ApiResult.Unreachable))
    }

    @Test
    fun oneFailureDoesNotAlert() {
        val (notices, state) = feed(CheckOutcome.UNREACHABLE)
        assertEquals(listOf<Notice?>(null), notices)
        assertEquals(1, state.failures)
        assertFalse(state.offlineNotified)
    }

    @Test
    fun twoConsecutiveFailuresGiveExactlyOneAlert() {
        val (notices, state) = feed(CheckOutcome.UNREACHABLE, CheckOutcome.UNREACHABLE)
        assertEquals(listOf(null, Notice.PC_OFFLINE), notices)
        assertEquals(2, state.failures)
        assertTrue(state.offlineNotified)
    }

    @Test
    fun aLongOutageNeverAlertsTwice() {
        val outcomes = Array(20) { CheckOutcome.UNREACHABLE }
        val (notices, state) = feed(*outcomes)
        assertEquals(1, notices.count { it == Notice.PC_OFFLINE })
        assertEquals(listOf(null, Notice.PC_OFFLINE), notices.take(2))
        assertTrue(notices.drop(2).all { it == null })
        assertEquals("the counter saturates", HeartbeatPolicy.FAILURES_BEFORE_ALERT, state.failures)
    }

    @Test
    fun recoveryAfterAnAlertGivesExactlyOneBackOnlineNotice() {
        val (notices, state) = feed(
            CheckOutcome.UNREACHABLE,
            CheckOutcome.UNREACHABLE,
            CheckOutcome.UNREACHABLE,
            CheckOutcome.REACHABLE,
            CheckOutcome.REACHABLE,
            CheckOutcome.REACHABLE,
        )
        assertEquals(listOf(null, Notice.PC_OFFLINE, null, Notice.PC_BACK_ONLINE, null, null), notices)
        assertEquals(HeartbeatState(), state)
    }

    @Test
    fun aSingleFailureThenRecoveryIsSilent() {
        val (notices, state) = feed(
            CheckOutcome.UNREACHABLE,
            CheckOutcome.REACHABLE,
            CheckOutcome.UNREACHABLE,
            CheckOutcome.REACHABLE,
        )
        assertTrue(notices.all { it == null })
        assertEquals(HeartbeatState(), state)
    }

    @Test
    fun failuresMustBeConsecutive() {
        val (notices, _) = feed(
            CheckOutcome.UNREACHABLE,
            CheckOutcome.REACHABLE,
            CheckOutcome.UNREACHABLE,
        )
        assertTrue(notices.none { it == Notice.PC_OFFLINE })
    }

    @Test
    fun aSecondOutageAlertsAgain() {
        val (notices, _) = feed(
            CheckOutcome.UNREACHABLE,
            CheckOutcome.UNREACHABLE,
            CheckOutcome.REACHABLE,
            CheckOutcome.UNREACHABLE,
            CheckOutcome.UNREACHABLE,
            CheckOutcome.REACHABLE,
        )
        assertEquals(
            listOf(null, Notice.PC_OFFLINE, Notice.PC_BACK_ONLINE, null, Notice.PC_OFFLINE, Notice.PC_BACK_ONLINE),
            notices,
        )
    }

    @Test
    fun signInNeededIsReportedOncePerOptIn() {
        val (notices, state) = feed(
            CheckOutcome.SIGN_IN_NEEDED,
            CheckOutcome.SIGN_IN_NEEDED,
            CheckOutcome.SIGN_IN_NEEDED,
        )
        assertEquals(listOf(Notice.SIGN_IN_NEEDED, null, null), notices)
        assertTrue(state.signInNotified)
        assertEquals(0, state.failures)
        // A short outage in between does not re-arm the notice.
        val (again, _) = feed(
            CheckOutcome.SIGN_IN_NEEDED,
            CheckOutcome.UNREACHABLE,
            CheckOutcome.SIGN_IN_NEEDED,
        )
        assertEquals(listOf(Notice.SIGN_IN_NEEDED, null, null), again)
        // A later read with a session does not re-arm it: a check from an ended app process has
        // no session by design, so the notice is never repeated after every app run.
        val (later, end) = feed(
            CheckOutcome.SIGN_IN_NEEDED,
            CheckOutcome.REACHABLE,
            CheckOutcome.SIGN_IN_NEEDED,
            CheckOutcome.REACHABLE,
            CheckOutcome.SIGN_IN_NEEDED,
        )
        assertEquals(listOf(Notice.SIGN_IN_NEEDED, null, null, null, null), later)
        assertTrue(end.signInNotified)
    }

    @Test
    fun signInNeededCountsAsReachable() {
        // One failure, then a soos-remote login_required answer: the counter restarts.
        val (notices, state) = feed(
            CheckOutcome.UNREACHABLE,
            CheckOutcome.SIGN_IN_NEEDED,
            CheckOutcome.UNREACHABLE,
        )
        assertTrue(notices.none { it == Notice.PC_OFFLINE })
        assertEquals(1, state.failures)
        // After an alert, the first login_required answer is the back-online notice; the
        // sign-in notice follows at the next check.
        val (afterAlert, end) = feed(
            CheckOutcome.UNREACHABLE,
            CheckOutcome.UNREACHABLE,
            CheckOutcome.SIGN_IN_NEEDED,
            CheckOutcome.SIGN_IN_NEEDED,
            CheckOutcome.SIGN_IN_NEEDED,
        )
        assertEquals(
            listOf(null, Notice.PC_OFFLINE, Notice.PC_BACK_ONLINE, Notice.SIGN_IN_NEEDED, null),
            afterAlert,
        )
        assertFalse(end.offlineNotified)
        assertEquals(0, end.failures)
    }

    @Test
    fun anOutageDuringASignInEpisodeStillAlerts() {
        val (notices, _) = feed(
            CheckOutcome.SIGN_IN_NEEDED,
            CheckOutcome.UNREACHABLE,
            CheckOutcome.UNREACHABLE,
            CheckOutcome.SIGN_IN_NEEDED,
        )
        assertEquals(listOf(Notice.SIGN_IN_NEEDED, null, Notice.PC_OFFLINE, Notice.PC_BACK_ONLINE), notices)
    }

    @Test
    fun storedStatesAreClamped() {
        assertEquals(0, HeartbeatPolicy.sanitize(HeartbeatState(failures = -7)).failures)
        assertEquals(2, HeartbeatPolicy.sanitize(HeartbeatState(failures = Int.MAX_VALUE)).failures)
        assertEquals(1, HeartbeatPolicy.sanitize(HeartbeatState(failures = 1)).failures)
        val flags = HeartbeatState(failures = 9, offlineNotified = true, signInNotified = true)
        assertEquals(HeartbeatState(2, offlineNotified = true, signInNotified = true), HeartbeatPolicy.sanitize(flags))
        // A corrupt count never overflows and never alerts twice.
        val step = HeartbeatPolicy.next(HeartbeatState(failures = Int.MAX_VALUE), CheckOutcome.UNREACHABLE)
        assertEquals(Notice.PC_OFFLINE, step.notice)
        assertEquals(2, step.state.failures)
        val negative = HeartbeatPolicy.next(HeartbeatState(failures = Int.MIN_VALUE), CheckOutcome.UNREACHABLE)
        assertNull(negative.notice)
        assertEquals(1, negative.state.failures)
    }
}
