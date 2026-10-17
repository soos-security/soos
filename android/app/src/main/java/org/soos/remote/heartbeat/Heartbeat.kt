package org.soos.remote.heartbeat

import org.soos.remote.net.ApiResult
import org.soos.remote.net.StatusView

/** What one check of `GET /api/status` proved. */
enum class CheckOutcome {
    /** The PC and `soos-remote` answered. */
    REACHABLE,

    /** `soos-remote` answered `403 login_required`: reachable, but the status needs a sign-in. */
    SIGN_IN_NEEDED,

    /** No answer from `soos-remote` (network error, timeout, malformed body, gateway error). */
    UNREACHABLE,
}

/** The local notifications of the heartbeat (fixed texts in [HeartbeatTexts]). */
enum class Notice { PC_OFFLINE, PC_BACK_ONLINE, SIGN_IN_NEEDED }

/**
 * The stored heartbeat state: consecutive failures (0..[HeartbeatPolicy.FAILURES_BEFORE_ALERT],
 * saturating), whether the current outage was notified, and whether the sign-in notice was
 * already shown since the heartbeat was turned on. Holds no secret and no PC data.
 */
data class HeartbeatState(
    val failures: Int = 0,
    val offlineNotified: Boolean = false,
    val signInNotified: Boolean = false,
)

/** The state after one check and the notice to show, if any. */
data class HeartbeatStep(val state: HeartbeatState, val notice: Notice?)

/** The platform job of the heartbeat (no network constraint: see the ADR amendment). */
data class HeartbeatJobSpec(val jobId: Int, val intervalMs: Long, val flexMs: Long, val persisted: Boolean)

/**
 * The opt-in PC-offline heartbeat (owner decision of 2026-10-17 on #359): a periodic check of
 * the PC status, one alert after [FAILURES_BEFORE_ALERT] consecutive failures, one back-online
 * notice on recovery, at most one sign-in notice per opt-in. Pure.
 */
object HeartbeatPolicy {
    /** The app's only job id. */
    const val JOB_ID = 359

    /** Period of the check: 15 min, the platform minimum. */
    const val INTERVAL_MS = 900_000L

    /** Flex of each period: 5 min, the platform minimum (checks are 10 to 20 min apart). */
    const val FLEX_MS = 300_000L

    /** Consecutive failed checks before the offline alert. */
    const val FAILURES_BEFORE_ALERT = 2

    /** The job: persisted, so it survives a reboot (the boot permission of the manifest, no receiver). */
    val JOB = HeartbeatJobSpec(JOB_ID, INTERVAL_MS, FLEX_MS, persisted = true)

    /** A `soos-remote` answer proves reachability; only no answer at all is a failure. */
    fun classify(result: ApiResult<StatusView>): CheckOutcome = when (result) {
        is ApiResult.Ok -> CheckOutcome.REACHABLE
        is ApiResult.Refused -> when {
            result.status == 403 && result.result == "login_required" -> CheckOutcome.SIGN_IN_NEEDED
            result.result != null -> CheckOutcome.REACHABLE
            else -> CheckOutcome.UNREACHABLE
        }
        ApiResult.Unreachable -> CheckOutcome.UNREACHABLE
    }

    /** A stored state with its counter clamped into `0..FAILURES_BEFORE_ALERT`. */
    fun sanitize(state: HeartbeatState): HeartbeatState =
        state.copy(failures = state.failures.coerceIn(0, FAILURES_BEFORE_ALERT))

    /** The transition of one check. */
    fun next(state: HeartbeatState, outcome: CheckOutcome): HeartbeatStep {
        val current = sanitize(state)
        return when (outcome) {
            CheckOutcome.UNREACHABLE -> {
                val failures = (current.failures + 1).coerceAtMost(FAILURES_BEFORE_ALERT)
                if (failures >= FAILURES_BEFORE_ALERT && !current.offlineNotified) {
                    HeartbeatStep(current.copy(failures = failures, offlineNotified = true), Notice.PC_OFFLINE)
                } else {
                    HeartbeatStep(current.copy(failures = failures), null)
                }
            }
            // The sign-in notice stays given: a background check without a session is normal
            // (the process ended), so it is shown once per opt-in, never after every app run.
            CheckOutcome.REACHABLE -> HeartbeatStep(
                HeartbeatState(signInNotified = current.signInNotified),
                if (current.offlineNotified) Notice.PC_BACK_ONLINE else null,
            )
            CheckOutcome.SIGN_IN_NEEDED -> when {
                current.offlineNotified -> HeartbeatStep(
                    current.copy(failures = 0, offlineNotified = false),
                    Notice.PC_BACK_ONLINE,
                )
                !current.signInNotified -> HeartbeatStep(
                    HeartbeatState(signInNotified = true),
                    Notice.SIGN_IN_NEEDED,
                )
                else -> HeartbeatStep(HeartbeatState(signInNotified = true), null)
            }
        }
    }
}

/** A notification text. */
data class NoticeText(val title: String, val body: String)

/** Fixed English notification texts: no PC name, address, status value or time. */
object HeartbeatTexts {
    const val CHANNEL_ID = "pc_status"
    const val CHANNEL_NAME = "PC status"
    const val CHANNEL_DESCRIPTION = "Alerts when your PC stops answering and when it is back."

    private const val ID_PC = 1
    private const val ID_SIGN_IN = 2

    private val OFFLINE = NoticeText(
        "PC went offline",
        "soos could not reach your PC in the last two checks.",
    )
    private val BACK_ONLINE = NoticeText(
        "PC back online",
        "soos can reach your PC again.",
    )
    private val SIGN_IN = NoticeText(
        "Sign-in needed",
        "Open soos and sign in with your passkey to see the PC status.",
    )

    fun of(notice: Notice): NoticeText = when (notice) {
        Notice.PC_OFFLINE -> OFFLINE
        Notice.PC_BACK_ONLINE -> BACK_ONLINE
        Notice.SIGN_IN_NEEDED -> SIGN_IN
    }

    /** The back-online notification replaces the offline one. */
    fun notificationId(notice: Notice): Int = when (notice) {
        Notice.PC_OFFLINE, Notice.PC_BACK_ONLINE -> ID_PC
        Notice.SIGN_IN_NEEDED -> ID_SIGN_IN
    }
}
