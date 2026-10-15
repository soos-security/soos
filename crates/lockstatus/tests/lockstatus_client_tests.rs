//! Contract tests of the bounded `LockState` client (GitHub #401; matrix LKA1).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

mod common;

use std::time::{Duration, Instant};

use common::{my_uid, Behavior, MockDaemon};
use soos_lockstatus::client::{fetch_view, monotonic_now_ms, query_lock_state, LockStatusError};
use soos_lockstatus::view::{LockView, ViewState};
use soos_lockstatus::{CLIENT_SERVICE, EXCHANGE_BUDGET_MS};
use soos_protocol::{LockWidgetState, ReasonClass, RequestKind, Verdict, CURRENT_VERSION};

const BUDGET: Duration = Duration::from_millis(EXCHANGE_BUDGET_MS);
/// CI slack on top of the product budget (runners are slow); never tighter than the budget.
const SLACK: Duration = Duration::from_millis(700);

#[test]
fn test_lka1_budget_constant_is_250_ms() {
    assert_eq!(EXCHANGE_BUDGET_MS, 250);
    assert_eq!(CLIENT_SERVICE, "soos-lockstatus");
}

#[test]
fn test_lka1_request_fields_and_one_request_per_connection() {
    let daemon = MockDaemon::start(Behavior::Answer(LockWidgetState::Scanning, 5));
    let before = monotonic_now_ms().unwrap();
    let response = query_lock_state(&daemon.path, my_uid(), BUDGET).unwrap();
    assert_eq!(response.state, LockWidgetState::Scanning);
    assert_eq!(response.sequence, 5);
    assert_eq!(daemon_wait(&daemon, 1), 1);
    let seen = daemon.seen();
    assert_eq!(seen.len(), 1);
    let req = &seen[0];
    assert_eq!(req.version, CURRENT_VERSION);
    assert_eq!(req.kind, RequestKind::LockState);
    assert_eq!(req.uid_hint, my_uid());
    assert_eq!(req.service, CLIENT_SERVICE);
    assert!(req.tagged, "the request must carry the client message tag");
    let deadline_ms = req.deadline_monotonic_ns / 1_000_000;
    assert!(
        deadline_ms >= before && deadline_ms <= before + EXCHANGE_BUDGET_MS + 5_000,
        "deadline {deadline_ms} not derived from CLOCK_MONOTONIC + budget (before {before})"
    );
    // Give the mock the time to detect a second frame on the same connection.
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(
        daemon
            .stats
            .second_requests
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert_eq!(daemon.connections(), 1);
}

fn daemon_wait(daemon: &MockDaemon, requests: usize) -> usize {
    let until = Instant::now() + Duration::from_secs(5);
    while daemon.requests() < requests && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(2));
    }
    daemon.requests()
}

#[test]
fn test_lka1_each_state_is_fetched() {
    for (state, expected) in [
        (LockWidgetState::Idle, ViewState::Idle),
        (LockWidgetState::Scanning, ViewState::Scanning),
        (LockWidgetState::NotRecognized, ViewState::NotRecognized),
    ] {
        let daemon = MockDaemon::start(Behavior::Answer(state, 11));
        assert_eq!(
            fetch_view(&daemon.path, my_uid(), BUDGET),
            LockView {
                state: expected,
                sequence: 11
            }
        );
    }
}

#[test]
fn test_lka1_refusal_of_any_verdict_is_unavailable() {
    for (verdict, reason) in [
        (Verdict::ProtocolError, ReasonClass::UidMismatch),
        (Verdict::ProtocolError, ReasonClass::RateLimited),
        (Verdict::Deny, ReasonClass::PadFailed),
        (Verdict::Unavailable, ReasonClass::InternalError),
        // Even an `Allow` answer to a lock-state request is a refusal, never a success.
        (Verdict::Allow, ReasonClass::FaceMatch),
    ] {
        let daemon = MockDaemon::start(Behavior::Refuse(verdict, reason));
        assert!(matches!(
            query_lock_state(&daemon.path, my_uid(), BUDGET),
            Err(LockStatusError::Refused)
        ));
        assert_eq!(
            fetch_view(&daemon.path, my_uid(), BUDGET),
            LockView::UNAVAILABLE
        );
    }
}

#[test]
fn test_lka1_other_version_is_refused() {
    let daemon = MockDaemon::start(Behavior::WrongVersion);
    assert!(matches!(
        query_lock_state(&daemon.path, my_uid(), BUDGET),
        Err(LockStatusError::Version(v)) if v == CURRENT_VERSION + 1
    ));
    assert_eq!(
        fetch_view(&daemon.path, my_uid(), BUDGET),
        LockView::UNAVAILABLE
    );
}

#[test]
fn test_lka1_zero_and_oversize_lengths_are_refused_before_allocation() {
    for declared in [0u32, 4097, u32::MAX] {
        let daemon = MockDaemon::start(Behavior::Raw(declared.to_be_bytes().to_vec()));
        let start = Instant::now();
        let result = query_lock_state(&daemon.path, my_uid(), BUDGET);
        assert!(
            matches!(result, Err(LockStatusError::FrameLength(n)) if n == declared as usize),
            "{declared}: {result:?}"
        );
        // Refused from the prefix alone: no wait for a body that never comes.
        assert!(start.elapsed() < BUDGET, "{declared}");
    }
}

#[test]
fn test_lka1_garbage_and_truncated_bodies_are_unavailable() {
    let mut truncated = 12u32.to_be_bytes().to_vec();
    truncated.extend_from_slice(&[1, 1]);
    for raw in [
        {
            let mut v = 3u32.to_be_bytes().to_vec();
            v.extend_from_slice(&[0xFF, 0xFF, 0xFF]);
            v
        },
        {
            // A lock state with an unknown state byte (3).
            let mut v = 3u32.to_be_bytes().to_vec();
            v.extend_from_slice(&[CURRENT_VERSION, 3, 1]);
            v
        },
        truncated,
    ] {
        let daemon = MockDaemon::start(Behavior::Raw(raw.clone()));
        let result = query_lock_state(&daemon.path, my_uid(), BUDGET);
        assert!(result.is_err(), "{raw:?}: {result:?}");
        assert_eq!(
            fetch_view(&daemon.path, my_uid(), BUDGET),
            LockView::UNAVAILABLE
        );
    }
}

#[test]
fn test_lka1_old_daemon_closing_is_unavailable() {
    let daemon = MockDaemon::start(Behavior::Close);
    assert!(matches!(
        query_lock_state(&daemon.path, my_uid(), BUDGET),
        Err(LockStatusError::Io(_))
    ));
    assert_eq!(
        fetch_view(&daemon.path, my_uid(), BUDGET),
        LockView::UNAVAILABLE
    );
}

#[test]
fn test_lka1_stalled_daemon_times_out_within_the_budget() {
    let daemon = MockDaemon::start(Behavior::Stall(Duration::from_secs(5)));
    let start = Instant::now();
    let result = query_lock_state(&daemon.path, my_uid(), BUDGET);
    let elapsed = start.elapsed();
    assert!(
        matches!(result, Err(LockStatusError::Timeout)),
        "{result:?}"
    );
    assert!(elapsed >= BUDGET - Duration::from_millis(20), "{elapsed:?}");
    assert!(elapsed < BUDGET + SLACK, "{elapsed:?}");
}

#[test]
fn test_lka1_late_answer_is_a_timeout() {
    let daemon = MockDaemon::start(Behavior::DelayedAnswer(
        Duration::from_millis(600),
        LockWidgetState::Scanning,
        1,
    ));
    let start = Instant::now();
    assert!(matches!(
        query_lock_state(&daemon.path, my_uid(), BUDGET),
        Err(LockStatusError::Timeout)
    ));
    assert!(start.elapsed() < BUDGET + SLACK);
}

#[test]
fn test_lka1_missing_socket_is_unavailable_quickly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("absent.sock");
    let start = Instant::now();
    assert!(matches!(
        query_lock_state(&path, my_uid(), BUDGET),
        Err(LockStatusError::Connect(_))
    ));
    assert_eq!(fetch_view(&path, my_uid(), BUDGET), LockView::UNAVAILABLE);
    assert!(start.elapsed() < BUDGET);
}

#[test]
fn test_lka1_zero_budget_never_touches_the_socket() {
    let daemon = MockDaemon::start(Behavior::Answer(LockWidgetState::Scanning, 1));
    assert!(matches!(
        query_lock_state(&daemon.path, my_uid(), Duration::ZERO),
        Err(LockStatusError::Timeout)
    ));
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(daemon.connections(), 0);
}

#[test]
fn test_lka1_overlong_socket_path_is_unavailable() {
    let long = std::path::PathBuf::from(format!("/tmp/{}", "x".repeat(200)));
    assert_eq!(fetch_view(&long, my_uid(), BUDGET), LockView::UNAVAILABLE);
}
