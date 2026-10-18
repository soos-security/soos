package org.soos.remote.alerts

import org.soos.remote.net.ApiResult

/** The fixed feedback of an acknowledgement (the UI maps each one to a `strings.xml` text). */
enum class AckFeedback {
    DONE,
    STALE_VIEW,
    RATE_LIMITED,
    DISABLED,
    UNAVAILABLE,
    BAD_REQUEST,
    LOGIN_REQUIRED,
    REFUSED,
    UNREACHABLE,
}

/** What one refresh of the home screen found. */
enum class RefreshOutcome {
    /** Signed in: the status and the alerts card were loaded. */
    AUTHENTICATED,

    /** Not signed in (or the auth state was refused): the alerts card is hidden. */
    SIGNED_OUT,

    /** No answer from the PC: the screen shows its own unreachable message. */
    UNREACHABLE,
}

/**
 * What to show after a refresh: [card] under the alerts card (visible only while signed in),
 * [notice] as the screen message (visible while the alerts card is hidden).
 */
data class FeedbackDisplay(val card: AckFeedback?, val notice: AckFeedback?) {
    companion object {
        val NONE = FeedbackDisplay(card = null, notice = null)
    }
}

/**
 * The acknowledgement feedback of the alerts card (GitHub #420). Every refresh starts from no
 * feedback, so an "Acknowledged." never outlives the view it acknowledged; the refusal of an
 * acknowledgement is carried through its follow-up refresh and shown under the fresh view.
 * Pure: no Android API, no I/O; a server token only selects a fixed feedback.
 */
object AlertsFeedback {
    /** The feedback of an acknowledgement answer. */
    fun ofAck(result: ApiResult<*>): AckFeedback = when (result) {
        is ApiResult.Ok -> AckFeedback.DONE
        is ApiResult.Refused -> when (result.result) {
            "stale_view" -> AckFeedback.STALE_VIEW
            "rate_limited" -> AckFeedback.RATE_LIMITED
            "alerts_disabled" -> AckFeedback.DISABLED
            "unavailable" -> AckFeedback.UNAVAILABLE
            "bad_request" -> AckFeedback.BAD_REQUEST
            "login_required" -> AckFeedback.LOGIN_REQUIRED
            else -> AckFeedback.REFUSED
        }
        ApiResult.Unreachable -> AckFeedback.UNREACHABLE
    }

    /**
     * Whether the acknowledgement needs a follow-up refresh: every answer but `200`, whose body
     * is already the fresh view (a refused view is never acknowledged blindly).
     */
    fun needsRefresh(result: ApiResult<*>): Boolean = result !is ApiResult.Ok

    /**
     * The feedback once a refresh is done. [carried] is the feedback of the acknowledgement that
     * asked for this refresh, `null` for every other refresh (user, pull, sign-in, lock, screen
     * opening), which therefore shows no feedback. Signed in, the carried feedback stays under
     * the alerts card; signed out, the card is hidden and only the login-required text moves to
     * the screen message; unreachable, the screen's own unreachable message is enough.
     */
    fun afterRefresh(carried: AckFeedback?, outcome: RefreshOutcome): FeedbackDisplay = when (outcome) {
        RefreshOutcome.AUTHENTICATED -> FeedbackDisplay(card = carried, notice = null)
        RefreshOutcome.SIGNED_OUT -> if (carried == AckFeedback.LOGIN_REQUIRED) {
            FeedbackDisplay(card = null, notice = AckFeedback.LOGIN_REQUIRED)
        } else {
            FeedbackDisplay.NONE
        }
        RefreshOutcome.UNREACHABLE -> FeedbackDisplay.NONE
    }
}
