package org.soos.remote

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.soos.remote.alerts.AckFeedback
import org.soos.remote.alerts.AlertsFeedback
import org.soos.remote.alerts.FeedbackDisplay
import org.soos.remote.alerts.RefreshOutcome
import org.soos.remote.net.AlertsView
import org.soos.remote.net.ApiResult

class AlertsFeedbackTest {
    private val view = AlertsView(
        state = "active",
        reason = null,
        epoch = "0123456789abcdef",
        lockScreen = "monitored",
        unacknowledgedWrongPassword = 0,
        unacknowledgedLockedOut = 0,
        lastUnixMs = null,
        lastSource = null,
        through = 4,
        history = emptyList(),
    )

    @Test
    fun aSuccessfulAckThenARefreshClearsTheFeedback() {
        val ack = AlertsFeedback.ofAck(ApiResult.Ok(view))
        assertEquals(AckFeedback.DONE, ack)
        assertEquals(false, AlertsFeedback.needsRefresh(ApiResult.Ok(view)))
        // A user, pull or sign-in refresh carries no acknowledgement: the card shows nothing.
        for (outcome in RefreshOutcome.entries) {
            assertEquals(FeedbackDisplay.NONE, AlertsFeedback.afterRefresh(null, outcome))
        }
        assertNull(FeedbackDisplay.NONE.card)
        assertNull(FeedbackDisplay.NONE.notice)
    }

    @Test
    fun staleViewKeepsItsTextAfterTheFollowUpRefresh() {
        val refusal = ApiResult.Refused(409, "stale_view")
        val ack = AlertsFeedback.ofAck(refusal)
        assertEquals(AckFeedback.STALE_VIEW, ack)
        assertEquals(true, AlertsFeedback.needsRefresh(refusal))
        assertEquals(
            FeedbackDisplay(card = AckFeedback.STALE_VIEW, notice = null),
            AlertsFeedback.afterRefresh(ack, RefreshOutcome.AUTHENTICATED),
        )
        val limited = AlertsFeedback.ofAck(ApiResult.Refused(429, "rate_limited"))
        assertEquals(AckFeedback.RATE_LIMITED, limited)
        assertEquals(
            FeedbackDisplay(card = AckFeedback.RATE_LIMITED, notice = null),
            AlertsFeedback.afterRefresh(limited, RefreshOutcome.AUTHENTICATED),
        )
    }

    @Test
    fun loginRequiredThenASignedOutRefreshStillShowsTheLoginRequiredText() {
        val refusal = ApiResult.Refused(403, "login_required")
        val ack = AlertsFeedback.ofAck(refusal)
        assertEquals(AckFeedback.LOGIN_REQUIRED, ack)
        assertEquals(true, AlertsFeedback.needsRefresh(refusal))
        // The signed-out refresh hides the alerts card: the text moves to the screen message.
        assertEquals(
            FeedbackDisplay(card = null, notice = AckFeedback.LOGIN_REQUIRED),
            AlertsFeedback.afterRefresh(ack, RefreshOutcome.SIGNED_OUT),
        )
        // Still signed in after all (another tab signed in again): the card keeps the text.
        assertEquals(
            FeedbackDisplay(card = AckFeedback.LOGIN_REQUIRED, notice = null),
            AlertsFeedback.afterRefresh(ack, RefreshOutcome.AUTHENTICATED),
        )
    }

    @Test
    fun anUnreachableAckShowsTheUnreachableText() {
        val ack = AlertsFeedback.ofAck(ApiResult.Unreachable)
        assertEquals(AckFeedback.UNREACHABLE, ack)
        assertEquals(true, AlertsFeedback.needsRefresh(ApiResult.Unreachable))
        assertEquals(
            FeedbackDisplay(card = AckFeedback.UNREACHABLE, notice = null),
            AlertsFeedback.afterRefresh(ack, RefreshOutcome.AUTHENTICATED),
        )
        // The PC is still unreachable: the refresh shows its own unreachable message instead.
        assertEquals(
            FeedbackDisplay.NONE,
            AlertsFeedback.afterRefresh(ack, RefreshOutcome.UNREACHABLE),
        )
    }

    @Test
    fun everyRefusalTokenSelectsAFixedFeedback() {
        for ((token, expected) in listOf(
            "stale_view" to AckFeedback.STALE_VIEW,
            "rate_limited" to AckFeedback.RATE_LIMITED,
            "alerts_disabled" to AckFeedback.DISABLED,
            "unavailable" to AckFeedback.UNAVAILABLE,
            "bad_request" to AckFeedback.BAD_REQUEST,
            "login_required" to AckFeedback.LOGIN_REQUIRED,
            "forbidden" to AckFeedback.REFUSED,
            "something_new" to AckFeedback.REFUSED,
            null to AckFeedback.REFUSED,
        )) {
            assertEquals("$token", expected, AlertsFeedback.ofAck(ApiResult.Refused(400, token)))
        }
    }

    @Test
    fun aSignedOutRefreshShowsOnlyTheLoginRequiredNotice() {
        for (ack in AckFeedback.entries) {
            val expected = if (ack == AckFeedback.LOGIN_REQUIRED) {
                FeedbackDisplay(card = null, notice = AckFeedback.LOGIN_REQUIRED)
            } else {
                FeedbackDisplay.NONE
            }
            assertEquals("$ack", expected, AlertsFeedback.afterRefresh(ack, RefreshOutcome.SIGNED_OUT))
            assertEquals("$ack", FeedbackDisplay.NONE, AlertsFeedback.afterRefresh(ack, RefreshOutcome.UNREACHABLE))
        }
    }
}
