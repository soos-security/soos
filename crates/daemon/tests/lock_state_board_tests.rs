//! Contract tests of the daemon lock-state snapshot (GitHub #380, PR 1 of #352; ADR
//! 2026-10-13 "Read-Only Lock-Screen State Channel"; matrix LKS4-LKS6).
//!
//! The board is clockless: every call takes an explicit `now_ns`, so the projection rules
//! (reveal floor, hold, stale scanning guard, supersession, eviction) are tested exactly.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use soos_daemon::consensus::MAX_CAMERA_WAKE_WAIT_MS;
use soos_daemon::inference::{DEFAULT_INFERENCE_ESTIMATE_MS, RESPONSE_WRITE_MARGIN_MS};
use soos_daemon::lock_state::{
    LockStateBoard, LockStateView, ScanEnd, MAX_LOCK_STATE_UIDS, MAX_SCANNING_MS,
    NOT_RECOGNIZED_HOLD_MS, NOT_RECOGNIZED_REVEAL_FLOOR_MS, REVEAL_FLOOR_MARGIN_MS,
};
use soos_daemon::pipeline::{DECISION_BUDGET_MS, FRAME_POLL_INTERVAL_MS};
use soos_daemon::presence::PRESENCE_WAKE_SETTLE_MS;
use soos_protocol::types::LockWidgetState;

const MS: u64 = 1_000_000;
const T0: u64 = 1_000_000 * MS;
const UID: u32 = 1000;
const FLOOR: u64 = NOT_RECOGNIZED_REVEAL_FLOOR_MS * MS;
const HOLD: u64 = NOT_RECOGNIZED_HOLD_MS * MS;

fn view(state: LockWidgetState, sequence: u64) -> LockStateView {
    LockStateView { state, sequence }
}

#[test]
fn test_lks_constants_are_derived_and_bounded() {
    assert_eq!(
        NOT_RECOGNIZED_REVEAL_FLOOR_MS,
        MAX_CAMERA_WAKE_WAIT_MS
            + PRESENCE_WAKE_SETTLE_MS
            + DECISION_BUDGET_MS
            + RESPONSE_WRITE_MARGIN_MS
            + REVEAL_FLOOR_MARGIN_MS,
        "the reveal floor covers the longest regular non-matching scan plus its margin"
    );
    assert_eq!(
        REVEAL_FLOOR_MARGIN_MS,
        DEFAULT_INFERENCE_ESTIMATE_MS + FRAME_POLL_INTERVAL_MS,
        "margin: one nominal inference overrunning its admission plus one frame poll"
    );
    assert_eq!(NOT_RECOGNIZED_REVEAL_FLOOR_MS, 3240);
    assert_eq!(NOT_RECOGNIZED_HOLD_MS, 3000);
    assert_eq!(MAX_SCANNING_MS, 10_000);
    assert_eq!(MAX_LOCK_STATE_UIDS, 32);
    const { assert!(MAX_SCANNING_MS > NOT_RECOGNIZED_REVEAL_FLOOR_MS) };
}

#[test]
fn test_lks_unknown_uid_reads_idle_zero() {
    let board = LockStateBoard::new();
    assert_eq!(board.view(UID, T0), view(LockWidgetState::Idle, 0));
}

#[test]
fn test_lks_scan_reads_scanning_and_increments_sequence() {
    let board = LockStateBoard::new();
    let guard = board.begin_scan(UID, T0);
    assert_eq!(board.view(UID, T0), view(LockWidgetState::Scanning, 1));
    assert_eq!(
        board.view(UID, T0 + 500 * MS),
        view(LockWidgetState::Scanning, 1)
    );
    guard.finish(ScanEnd::Recognized, Some(T0 + 600 * MS));
    assert_eq!(
        board.view(UID, T0 + 600 * MS),
        view(LockWidgetState::Scanning, 1),
        "even a recognized end stays hidden until the floor (uniform rule)"
    );
    assert_eq!(
        board.view(UID, T0 + FLOOR),
        view(LockWidgetState::Idle, 2),
        "a recognized scan reads Idle: there is no success state"
    );
    assert_eq!(
        board.view(UID, T0 + FLOOR + HOLD + 1),
        view(LockWidgetState::Idle, 2)
    );
}

#[test]
fn test_lks_other_uid_sees_nothing() {
    let board = LockStateBoard::new();
    let guard = board.begin_scan(UID, T0);
    assert_eq!(board.view(UID + 1, T0), view(LockWidgetState::Idle, 0));
    guard.finish(ScanEnd::NotRecognized, Some(T0 + 100 * MS));
    assert_eq!(
        board.view(UID + 1, T0 + FLOOR),
        view(LockWidgetState::Idle, 0)
    );
    // The other UID's own counter starts at its own first transition.
    let other = board.begin_scan(UID + 1, T0 + FLOOR);
    assert_eq!(
        board.view(UID + 1, T0 + FLOOR),
        view(LockWidgetState::Scanning, 1)
    );
    drop(other);
}

#[test]
fn test_lks_not_recognized_is_hidden_until_the_reveal_floor() {
    let board = LockStateBoard::new();
    let guard = board.begin_scan(UID, T0);
    // A fast end (e.g. a presentation-attack veto at the first capture).
    guard.finish(ScanEnd::NotRecognized, Some(T0 + 50 * MS));
    assert_eq!(
        board.view(UID, T0 + 50 * MS),
        view(LockWidgetState::Scanning, 1),
        "neither the state nor the sequence reveals the early end"
    );
    assert_eq!(
        board.view(UID, T0 + FLOOR - 1),
        view(LockWidgetState::Scanning, 1)
    );
    assert_eq!(
        board.view(UID, T0 + FLOOR),
        view(LockWidgetState::NotRecognized, 2)
    );
    assert_eq!(
        board.view(UID, T0 + FLOOR + HOLD - 1),
        view(LockWidgetState::NotRecognized, 2)
    );
    assert_eq!(
        board.view(UID, T0 + FLOOR + HOLD),
        view(LockWidgetState::Idle, 2),
        "the hold decays to Idle"
    );
}

#[test]
fn test_lks_fast_and_slow_failures_are_indistinguishable() {
    // Same begin, one ends at 50 ms (spoof veto), the other at the decision budget.
    let fast = LockStateBoard::new();
    let slow = LockStateBoard::new();
    fast.begin_scan(UID, T0)
        .finish(ScanEnd::NotRecognized, Some(T0 + 50 * MS));
    slow.begin_scan(UID, T0)
        .finish(ScanEnd::NotRecognized, Some(T0 + DECISION_BUDGET_MS * MS));
    let mut now = T0;
    while now <= T0 + FLOOR + HOLD + 500 * MS {
        assert_eq!(fast.view(UID, now), slow.view(UID, now), "at {now}");
        now += 50 * MS;
    }
}

#[test]
fn test_lks_late_failure_reveals_at_its_end() {
    let board = LockStateBoard::new();
    let end = T0 + FLOOR + 400 * MS;
    board
        .begin_scan(UID, T0)
        .finish(ScanEnd::NotRecognized, Some(end));
    assert_eq!(board.view(UID, end - 1), view(LockWidgetState::Scanning, 1));
    assert_eq!(
        board.view(UID, end),
        view(LockWidgetState::NotRecognized, 2)
    );
    assert_eq!(
        board.view(UID, end + HOLD - 1),
        view(LockWidgetState::NotRecognized, 2)
    );
    assert_eq!(board.view(UID, end + HOLD), view(LockWidgetState::Idle, 2));
}

#[test]
fn test_lks_clock_failure_at_finish_reads_idle() {
    let board = LockStateBoard::new();
    board
        .begin_scan(UID, T0)
        .finish(ScanEnd::NotRecognized, None);
    assert_eq!(board.view(UID, T0), view(LockWidgetState::Scanning, 1));
    assert_eq!(board.view(UID, T0 + FLOOR), view(LockWidgetState::Idle, 2));
    assert_eq!(
        board.view(UID, T0 + FLOOR + HOLD + 1),
        view(LockWidgetState::Idle, 2)
    );
}

#[test]
fn test_lks_abandoned_scan_reads_idle_after_the_floor() {
    let board = LockStateBoard::new();
    let guard = board.begin_scan(UID, T0);
    drop(guard);
    assert_eq!(
        board.view(UID, T0 + MS),
        view(LockWidgetState::Scanning, 1),
        "an abandoned scan is hidden like any other end"
    );
    for now in [T0 + FLOOR, T0 + FLOOR + HOLD] {
        assert_eq!(board.view(UID, now), view(LockWidgetState::Idle, 2));
    }
}

#[test]
fn test_lks_stale_scanning_reads_idle() {
    let board = LockStateBoard::new();
    let guard = board.begin_scan(UID, T0);
    assert_eq!(
        board.view(UID, T0 + MAX_SCANNING_MS * MS),
        view(LockWidgetState::Scanning, 1)
    );
    assert_eq!(
        board.view(UID, T0 + MAX_SCANNING_MS * MS + 1),
        view(LockWidgetState::Idle, 1)
    );
    drop(guard);
}

#[test]
fn test_lks_clock_earlier_than_stamps_does_not_panic_or_reveal() {
    let board = LockStateBoard::new();
    board
        .begin_scan(UID, T0)
        .finish(ScanEnd::NotRecognized, Some(T0 + 10 * MS));
    assert_eq!(board.view(UID, 0), view(LockWidgetState::Scanning, 1));
    assert_eq!(board.view(UID, u64::MAX), view(LockWidgetState::Idle, 2));
    let guard = board.begin_scan(UID + 7, u64::MAX);
    assert_eq!(board.view(UID + 7, 0), view(LockWidgetState::Scanning, 1));
    guard.finish(ScanEnd::NotRecognized, Some(u64::MAX));
    assert_eq!(
        board.view(UID + 7, 0),
        view(LockWidgetState::Scanning, 1),
        "a clock reading before the scan start keeps the end hidden"
    );
    let saturated = board.view(UID + 7, u64::MAX);
    assert_ne!(saturated.state, LockWidgetState::Scanning);
    assert_eq!(saturated.sequence, 2);
}

#[test]
fn test_lks_later_scan_supersedes_the_earlier_guard() {
    let board = LockStateBoard::new();
    let presence = board.begin_scan(UID, T0);
    let pam = board.begin_scan(UID, T0 + 100 * MS);
    assert_eq!(
        board.view(UID, T0 + 100 * MS),
        view(LockWidgetState::Scanning, 3)
    );
    // The preempted presence scan settles: no effect on the PAM scan.
    drop(presence);
    assert_eq!(
        board.view(UID, T0 + 100 * MS + FLOOR + MS),
        view(LockWidgetState::Scanning, 3)
    );
    pam.finish(ScanEnd::NotRecognized, Some(T0 + 300 * MS));
    let reveal = T0 + 100 * MS + FLOOR;
    assert_eq!(
        board.view(UID, reveal - 1),
        view(LockWidgetState::Scanning, 3)
    );
    assert_eq!(
        board.view(UID, reveal),
        view(LockWidgetState::NotRecognized, 4)
    );
}

#[test]
fn test_lks_supersession_does_not_reveal_a_hidden_end() {
    // Board A: the first scan already ended (hidden, e.g. an early spoof veto).
    // Board B: the first scan is still running. A second scan begins on both.
    let a = LockStateBoard::new();
    let b = LockStateBoard::new();
    a.begin_scan(UID, T0)
        .finish(ScanEnd::NotRecognized, Some(T0 + 20 * MS));
    let running = b.begin_scan(UID, T0);
    let second_a = a.begin_scan(UID, T0 + 500 * MS);
    let second_b = b.begin_scan(UID, T0 + 500 * MS);
    for now in [T0 + 500 * MS, T0 + 600 * MS, T0 + FLOOR] {
        assert_eq!(a.view(UID, now), b.view(UID, now), "at {now}");
    }
    // Dropping the preempted running scan changes nothing visible either.
    drop(running);
    assert_eq!(a.view(UID, T0 + FLOOR), b.view(UID, T0 + FLOOR));
    drop(second_a);
    drop(second_b);
}

#[test]
fn test_lks_superseded_guard_finish_is_a_no_op() {
    let board = LockStateBoard::new();
    let first = board.begin_scan(UID, T0);
    let second = board.begin_scan(UID, T0 + MS);
    first.finish(ScanEnd::NotRecognized, Some(T0 + 2 * MS));
    assert_eq!(
        board.view(UID, T0 + FLOOR + 10 * MS),
        view(LockWidgetState::Scanning, 3)
    );
    second.finish(ScanEnd::Recognized, Some(T0 + 3 * MS));
    assert_eq!(
        board.view(UID, T0 + MS + FLOOR),
        view(LockWidgetState::Idle, 4)
    );
}

#[test]
fn test_lks_view_is_pure() {
    let board = LockStateBoard::new();
    let guard = board.begin_scan(UID, T0);
    for i in 0..1000 {
        let _ = board.view(UID, T0 + i * MS);
        let _ = board.view(UID + 1, T0 + i * MS);
    }
    assert_eq!(board.view(UID, T0), view(LockWidgetState::Scanning, 1));
    assert_eq!(board.view(UID + 1, T0), view(LockWidgetState::Idle, 0));
    assert_eq!(board.tracked_uids(), 1, "reads never create entries");
    drop(guard);
}

#[test]
fn test_lks_board_is_bounded_and_evicts_the_oldest() {
    let board = LockStateBoard::new();
    let total = MAX_LOCK_STATE_UIDS as u32 + 8;
    for i in 0..total {
        board
            .begin_scan(10_000 + i, T0 + u64::from(i) * MS)
            .finish(ScanEnd::Recognized, Some(T0 + u64::from(i) * MS));
    }
    assert_eq!(board.tracked_uids(), MAX_LOCK_STATE_UIDS);
    let late = T0 + 100 * MS + FLOOR;
    // The oldest entries were evicted and read as never published.
    assert_eq!(board.view(10_000, late), view(LockWidgetState::Idle, 0));
    assert_eq!(
        board.view(10_000 + total - 1, late),
        view(LockWidgetState::Idle, 2)
    );
}

#[test]
fn test_lks_clones_share_one_board() {
    let board = LockStateBoard::new();
    let clone = board.clone();
    let guard = clone.begin_scan(UID, T0);
    assert_eq!(board.view(UID, T0), view(LockWidgetState::Scanning, 1));
    drop(guard);
    assert_eq!(board.view(UID, T0 + FLOOR), view(LockWidgetState::Idle, 2));
}
