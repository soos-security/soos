//! Contract tests of GitHub #323 for the presence worker tick (matrix PAU4–PAU15, PAU19,
//! PAU27, PAU28), driven deterministically through `PresenceWorker::tick()` with a shifted
//! test clock, `MockPresenceLogind`, a settable display probe and scripted account guards.
//!
//! Power rule (spec §8): `unlock_calls == 0` is asserted after every failure path, not only
//! "no panic"; "no attempt" is asserted through the shared rate limiter, "no camera" through
//! the `notify_activity` counter and "no inference" through the extractor counter.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

mod common;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{
    all_logind_errors, bound_record, build_worker, fast_presence_config, locked_session, session,
    sid, Enrollment, PipelineOptions, ScriptedAccountGuard, ScriptedAnswer, StaticAccountGuard,
    WorkerFixture, MS_NS,
};
use soos_daemon::inference::InteractiveDemandGuard;
use soos_daemon::pipeline::current_monotonic_nanos;
use soos_daemon::presence::account::{
    AccountGuard, AccountRefusal, AccountState, SystemAccountGuard,
};
use soos_daemon::presence::display::DisplayState;
use soos_daemon::presence::logind::{LogindSessionState, PresenceLogindError};
use soos_daemon::presence::worker::{PresenceTick, ScanOutcome, SkipReason};
use soos_daemon::presence::{
    ACCOUNT_CHECK_TIMEOUT_MS, DBUS_CALL_TIMEOUT_MS, GLOBAL_DISABLE_FLAG, MAX_ALLOW_TO_UNLOCK_MS,
    MAX_PRESENCE_SEAT_SESSIONS, PRESENCE_DISABLE_FLAG, UNLOCK_CONFIRM_TIMEOUT_MS,
};
use soos_daemon::session_policy::SessionRecord;
use soos_daemon::DaemonError;
use soos_inference_ort::{AttackType, PadResult};
use soos_policy::ThresholdConfig;
use soos_protocol::types::ReasonClass;

const UID: u32 = 1000;

fn advance(offset: &AtomicU64, ms: u64) {
    offset.fetch_add(ms * MS_NS, Ordering::SeqCst);
}

fn unlocked(id: &str, uid: u32) -> PresenceTick {
    PresenceTick::Scanned(ScanOutcome::Unlocked {
        session: sid(id),
        uid,
    })
}

fn usable() -> ScriptedAccountGuard {
    ScriptedAccountGuard::always(AccountState::Usable)
}

/// A worker over one bound, locked session "2" of UID 1000 ("alice"), enrolled with the
/// mock-frame identity.
async fn eligible(clock: fn() -> Result<u64, DaemonError>) -> WorkerFixture<ScriptedAccountGuard> {
    eligible_with(PipelineOptions::new(clock), usable()).await
}

async fn eligible_with<A: AccountGuard>(options: PipelineOptions, guard: A) -> WorkerFixture<A> {
    let fx = build_worker(options, fast_presence_config(), guard).await;
    fx.logind
        .set_sessions(vec![locked_session("2", UID, "alice")]);
    fx
}

/// Ticks once inside the grace (asserting it), shifts the clock past the grace, ticks again.
async fn tick_past_grace<A: AccountGuard>(
    fx: &mut WorkerFixture<A>,
    offset: &AtomicU64,
) -> PresenceTick {
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::InGrace),
        "the first observation of a lock is inside the grace"
    );
    advance(offset, 1000);
    fx.worker.tick().await
}

/// Asserts the fail-closed "nothing happened" state: no unlock call, no camera wake, no
/// inference and no attempt recorded.
fn assert_nothing_spent<A: AccountGuard>(fx: &WorkerFixture<A>, label: &str) {
    assert_eq!(fx.logind.unlock_calls(), 0, "{label}: no unlock call");
    assert_eq!(
        fx.parts.camera.wakes(),
        0,
        "{label}: the camera is never woken"
    );
    assert_eq!(fx.parts.inferences(), 0, "{label}: no inference");
    assert_eq!(fx.parts.tracked_uids(), 0, "{label}: no attempt recorded");
}

// ---------------------------------------------------------------------------------------
// Nominal path
// ---------------------------------------------------------------------------------------

/// PAU10 / PAU11: an eligible session is unlocked after the grace, with one attempt, three
/// passing captures, a fresh logind re-check and a fresh account check.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_worker_unlocks_an_eligible_session_after_the_grace() {
    test_clock!(OFFSET, clock);
    let guard = usable();
    let mut fx = eligible_with(PipelineOptions::new(clock), guard.clone()).await;
    let tick = tick_past_grace(&mut fx, &OFFSET).await;
    assert_eq!(tick, unlocked("2", UID));
    assert_eq!(fx.logind.unlock_calls(), 1, "exactly one unlock per Allow");
    assert_eq!(fx.logind.unlocked_ids(), vec![sid("2")]);
    assert!(
        fx.logind.state_calls() >= 1,
        "a fresh re-check precedes the unlock"
    );
    assert_eq!(
        fx.logind.state_requests(),
        vec![sid("2")],
        "the re-check targets exactly the candidate session (T3)"
    );
    assert_eq!(
        fx.parts.inferences(),
        3,
        "k = 3 consecutive passing captures"
    );
    assert_eq!(fx.parts.remaining_attempts(UID), 39, "one attempt per scan");
    assert!(fx.parts.camera.wakes() >= 1);
    assert_eq!(
        guard.checked(),
        vec![("alice".to_string(), UID), ("alice".to_string(), UID)],
        "the account guard runs before the scan and again in the re-check"
    );
}

// ---------------------------------------------------------------------------------------
// PAU4 — kill switch
// ---------------------------------------------------------------------------------------

/// PAU4: either flag stops every side effect: no logind snapshot, no attempt, no camera,
/// no unlock.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_kill_switch_stops_every_side_effect() {
    for flag in [GLOBAL_DISABLE_FLAG, PRESENCE_DISABLE_FLAG] {
        test_clock!(OFFSET, clock);
        let mut fx = eligible(clock).await;
        std::fs::write(fx.switch_dir.join(flag), "").unwrap();
        for _ in 0..5 {
            assert_eq!(
                fx.worker.tick().await,
                PresenceTick::Skipped(SkipReason::KillSwitch),
                "{flag}"
            );
            advance(&OFFSET, 2000);
        }
        assert_eq!(fx.logind.seat_calls(), 0, "{flag}: no logind snapshot");
        assert_nothing_spent(&fx, flag);
    }
}

/// PAU4: removing the flag resumes at the next tick (no restart) with every grace
/// restarted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_kill_switch_removal_resumes_with_grace_restarted() {
    test_clock!(OFFSET, clock);
    let mut fx = eligible(clock).await;
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::InGrace)
    );
    let flag = fx.switch_dir.join(PRESENCE_DISABLE_FLAG);
    std::fs::write(&flag, "").unwrap();
    advance(&OFFSET, 5000);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::KillSwitch)
    );
    std::fs::remove_file(&flag).unwrap();
    // The session has been locked for > 5 s, but the grace restarts after re-enabling.
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::InGrace),
        "re-enabling restarts every grace"
    );
    assert_nothing_spent(&fx, "grace restarted");
    advance(&OFFSET, 1000);
    assert_eq!(fx.worker.tick().await, unlocked("2", UID));
}

// ---------------------------------------------------------------------------------------
// PAU5 — binding
// ---------------------------------------------------------------------------------------

/// PAU5: remote, unknown-remote, seatless, inactive, greeter/manager, foreign-UID-less and
/// unlocked sessions are never scanned and never unlocked.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_ineligible_sessions_are_never_scanned_or_unlocked() {
    let base = bound_record(UID);
    let with = |record: SessionRecord, locked: bool| LogindSessionState {
        id: sid("2"),
        record,
        locked,
        user_name: Some(common::user("alice")),
    };
    let cases: Vec<(&str, LogindSessionState)> = vec![
        (
            "remote",
            with(
                SessionRecord {
                    remote: Some(true),
                    ..base.clone()
                },
                true,
            ),
        ),
        (
            "unknown remote",
            with(
                SessionRecord {
                    remote: None,
                    ..base.clone()
                },
                true,
            ),
        ),
        (
            "seatless",
            with(
                SessionRecord {
                    seat: None,
                    ..base.clone()
                },
                true,
            ),
        ),
        (
            "inactive",
            with(
                SessionRecord {
                    active: false,
                    ..base.clone()
                },
                true,
            ),
        ),
        (
            "greeter",
            with(
                SessionRecord {
                    class: Some("greeter".into()),
                    ..base.clone()
                },
                true,
            ),
        ),
        (
            "manager",
            with(
                SessionRecord {
                    class: Some("manager".into()),
                    ..base.clone()
                },
                true,
            ),
        ),
        (
            "no uid",
            with(
                SessionRecord {
                    uid: None,
                    ..base.clone()
                },
                true,
            ),
        ),
        ("unlocked", with(base.clone(), false)),
    ];
    for (label, state) in cases {
        test_clock!(OFFSET, clock);
        let mut fx = build_worker(
            PipelineOptions::new(clock),
            fast_presence_config(),
            usable(),
        )
        .await;
        fx.logind.set_sessions(vec![state]);
        for _ in 0..4 {
            assert_eq!(
                fx.worker.tick().await,
                PresenceTick::Skipped(SkipReason::NoLockedSession),
                "{label}"
            );
            advance(&OFFSET, 5000);
        }
        assert_nothing_spent(&fx, label);
    }
}

// ---------------------------------------------------------------------------------------
// PAU6 — grace
// ---------------------------------------------------------------------------------------

/// PAU6: no scan before the grace elapses; scan once it has.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_worker_respects_the_lock_grace() {
    test_clock!(OFFSET, clock);
    let mut fx = eligible(clock).await;
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::InGrace)
    );
    advance(&OFFSET, 700);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::InGrace)
    );
    assert_nothing_spent(&fx, "in grace");
    advance(&OFFSET, 300);
    assert_eq!(fx.worker.tick().await, unlocked("2", UID));
}

/// PAU6: a session seen unlocked then locked again restarts its grace.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_worker_relock_restarts_the_grace() {
    test_clock!(OFFSET, clock);
    let mut fx = eligible(clock).await;
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::InGrace)
    );
    advance(&OFFSET, 900);
    fx.logind
        .set_sessions(vec![session("2", UID, "alice", false)]);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::NoLockedSession)
    );
    advance(&OFFSET, 200);
    fx.logind
        .set_sessions(vec![locked_session("2", UID, "alice")]);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::InGrace),
        "1.1 s after the first lock, but the new lock period restarted the grace"
    );
    assert_nothing_spent(&fx, "relocked");
}

/// Scan interval: after a scan that did not unlock, the next scan waits `scan_interval`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_worker_spaces_scans_by_the_scan_interval() {
    test_clock!(OFFSET, clock);
    let mut options = PipelineOptions::new(clock);
    options.enrolled = vec![(UID, Enrollment::Similarity(0.10))];
    let config = soos_daemon::presence::config::PresenceConfig {
        scan_interval: Duration::from_millis(5000),
        ..fast_presence_config()
    };
    let mut fx = build_worker(options, config, usable()).await;
    fx.logind
        .set_sessions(vec![locked_session("2", UID, "alice")]);
    assert_eq!(
        tick_past_grace(&mut fx, &OFFSET).await,
        PresenceTick::Scanned(ScanOutcome::NoMatch)
    );
    let wakes = fx.parts.camera.wakes();
    advance(&OFFSET, 1000);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::NotDue)
    );
    assert_eq!(
        fx.parts.camera.wakes(),
        wakes,
        "no scan before the interval"
    );
    advance(&OFFSET, 4000);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Scanned(ScanOutcome::NoMatch)
    );
    assert_eq!(fx.parts.remaining_attempts(UID), 38);
    assert_eq!(fx.logind.unlock_calls(), 0);
}

// ---------------------------------------------------------------------------------------
// PAU7 — exactly one candidate
// ---------------------------------------------------------------------------------------

/// PAU7: two eligible, enrolled sessions ⇒ no scan, no attempt, no unlock.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_two_eligible_sessions_are_ambiguous() {
    test_clock!(OFFSET, clock);
    let mut options = PipelineOptions::new(clock);
    options.enrolled = vec![
        (1000, Enrollment::LiveIdentity),
        (1001, Enrollment::LiveIdentity),
    ];
    let mut fx = build_worker(options, fast_presence_config(), usable()).await;
    let mut second = locked_session("3", 1001, "bob");
    second.record.seat = Some("seat1".into());
    fx.logind
        .set_sessions(vec![locked_session("2", 1000, "alice"), second]);
    assert_eq!(
        tick_past_grace(&mut fx, &OFFSET).await,
        PresenceTick::Skipped(SkipReason::AmbiguousCandidates)
    );
    for _ in 0..3 {
        advance(&OFFSET, 2000);
        assert_eq!(
            fx.worker.tick().await,
            PresenceTick::Skipped(SkipReason::AmbiguousCandidates)
        );
    }
    assert_nothing_spent(&fx, "ambiguous");
}

/// PAU7: one eligible enrolled session plus one not enrolled ⇒ the enrolled one is scanned.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_enrolled_session_is_scanned_next_to_a_not_enrolled_one() {
    test_clock!(OFFSET, clock);
    let mut fx = build_worker(
        PipelineOptions::new(clock),
        fast_presence_config(),
        usable(),
    )
    .await;
    let mut other = locked_session("3", 1001, "bob");
    other.record.seat = Some("seat1".into());
    fx.logind
        .set_sessions(vec![locked_session("2", UID, "alice"), other]);
    assert_eq!(tick_past_grace(&mut fx, &OFFSET).await, unlocked("2", UID));
    assert_eq!(fx.logind.unlocked_ids(), vec![sid("2")]);
    assert_eq!(
        fx.parts.remaining_attempts(1001),
        40,
        "UID 1001 spent nothing"
    );
}

// ---------------------------------------------------------------------------------------
// PAU8 — unusable templates
// ---------------------------------------------------------------------------------------

/// `(label, enrolled templates, expected skip reason)`.
type UnusableTemplateCase = (&'static str, Vec<(u32, Enrollment)>, SkipReason);

/// PAU8: not enrolled, foreign template and store error ⇒ no attempt, no camera, no
/// inference, no unlock.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_unusable_templates_cost_no_attempt_and_no_camera() {
    let cases: Vec<UnusableTemplateCase> = vec![
        ("not enrolled", vec![], SkipReason::NotEnrolled),
        (
            "foreign template",
            vec![(UID, Enrollment::RetiredArcFace)],
            SkipReason::ForeignTemplate,
        ),
        (
            "store error",
            vec![(UID, Enrollment::Corrupt)],
            SkipReason::TemplateStoreError,
        ),
    ];
    for (label, enrolled, reason) in cases {
        test_clock!(OFFSET, clock);
        let mut options = PipelineOptions::new(clock);
        options.enrolled = enrolled;
        let mut fx = eligible_with(options, usable()).await;
        let tick = tick_past_grace(&mut fx, &OFFSET).await;
        assert!(
            tick == PresenceTick::Skipped(reason)
                || tick == PresenceTick::Skipped(SkipReason::NoCandidate),
            "{label}: got {tick:?}"
        );
        for _ in 0..3 {
            advance(&OFFSET, 2000);
            let _ = fx.worker.tick().await;
        }
        assert_nothing_spent(&fx, label);
    }
}

// ---------------------------------------------------------------------------------------
// PAU9 — lid and screen gating
// ---------------------------------------------------------------------------------------

/// PAU9: a closed lid or a DPMS-off screen gates the scan (no attempt, no camera).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_closed_lid_or_blank_screen_gates_the_scan() {
    for (label, lid_closed, display, reason) in [
        ("lid closed", true, DisplayState::On, SkipReason::LidClosed),
        (
            "screen off",
            false,
            DisplayState::Off,
            SkipReason::DisplayOff,
        ),
    ] {
        test_clock!(OFFSET, clock);
        let mut fx = eligible(clock).await;
        fx.logind.set_lid(Ok(lid_closed));
        fx.display.set(display);
        assert_eq!(
            tick_past_grace(&mut fx, &OFFSET).await,
            PresenceTick::Skipped(reason),
            "{label}"
        );
        advance(&OFFSET, 2000);
        assert_eq!(fx.worker.tick().await, PresenceTick::Skipped(reason));
        assert_nothing_spent(&fx, label);
    }
}

/// PAU9: an undetectable screen (`Unknown`) or a lid read error never gates.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_undetectable_lid_or_screen_does_not_gate() {
    for (label, lid, display) in [
        ("display unknown", Ok(false), DisplayState::Unknown),
        (
            "lid error",
            Err(PresenceLogindError::Call(
                "org.freedesktop.DBus.Error.UnknownProperty".into(),
            )),
            DisplayState::On,
        ),
    ] {
        test_clock!(OFFSET, clock);
        let mut fx = eligible(clock).await;
        fx.logind.set_lid(lid);
        fx.display.set(display);
        assert_eq!(
            tick_past_grace(&mut fx, &OFFSET).await,
            unlocked("2", UID),
            "{label}"
        );
    }
}

// ---------------------------------------------------------------------------------------
// PAU10 — pipeline unchanged
// ---------------------------------------------------------------------------------------

/// PAU10: one spoof capture vetoes the scan; the attempt is spent, nothing is unlocked.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_spoof_capture_vetoes_the_presence_scan() {
    test_clock!(OFFSET, clock);
    let mut fx = eligible(clock).await;
    fx.parts.pad.set_result_sequence(vec![
        PadResult::live(0.98),
        PadResult::spoof(0.99, AttackType::PrintPhoto),
        PadResult::live(0.98),
        PadResult::live(0.98),
    ]);
    assert_eq!(
        tick_past_grace(&mut fx, &OFFSET).await,
        PresenceTick::Scanned(ScanOutcome::SpoofVetoed)
    );
    assert_eq!(fx.logind.unlock_calls(), 0);
    assert_eq!(fx.parts.remaining_attempts(UID), 39);
}

/// PAU10: `[pipeline.thresholds]` apply to presence: a raised `match_threshold` makes the
/// same mock score fail, while the default lets it unlock.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_policy_thresholds_apply_to_presence() {
    test_clock!(OFFSET_DEFAULT, clock_default);
    let mut options = PipelineOptions::new(clock_default);
    options.enrolled = vec![(UID, Enrollment::Similarity(0.60))];
    let mut fx = eligible_with(options, usable()).await;
    assert_eq!(
        tick_past_grace(&mut fx, &OFFSET_DEFAULT).await,
        unlocked("2", UID),
        "cosine 0.60 passes the default match threshold 0.50"
    );

    test_clock!(OFFSET_RAISED, clock_raised);
    let mut options = PipelineOptions::new(clock_raised);
    options.enrolled = vec![(UID, Enrollment::Similarity(0.60))];
    options.thresholds = ThresholdConfig::builder()
        .match_threshold(0.70)
        .build()
        .unwrap();
    let mut fx = eligible_with(options, usable()).await;
    assert_eq!(
        tick_past_grace(&mut fx, &OFFSET_RAISED).await,
        PresenceTick::Scanned(ScanOutcome::NoMatch),
        "a raised match_threshold must make the same score fail"
    );
    assert_eq!(fx.logind.unlock_calls(), 0);
}

// ---------------------------------------------------------------------------------------
// PAU11 — unlock only after Allow + fresh re-check
// ---------------------------------------------------------------------------------------

/// PAU11: the re-check refuses when the session is gone, unlocked, unbound or owned by
/// another UID.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_recheck_refusals_never_unlock() {
    let mut remote = locked_session("2", UID, "alice");
    remote.record.remote = Some(true);
    let mut inactive = locked_session("2", UID, "alice");
    inactive.record.active = false;
    let mut seatless = locked_session("2", UID, "alice");
    seatless.record.seat = None;
    let cases: Vec<(&str, Option<LogindSessionState>)> = vec![
        ("gone", None),
        ("unlocked", Some(session("2", UID, "alice", false))),
        ("other uid", Some(locked_session("2", 1001, "alice"))),
        ("remote", Some(remote)),
        ("inactive", Some(inactive)),
        ("seatless", Some(seatless)),
    ];
    for (label, recheck) in cases {
        test_clock!(OFFSET, clock);
        let mut fx = eligible(clock).await;
        fx.logind.queue_session_state(Ok(recheck));
        assert_eq!(
            tick_past_grace(&mut fx, &OFFSET).await,
            PresenceTick::Scanned(ScanOutcome::SessionChanged),
            "{label}"
        );
        assert_eq!(fx.logind.unlock_calls(), 0, "{label}: never unlocked");
    }
}

/// PAU11: every `PresenceLogindError` at the re-check ⇒ `SessionChanged`, no unlock.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_logind_error_at_recheck_never_unlocks() {
    for err in all_logind_errors() {
        test_clock!(OFFSET, clock);
        let mut fx = eligible(clock).await;
        fx.logind.queue_session_state(Err(err.clone()));
        assert_eq!(
            tick_past_grace(&mut fx, &OFFSET).await,
            PresenceTick::Scanned(ScanOutcome::SessionChanged),
            "{err:?}"
        );
        assert_eq!(fx.logind.unlock_calls(), 0, "{err:?}");
    }
}

/// PAU11: every `PresenceLogindError` at the snapshot ⇒ skipped, nothing spent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_logind_error_at_snapshot_never_unlocks() {
    for err in all_logind_errors() {
        test_clock!(OFFSET, clock);
        let mut fx = eligible(clock).await;
        fx.logind.set_sessions_error(err.clone());
        let expected = if err == PresenceLogindError::TooManySessions {
            SkipReason::TooManySessions
        } else {
            SkipReason::LogindUnavailable
        };
        assert_eq!(
            fx.worker.tick().await,
            PresenceTick::Skipped(expected),
            "{err:?}"
        );
        assert_nothing_spent(&fx, "snapshot error");
    }
}

/// PAU11: an `UnlockSession` error ⇒ `UnlockFailed` (the call was attempted once).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_unlock_call_error_is_reported_as_unlock_failed() {
    for err in all_logind_errors() {
        test_clock!(OFFSET, clock);
        let mut fx = eligible(clock).await;
        fx.logind.set_unlock_result(Err(err.clone()));
        assert_eq!(
            tick_past_grace(&mut fx, &OFFSET).await,
            PresenceTick::Scanned(ScanOutcome::UnlockFailed),
            "{err:?}"
        );
        assert_eq!(fx.logind.unlock_calls(), 1, "{err:?}");
    }
}

/// PAU11: more than `MAX_ALLOW_TO_UNLOCK_MS` between `Allow` and the unlock call discards
/// the `Allow`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_expired_allow_is_discarded() {
    test_clock!(OFFSET, clock);
    let mut fx = eligible(clock).await;
    fx.logind.on_session_state(|| {
        OFFSET.fetch_add((MAX_ALLOW_TO_UNLOCK_MS + 1) * MS_NS, Ordering::SeqCst);
    });
    assert_eq!(
        tick_past_grace(&mut fx, &OFFSET).await,
        PresenceTick::Scanned(ScanOutcome::AllowExpired)
    );
    assert_eq!(fx.logind.unlock_calls(), 0);
}

/// PAU11: a clock failure at the start of a tick skips it before any logind call.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_clock_failure_skips_the_tick() {
    let fx = eligible(common::failing_clock).await;
    let mut worker = fx.worker;
    for _ in 0..3 {
        assert_eq!(
            worker.tick().await,
            PresenceTick::Skipped(SkipReason::ClockUnavailable)
        );
    }
    assert_eq!(fx.logind.seat_calls(), 0);
    assert_eq!(fx.logind.unlock_calls(), 0);
    assert_eq!(fx.parts.camera.wakes(), 0);
}

static FLAKY_FAIL: AtomicBool = AtomicBool::new(false);
static FLAKY_OFFSET: AtomicU64 = AtomicU64::new(0);

fn flaky_clock() -> Result<u64, DaemonError> {
    if FLAKY_FAIL.load(Ordering::SeqCst) {
        return Err(DaemonError::Clock("injected".into()));
    }
    current_monotonic_nanos().map(|now| now + FLAKY_OFFSET.load(Ordering::SeqCst))
}

/// PAU11: a clock failure after the `Allow` (during the re-check) never unlocks.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_clock_failure_after_allow_never_unlocks() {
    let mut fx = eligible(flaky_clock).await;
    fx.logind
        .on_session_state(|| FLAKY_FAIL.store(true, Ordering::SeqCst));
    let tick = tick_past_grace(&mut fx, &FLAKY_OFFSET).await;
    assert!(
        matches!(tick, PresenceTick::Scanned(ref outcome) if !matches!(outcome, ScanOutcome::Unlocked { .. })),
        "a clock failure after the Allow must not unlock, got {tick:?}"
    );
    assert_eq!(fx.logind.unlock_calls(), 0);
}

/// PAU11: an inference job panic and `VisionError::Inference` abort the scan.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_inference_failures_never_unlock() {
    test_clock!(OFFSET_PANIC, clock_panic);
    let mut fx = eligible(clock_panic).await;
    fx.parts
        .set_extractor_hook(|| panic!("injected inference job panic"));
    let tick = tick_past_grace(&mut fx, &OFFSET_PANIC).await;
    assert!(
        matches!(tick, PresenceTick::Scanned(ScanOutcome::Aborted(_))),
        "an inference job panic must abort, got {tick:?}"
    );
    assert_eq!(fx.logind.unlock_calls(), 0);

    test_clock!(OFFSET_VISION, clock_vision);
    let mut fx = eligible(clock_vision).await;
    fx.parts.detector.set_fail_next(true);
    assert_eq!(
        tick_past_grace(&mut fx, &OFFSET_VISION).await,
        PresenceTick::Scanned(ScanOutcome::Aborted(ReasonClass::ModelUnavailable))
    );
    assert_eq!(fx.logind.unlock_calls(), 0);
}

/// PAU11: a camera that never becomes ready ⇒ `CameraUnavailable` (attempt spent).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_camera_not_ready_never_unlocks() {
    test_clock!(OFFSET, clock);
    let mut fx = eligible(clock).await;
    fx.parts.camera.never_ready.store(true, Ordering::SeqCst);
    assert_eq!(
        tick_past_grace(&mut fx, &OFFSET).await,
        PresenceTick::Scanned(ScanOutcome::CameraUnavailable)
    );
    assert_eq!(fx.logind.unlock_calls(), 0);
    assert_eq!(fx.parts.inferences(), 0);
    assert_eq!(fx.parts.remaining_attempts(UID), 39);
}

// ---------------------------------------------------------------------------------------
// PAU12 — PAM priority
// ---------------------------------------------------------------------------------------

/// PAU12: a live interactive demand skips the tick before any attempt or camera wake.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_interactive_demand_skips_the_presence_tick() {
    test_clock!(OFFSET, clock);
    let mut fx = eligible(clock).await;
    let demand = fx.gate.register_interactive();
    assert_eq!(
        tick_past_grace(&mut fx, &OFFSET).await,
        PresenceTick::Skipped(SkipReason::InteractiveDemand)
    );
    assert_nothing_spent(&fx, "interactive demand");
    drop(demand);
    assert_eq!(fx.worker.tick().await, unlocked("2", UID));
}

/// PAU12: an interactive request appearing during the scan preempts it (no unlock).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_interactive_request_preempts_a_running_scan() {
    test_clock!(OFFSET, clock);
    let mut fx = eligible(clock).await;
    let guards: Arc<Mutex<Vec<InteractiveDemandGuard>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let gate = fx.gate.clone();
        let guards = Arc::clone(&guards);
        fx.parts.set_extractor_hook(move || {
            let mut held = guards.lock().unwrap();
            if held.is_empty() {
                held.push(gate.register_interactive());
            }
        });
    }
    assert_eq!(
        tick_past_grace(&mut fx, &OFFSET).await,
        PresenceTick::Scanned(ScanOutcome::Preempted)
    );
    assert_eq!(fx.logind.unlock_calls(), 0);
    assert_eq!(fx.parts.inferences(), 1);
}

// ---------------------------------------------------------------------------------------
// PAU13 — camera standby
// ---------------------------------------------------------------------------------------

/// PAU13: with no tracked session, only in-grace, gated, not-enrolled or kill-switched
/// sessions, `notify_activity` is never called over 20 ticks.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_camera_stays_in_standby_without_an_eligible_scan() {
    for scenario in [
        "no session",
        "in grace",
        "lid closed",
        "not enrolled",
        "kill switch",
    ] {
        test_clock!(OFFSET, clock);
        let mut options = PipelineOptions::new(clock);
        let config = if scenario == "in grace" {
            soos_daemon::presence::config::PresenceConfig {
                lock_grace: Duration::from_millis(60_000),
                ..fast_presence_config()
            }
        } else {
            fast_presence_config()
        };
        if scenario == "not enrolled" {
            options.enrolled = vec![];
        }
        let mut fx = build_worker(options, config, usable()).await;
        if scenario != "no session" {
            fx.logind
                .set_sessions(vec![locked_session("2", UID, "alice")]);
        }
        if scenario == "lid closed" {
            fx.logind.set_lid(Ok(true));
        }
        if scenario == "kill switch" {
            std::fs::write(fx.switch_dir.join(PRESENCE_DISABLE_FLAG), "").unwrap();
        }
        for _ in 0..20 {
            let tick = fx.worker.tick().await;
            assert!(
                matches!(tick, PresenceTick::Skipped(_)),
                "{scenario}: no scan expected, got {tick:?}"
            );
            advance(&OFFSET, 1000);
        }
        assert_eq!(
            fx.parts.camera.wakes(),
            0,
            "{scenario}: notify_activity must never be called"
        );
        assert_nothing_spent(&fx, scenario);
    }
}

// ---------------------------------------------------------------------------------------
// PAU14 — bounds, timeouts, backoff
// ---------------------------------------------------------------------------------------

/// PAU14: more than `MAX_PRESENCE_SEAT_SESSIONS` seat sessions ⇒ `TooManySessions`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_too_many_seat_sessions_skip_the_tick() {
    test_clock!(OFFSET, clock);
    let mut fx = eligible(clock).await;
    let many: Vec<LogindSessionState> = (0..=MAX_PRESENCE_SEAT_SESSIONS)
        .map(|i| locked_session(&format!("{}", 10 + i), UID, "alice"))
        .collect();
    fx.logind.set_sessions(many);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::TooManySessions)
    );
    advance(&OFFSET, 60_000);
    let _ = fx.worker.tick().await;
    assert_nothing_spent(&fx, "too many sessions");
}

async fn bounded_tick<A: AccountGuard>(fx: &mut WorkerFixture<A>) -> (PresenceTick, Duration) {
    let started = Instant::now();
    let tick = tokio::time::timeout(Duration::from_secs(10), fx.worker.tick())
        .await
        .expect("a hung logind call must never hang the tick");
    (tick, started.elapsed())
}

/// PAU14: a never-resolving snapshot call ends the tick within `DBUS_CALL_TIMEOUT_MS`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_hung_snapshot_call_is_bounded() {
    test_clock!(OFFSET, clock);
    let mut fx = eligible(clock).await;
    fx.logind.0.hang_seat.store(true, Ordering::SeqCst);
    let (tick, elapsed) = bounded_tick(&mut fx).await;
    assert_eq!(tick, PresenceTick::Skipped(SkipReason::LogindUnavailable));
    assert!(
        elapsed < Duration::from_millis(DBUS_CALL_TIMEOUT_MS + 700),
        "the snapshot call must be bounded by DBUS_CALL_TIMEOUT_MS, took {elapsed:?}"
    );
    assert_nothing_spent(&fx, "hung snapshot");
}

/// PAU14: a never-resolving re-check or unlock call ends the scan without an unlock; a
/// never-resolving lid call only affects gating.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_hung_scan_calls_are_bounded() {
    for (label, expected) in [
        (
            "session_state",
            PresenceTick::Scanned(ScanOutcome::SessionChanged),
        ),
        (
            "unlock_session",
            PresenceTick::Scanned(ScanOutcome::UnlockFailed),
        ),
        ("lid_closed", unlocked("2", UID)),
    ] {
        test_clock!(OFFSET, clock);
        let mut fx = eligible(clock).await;
        assert_eq!(
            fx.worker.tick().await,
            PresenceTick::Skipped(SkipReason::InGrace)
        );
        advance(&OFFSET, 1000);
        let flag = match label {
            "session_state" => &fx.logind.0.hang_state,
            "unlock_session" => &fx.logind.0.hang_unlock,
            _ => &fx.logind.0.hang_lid,
        };
        flag.store(true, Ordering::SeqCst);
        let hung_at: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
        {
            let slot = Arc::clone(&hung_at);
            let mark = move || *slot.lock().unwrap() = Some(Instant::now());
            match label {
                "session_state" => fx.logind.on_session_state(mark),
                "unlock_session" => fx.logind.on_unlock(mark),
                _ => {}
            }
        }
        let (tick, _) = bounded_tick(&mut fx).await;
        assert_eq!(tick, expected, "hung {label}");
        if let Some(started) = *hung_at.lock().unwrap() {
            let waited = started.elapsed();
            assert!(
                waited < Duration::from_millis(DBUS_CALL_TIMEOUT_MS + 700),
                "hung {label} must be cut at DBUS_CALL_TIMEOUT_MS, waited {waited:?}"
            );
        } else {
            assert_eq!(label, "lid_closed", "the hung {label} call was never made");
        }
        if label == "session_state" {
            assert_eq!(fx.logind.unlock_calls(), 0, "hung re-check never unlocks");
        }
        if label == "unlock_session" {
            assert_eq!(
                fx.logind.unlocked_ids(),
                vec![sid("2")],
                "exactly one unlock attempt, for the candidate, and no retry"
            );
        }
    }
}

/// PAU14: after a logind failure the worker backs off (1 s, then 2 s), and a success
/// resets the backoff. Time is shifted on both clocks (test offset and real sleep) so the
/// assertion holds whichever clock measures the backoff window.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_logind_failure_backoff_doubles_and_resets() {
    test_clock!(OFFSET, clock);
    let mut fx = eligible(clock).await;
    async fn wait(offset: &AtomicU64, ms: u64) {
        advance(offset, ms);
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }
    fx.logind
        .set_sessions_error(PresenceLogindError::BusUnavailable);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::LogindUnavailable)
    );
    assert_eq!(fx.logind.seat_calls(), 1);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::Backoff)
    );
    assert_eq!(
        fx.logind.seat_calls(),
        1,
        "no call inside the backoff window"
    );

    // First backoff: 1 s.
    wait(&OFFSET, 1100).await;
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::LogindUnavailable)
    );
    assert_eq!(fx.logind.seat_calls(), 2);

    // Second backoff: 2 s (still backing off after 1.1 s).
    wait(&OFFSET, 1100).await;
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::Backoff),
        "the backoff doubled to 2 s"
    );
    assert_eq!(fx.logind.seat_calls(), 2);
    wait(&OFFSET, 1000).await;
    fx.logind.set_sessions(vec![]);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::NoLockedSession)
    );
    assert_eq!(fx.logind.seat_calls(), 3);

    // Success reset the backoff: the next failure backs off 1 s again.
    fx.logind.set_sessions_error(PresenceLogindError::Timeout);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::LogindUnavailable)
    );
    wait(&OFFSET, 1100).await;
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::LogindUnavailable),
        "after a success the backoff restarts at 1 s"
    );
    assert_eq!(fx.logind.seat_calls(), 5);
    assert_eq!(fx.logind.unlock_calls(), 0);
}

// ---------------------------------------------------------------------------------------
// PAU15 — locker ignoring the unlock
// ---------------------------------------------------------------------------------------

/// PAU15: a session still locked 5 s after a successful unlock call is not scanned again
/// until it is observed unlocked and re-locked.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_ignored_unlock_is_not_retried_until_the_next_lock_period() {
    test_clock!(OFFSET, clock);
    let mut fx = eligible(clock).await;
    assert_eq!(tick_past_grace(&mut fx, &OFFSET).await, unlocked("2", UID));
    // The locker ignores UnlockSession: the snapshot keeps reporting the session locked.
    advance(&OFFSET, UNLOCK_CONFIRM_TIMEOUT_MS + 100);
    for _ in 0..10 {
        let tick = fx.worker.tick().await;
        assert!(
            matches!(tick, PresenceTick::Skipped(_)),
            "a locker_ignored session is never scanned, got {tick:?}"
        );
        advance(&OFFSET, 2000);
    }
    assert_eq!(fx.logind.unlock_calls(), 1);

    fx.logind
        .set_sessions(vec![session("2", UID, "alice", false)]);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::NoLockedSession)
    );
    fx.logind
        .set_sessions(vec![locked_session("2", UID, "alice")]);
    assert_eq!(tick_past_grace(&mut fx, &OFFSET).await, unlocked("2", UID));
    assert_eq!(fx.logind.unlock_calls(), 2);
}

// ---------------------------------------------------------------------------------------
// Rate limit (D10)
// ---------------------------------------------------------------------------------------

/// D10 / PAU3: presence never consumes the last 5 attempts of the shared window.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_presence_keeps_the_pam_reserve() {
    test_clock!(OFFSET, clock);
    let mut options = PipelineOptions::new(clock);
    options.max_attempts = 6;
    options.enrolled = vec![(UID, Enrollment::Similarity(0.10))];
    let mut fx = eligible_with(options, usable()).await;
    assert_eq!(
        tick_past_grace(&mut fx, &OFFSET).await,
        PresenceTick::Scanned(ScanOutcome::NoMatch)
    );
    assert_eq!(fx.parts.remaining_attempts(UID), 5);
    let wakes = fx.parts.camera.wakes();
    advance(&OFFSET, 1000);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::RateLimited)
    );
    assert_eq!(
        fx.parts.remaining_attempts(UID),
        5,
        "the reserve is untouched"
    );
    assert_eq!(
        fx.parts.camera.wakes(),
        wakes,
        "a rate-limited tick never wakes the camera"
    );
    assert_eq!(fx.logind.unlock_calls(), 0);
}

/// D10: with `max_attempts <= 5` presence never scans and records nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_presence_never_scans_when_the_budget_is_the_reserve() {
    test_clock!(OFFSET, clock);
    let mut options = PipelineOptions::new(clock);
    options.max_attempts = 5;
    let mut fx = eligible_with(options, usable()).await;
    assert_eq!(
        tick_past_grace(&mut fx, &OFFSET).await,
        PresenceTick::Skipped(SkipReason::RateLimited)
    );
    assert_nothing_spent(&fx, "budget equal to the reserve");
}

// ---------------------------------------------------------------------------------------
// PAU27 / PAU28 — account guard in the worker
// ---------------------------------------------------------------------------------------

/// PAU28: a refused account at step 7 costs no attempt, no camera and no inference.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_refused_account_before_the_scan_costs_nothing() {
    for refusal in [
        AccountRefusal::Faillocked,
        AccountRefusal::AccountExpired,
        AccountRefusal::AccountInactive,
        AccountRefusal::PasswordExpired,
        AccountRefusal::PasswordChangeForced,
        AccountRefusal::PasswordLocked,
        AccountRefusal::RootAccount,
        AccountRefusal::Undeterminable,
    ] {
        test_clock!(OFFSET, clock);
        let mut fx = eligible_with(
            PipelineOptions::new(clock),
            StaticAccountGuard(AccountState::Refused(refusal)),
        )
        .await;
        let tick = tick_past_grace(&mut fx, &OFFSET).await;
        assert!(
            tick == PresenceTick::Skipped(SkipReason::AccountRefused)
                || tick == PresenceTick::Skipped(SkipReason::NoCandidate),
            "{refusal:?}: got {tick:?}"
        );
        assert_nothing_spent(&fx, "refused account");
    }
}

/// PAU28: the guard is consulted again after the `Allow`, with no cached result.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_account_is_rechecked_after_the_allow() {
    test_clock!(OFFSET, clock);
    let guard = ScriptedAccountGuard::script(
        vec![ScriptedAnswer {
            delay: Duration::ZERO,
            state: AccountState::Usable,
        }],
        AccountState::Refused(AccountRefusal::Faillocked),
    );
    let mut fx = eligible_with(PipelineOptions::new(clock), guard.clone()).await;
    assert_eq!(
        tick_past_grace(&mut fx, &OFFSET).await,
        PresenceTick::Scanned(ScanOutcome::AccountRefused(AccountRefusal::Faillocked))
    );
    assert_eq!(fx.logind.unlock_calls(), 0);
    assert_eq!(guard.calls(), 2, "step 7 and the fresh step-13 re-check");
}

/// PAU28: a guard blocking longer than `ACCOUNT_CHECK_TIMEOUT_MS` is `Undeterminable`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_slow_account_guard_is_undeterminable() {
    let slow = Duration::from_millis(ACCOUNT_CHECK_TIMEOUT_MS + 1000);
    // Step 7: the candidate is dropped.
    test_clock!(OFFSET_PRE, clock_pre);
    let guard = ScriptedAccountGuard::script(
        vec![ScriptedAnswer {
            delay: slow,
            state: AccountState::Usable,
        }],
        AccountState::Usable,
    );
    let mut fx = eligible_with(PipelineOptions::new(clock_pre), guard).await;
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::InGrace)
    );
    advance(&OFFSET_PRE, 1000);
    let started = Instant::now();
    let tick = fx.worker.tick().await;
    assert!(
        tick == PresenceTick::Skipped(SkipReason::AccountRefused)
            || tick == PresenceTick::Skipped(SkipReason::NoCandidate),
        "a guard timeout before the scan drops the candidate, got {tick:?}"
    );
    assert!(
        started.elapsed() < slow,
        "the worker must not wait for a guard past ACCOUNT_CHECK_TIMEOUT_MS"
    );
    assert_nothing_spent(&fx, "slow guard before the scan");

    // Step 13: the re-check times out ⇒ AccountRefused(Undeterminable).
    test_clock!(OFFSET_POST, clock_post);
    let guard = ScriptedAccountGuard::script(
        vec![
            ScriptedAnswer {
                delay: Duration::ZERO,
                state: AccountState::Usable,
            },
            ScriptedAnswer {
                delay: slow,
                state: AccountState::Usable,
            },
        ],
        AccountState::Usable,
    );
    let mut fx = eligible_with(PipelineOptions::new(clock_post), guard).await;
    assert_eq!(
        tick_past_grace(&mut fx, &OFFSET_POST).await,
        PresenceTick::Scanned(ScanOutcome::AccountRefused(AccountRefusal::Undeterminable))
    );
    assert_eq!(fx.logind.unlock_calls(), 0);
}

/// PAU28: a `Name` that changed between step 7 and the re-check ⇒ `SessionChanged`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_changed_owner_name_at_recheck_never_unlocks() {
    test_clock!(OFFSET, clock);
    let mut fx = eligible(clock).await;
    fx.logind
        .queue_session_state(Ok(Some(locked_session("2", UID, "mallory"))));
    assert_eq!(
        tick_past_grace(&mut fx, &OFFSET).await,
        PresenceTick::Scanned(ScanOutcome::SessionChanged)
    );
    assert_eq!(fx.logind.unlock_calls(), 0);

    test_clock!(OFFSET_NONE, clock_none);
    let mut fx = eligible(clock_none).await;
    let mut nameless = locked_session("2", UID, "alice");
    nameless.user_name = None;
    fx.logind.queue_session_state(Ok(Some(nameless)));
    assert_eq!(
        tick_past_grace(&mut fx, &OFFSET_NONE).await,
        PresenceTick::Scanned(ScanOutcome::SessionChanged),
        "a Name that disappeared at the re-check is a changed session"
    );
    assert_eq!(fx.logind.unlock_calls(), 0);
}

/// PAU27: a session whose `Name` is missing is never scanned or unlocked.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_session_without_owner_name_is_never_unlocked() {
    test_clock!(OFFSET, clock);
    let mut fx = build_worker(
        PipelineOptions::new(clock),
        fast_presence_config(),
        usable(),
    )
    .await;
    let mut nameless = locked_session("2", UID, "alice");
    nameless.user_name = None;
    fx.logind.set_sessions(vec![nameless]);
    let _ = fx.worker.tick().await;
    for _ in 0..3 {
        advance(&OFFSET, 2000);
        let tick = fx.worker.tick().await;
        assert!(
            matches!(tick, PresenceTick::Skipped(_)),
            "a nameless session is never scanned, got {tick:?}"
        );
    }
    assert_nothing_spent(&fx, "nameless session");
}

/// PAU27: a UID 0 session is never scanned or unlocked (production guard: `RootAccount`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_root_session_is_never_scanned_or_unlocked() {
    test_clock!(OFFSET, clock);
    let accounts = tempfile::tempdir().unwrap();
    let tally_dir = accounts.path().join("faillock");
    std::fs::create_dir_all(&tally_dir).unwrap();
    let conf = accounts.path().join("faillock.conf");
    std::fs::write(&conf, format!("dir = {}\n", tally_dir.display())).unwrap();
    let pam = accounts.path().join("pam.d");
    std::fs::create_dir_all(&pam).unwrap();
    let shadow = accounts.path().join("shadow");
    std::fs::write(
        &shadow,
        "root:$6$hash:19000:0:99999:7:::\nalice:$6$hash:19000:0:99999:7:::\n",
    )
    .unwrap();
    fn realtime() -> Result<u64, DaemonError> {
        Ok(19_500 * 86_400)
    }
    let guard = SystemAccountGuard::new()
        .with_faillock_conf(conf)
        .with_vendor_faillock_conf(accounts.path().join("vendor-faillock.conf"))
        .with_default_faillock_dir(tally_dir)
        .with_pam_dirs(vec![pam])
        .with_shadow(shadow)
        .with_realtime_fn(realtime);
    let mut options = PipelineOptions::new(clock);
    options.enrolled = vec![(0, Enrollment::LiveIdentity)];
    let mut fx = build_worker(options, fast_presence_config(), guard).await;
    fx.logind.set_sessions(vec![locked_session("2", 0, "root")]);
    let _ = fx.worker.tick().await;
    for _ in 0..3 {
        advance(&OFFSET, 2000);
        let tick = fx.worker.tick().await;
        assert!(
            matches!(tick, PresenceTick::Skipped(_)),
            "a root session is never scanned, got {tick:?}"
        );
    }
    assert_nothing_spent(&fx, "root session");
}

// ---------------------------------------------------------------------------------------
// PAU19 — run loop
// ---------------------------------------------------------------------------------------

/// PAU19: `run` stops promptly on the shutdown signal and never unlocks afterwards.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_run_stops_on_shutdown_and_never_unlocks_after_stop() {
    test_clock!(OFFSET, clock);
    let fx = eligible(clock).await;
    let WorkerFixture {
        worker,
        logind,
        parts,
        ..
    } = fx;
    let (stop, rx) = tokio::sync::watch::channel(false);
    let handle = tokio::spawn(worker.run(rx));
    tokio::time::sleep(Duration::from_millis(100)).await;
    stop.send(true).unwrap();
    // The session would now be past its grace: no unlock may happen after the stop.
    advance(&OFFSET, 10_000);
    let joined = tokio::time::timeout(Duration::from_millis(2500), handle).await;
    assert!(
        joined.is_ok(),
        "run must return within the drain budget after the stop"
    );
    joined.unwrap().unwrap();
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(logind.unlock_calls(), 0, "no unlock after the stop signal");
    assert_eq!(parts.camera.wakes(), 0);
}

// ---------------------------------------------------------------------------------------
// Round 2 (auditor T3, T4, T8)
// ---------------------------------------------------------------------------------------

/// T3: a re-check reply describing another session ID than the candidate never unlocks,
/// and the re-check asks logind for the candidate's ID only.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_recheck_is_bound_to_the_candidate_session_id() {
    test_clock!(OFFSET, clock);
    let mut fx = eligible(clock).await;
    fx.logind
        .queue_session_state(Ok(Some(locked_session("3", UID, "alice"))));
    assert_eq!(
        tick_past_grace(&mut fx, &OFFSET).await,
        PresenceTick::Scanned(ScanOutcome::SessionChanged),
        "a reply for session 3 cannot confirm candidate 2"
    );
    assert_eq!(fx.logind.unlock_calls(), 0);
    assert!(fx.logind.unlocked_ids().is_empty());
    assert_eq!(fx.logind.state_requests(), vec![sid("2")]);
}

/// T4: the logind state changes while the scan runs (candidate replaced by another
/// session, owner UID or name changed, unlocked, turned remote) ⇒ no unlock at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_session_swap_during_the_scan_never_unlocks() {
    let mut remote = locked_session("2", UID, "alice");
    remote.record.remote = Some(true);
    let swaps: Vec<(&str, Vec<LogindSessionState>)> = vec![
        (
            "candidate replaced by session 5",
            vec![locked_session("5", UID, "alice")],
        ),
        (
            "owner uid changed",
            vec![locked_session("2", 1001, "alice")],
        ),
        (
            "owner name changed",
            vec![locked_session("2", UID, "mallory")],
        ),
        (
            "unlocked by the user",
            vec![session("2", UID, "alice", false)],
        ),
        ("turned remote", vec![remote]),
        ("all sessions gone", vec![]),
    ];
    for (label, swapped) in swaps {
        test_clock!(OFFSET, clock);
        let mut fx = eligible(clock).await;
        {
            let logind = fx.logind.clone();
            let swapped = swapped.clone();
            fx.parts
                .set_extractor_hook(move || logind.set_sessions(swapped.clone()));
        }
        assert_eq!(
            tick_past_grace(&mut fx, &OFFSET).await,
            PresenceTick::Scanned(ScanOutcome::SessionChanged),
            "{label}"
        );
        assert_eq!(fx.logind.unlock_calls(), 0, "{label}: no unlock");
        let ids = fx.logind.unlocked_ids();
        assert!(
            !ids.contains(&sid("2")) && !ids.contains(&sid("5")),
            "{label}: neither the candidate nor the newcomer is unlocked"
        );
    }
}

/// T8: an interactive request registered at the camera wake preempts the scan before
/// any inference.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_interactive_demand_at_camera_wake_preempts_before_inference() {
    test_clock!(OFFSET, clock);
    let mut fx = eligible(clock).await;
    let guards: Arc<Mutex<Vec<InteractiveDemandGuard>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let gate = fx.gate.clone();
        let guards = Arc::clone(&guards);
        *fx.parts.camera.on_wake.lock().unwrap() = Some(Box::new(move || {
            guards.lock().unwrap().push(gate.register_interactive());
        }));
    }
    assert_eq!(
        tick_past_grace(&mut fx, &OFFSET).await,
        PresenceTick::Scanned(ScanOutcome::Preempted)
    );
    assert_eq!(
        fx.parts.inferences(),
        0,
        "no inference after the demand appeared"
    );
    assert_eq!(fx.logind.unlock_calls(), 0);
}

// ---------------------------------------------------------------------------------------
// Round 3 (auditor T9, spec rev 4): kill switch re-checked right before UnlockSession
// ---------------------------------------------------------------------------------------

/// T9 / PAU28 (rev 4): a kill switch engaged after the consensus `Allow` (here during the
/// step-13 re-check) stops the unlock: `KillSwitchEngaged`, no `UnlockSession`, and no retry
/// while the switch stays engaged; once removed, the grace restarts (tracker cleared).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_kill_switch_engaged_before_unlock_never_unlocks() {
    for variant in [GLOBAL_DISABLE_FLAG, PRESENCE_DISABLE_FLAG, "stat-error"] {
        test_clock!(OFFSET, clock);
        let mut fx = eligible(clock).await;
        let switch_dir = fx.switch_dir.clone();
        {
            let switch_dir = switch_dir.clone();
            fx.logind.on_session_state(move || {
                if variant == "stat-error" {
                    // The flag directory turns into a regular file: ENOTDIR on every stat.
                    let _ = std::fs::remove_dir_all(&switch_dir);
                    std::fs::write(&switch_dir, "").unwrap();
                } else {
                    std::fs::write(switch_dir.join(variant), "").unwrap();
                }
            });
        }
        assert_eq!(
            tick_past_grace(&mut fx, &OFFSET).await,
            PresenceTick::Scanned(ScanOutcome::KillSwitchEngaged),
            "{variant}"
        );
        assert_eq!(fx.logind.unlock_calls(), 0, "{variant}: no UnlockSession");
        assert!(fx.logind.unlocked_ids().is_empty(), "{variant}");
        assert_eq!(fx.logind.state_calls(), 1, "{variant}: one re-check only");

        let wakes = fx.parts.camera.wakes();
        let seat_calls = fx.logind.seat_calls();
        for _ in 0..3 {
            advance(&OFFSET, 2000);
            assert_eq!(
                fx.worker.tick().await,
                PresenceTick::Skipped(SkipReason::KillSwitch),
                "{variant}: no retry while the switch stays engaged"
            );
        }
        assert_eq!(fx.logind.unlock_calls(), 0, "{variant}");
        assert_eq!(fx.parts.camera.wakes(), wakes, "{variant}: no new scan");
        assert_eq!(
            fx.logind.seat_calls(),
            seat_calls,
            "{variant}: no logind call"
        );

        if variant == "stat-error" {
            std::fs::remove_file(&switch_dir).unwrap();
            std::fs::create_dir_all(&switch_dir).unwrap();
        } else {
            std::fs::remove_file(switch_dir.join(variant)).unwrap();
        }
        assert_eq!(
            fx.worker.tick().await,
            PresenceTick::Skipped(SkipReason::InGrace),
            "{variant}: the tracker was cleared, so the grace restarts"
        );
        assert_eq!(fx.logind.unlock_calls(), 0, "{variant}");
    }
}
