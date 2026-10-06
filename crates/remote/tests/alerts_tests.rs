//! Contract tests of the pure alert state of `soos-remote` (ADR 2026-10-06 "Failed-Password
//! Alerts in `soos-remote` From the System Journal", architect spec
//! `AI/architect_spec_remote_auth_alerts.md` §3.1, §4, tests 13–22 and 52; matrix RMC49–RMC51,
//! RMC54).
//!
//! The correlator (pairing, anchors, bounds), the alert book (coalescing, eviction,
//! saturation, acknowledgement, replay marker, persisted marker), the JSON view, and the
//! acknowledgement file. Pure except test 22 (a `TempDir`).
//!
//! Test seams defined by this contract (not named in the spec, needed because the bounds
//! cannot be reached by recording attempts one by one):
//! - `AlertBook::with_next_seq(self, next_seq: u64) -> Self` (`#[doc(hidden)]`): the seq the
//!   next recorded attempt receives; recording assigns it and needs `next_seq + 1` to exist,
//!   otherwise `Err(BookError::Overflow)` and nothing is recorded;
//! - `AlertBook::force_newest_count(&mut self, count: u32)` (`#[doc(hidden)]`): overwrites
//!   the `count` of the newest record (no-op without a record);
//! - `alerts::read_ack_file(path, owner_uid) -> Result<Option<u64>, AckFileError>` (`None` =
//!   absent) and `alerts::write_ack_file(path, acknowledged_until_us) -> Result<(),
//!   AckFileError>`: the ack-file read and write of spec §6.1.

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

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

use serde_json::{json, Value};

use soos_remote::alerts::{
    read_ack_file, write_ack_file, AlertBook, AlertRecord, AlertsEpoch, AlertsState, AlertsView,
    Attempt, AttemptKind, BookError, Correlator, LockScreenCoverage, UnavailableReason,
};
use soos_remote::journal::{AccountClass, HelperSide, Signal, SourceClass};
use soos_remote::{
    ACTION_ALERTS_ACK, ALERTS_ACK_FILE_NAME, ALERTS_EPOCH_HEADER, ALERTS_EPOCH_HEX_LEN,
    ALERTS_MARKER_FLUSH_MIN_INTERVAL_MS, ALERTS_THROUGH_HEADER, ALERT_COALESCE_WINDOW_US,
    ALERT_EVENT_MIN_INTERVAL_MS, ANCHOR_WINDOW_US, HISTORY_REBUILD_WINDOW_S,
    JOURNAL_BATCH_PAUSE_MS, JOURNAL_IDLE_TICK_MS, JOURNAL_LINES_PER_BATCH,
    JOURNAL_PROBE_TIMEOUT_MS, JOURNAL_READ_CHUNK_BYTES, JOURNAL_RESTART_MAX_MS,
    JOURNAL_RESTART_MIN_MS, JOURNAL_STABLE_RUN_MS, MAX_ALERTS_ACK_FILE_BYTES,
    MAX_ALERTS_THROUGH_DIGITS, MAX_ALERT_HISTORY, MAX_CURSOR_LEN, MAX_FIELD_BYTES,
    MAX_JOURNAL_LINE_BYTES, MAX_LOCK_SCREEN_PROGRAMS, MAX_MESSAGE_BYTES, MAX_PENDING_CHECKS,
    MAX_RECENT_FAILURES, MIN_ALERT_ACK_INTERVAL_MS, PAIR_WINDOW_US,
};

const T: u64 = 1_699_990_000_000_000;
const S: u64 = 1_000_000;
const MS: u64 = 1_000;

use AccountClass as A;
use AttemptKind as K;
use HelperSide as H;
use SourceClass as C;

fn chk(side: HelperSide, account: AccountClass, at_us: u64) -> Signal {
    Signal::Check {
        side,
        account,
        at_us,
    }
}

fn fail(
    class: SourceClass,
    side: HelperSide,
    account: AccountClass,
    at_us: u64,
    trusted: bool,
) -> Signal {
    Signal::Failure {
        class,
        side,
        account,
        at_us,
        trusted,
    }
}

fn locked(class: SourceClass, account: AccountClass, at_us: u64) -> Signal {
    Signal::LockedOut {
        class,
        account,
        at_us,
    }
}

fn attempt(class: SourceClass, account: AccountClass, kind: AttemptKind, at_us: u64) -> Attempt {
    Attempt {
        class,
        account,
        kind,
        at_us,
    }
}

fn wrong(class: SourceClass, at_us: u64) -> Attempt {
    attempt(class, A::Owner, K::WrongPassword, at_us)
}

/// Feeds every signal, then expires far in the future; returns every attempt in order.
fn run(signals: &[Signal]) -> Vec<Attempt> {
    let mut c = Correlator::new();
    let mut out = Vec::new();
    for s in signals {
        out.extend(c.push(*s));
    }
    out.extend(c.expire(u64::MAX));
    out
}

fn epoch() -> AlertsEpoch {
    AlertsEpoch::from_u64(0x0123_4567_89ab_cdef)
}

fn book() -> AlertBook {
    AlertBook::new(0, epoch(), T + 1_000 * S)
}

fn view(book: &AlertBook) -> AlertsView {
    book.view(AlertsState::Active, LockScreenCoverage::Monitored)
}

fn keys(value: &Value) -> BTreeSet<String> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("object expected: {value}"))
        .keys()
        .cloned()
        .collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

// ---------------------------------------------------------------------------------------
// §3.1 constants
// ---------------------------------------------------------------------------------------

/// §3.1: every alert constant has the specified value (single source in `lib.rs`).
#[test]
fn test_rmc_alerts_constants_match_the_spec() {
    assert_eq!(MAX_JOURNAL_LINE_BYTES, 24_576);
    assert_eq!(MAX_MESSAGE_BYTES, 4096);
    assert_eq!(MAX_FIELD_BYTES, 4096);
    assert_eq!(MAX_CURSOR_LEN, 256);
    assert_eq!(JOURNAL_PROBE_TIMEOUT_MS, 2000);
    assert_eq!(JOURNAL_RESTART_MIN_MS, 1000);
    assert_eq!(JOURNAL_RESTART_MAX_MS, 60_000);
    assert_eq!(JOURNAL_STABLE_RUN_MS, 60_000);
    assert_eq!(JOURNAL_LINES_PER_BATCH, 256);
    assert_eq!(JOURNAL_BATCH_PAUSE_MS, 50);
    assert_eq!(HISTORY_REBUILD_WINDOW_S, 86_400);
    assert_eq!(PAIR_WINDOW_US, 2_000_000);
    assert_eq!(ANCHOR_WINDOW_US, 3_600_000_000);
    assert_eq!(MAX_PENDING_CHECKS, 16);
    assert_eq!(MAX_RECENT_FAILURES, 16);
    assert_eq!(MAX_ALERT_HISTORY, 32);
    assert_eq!(ALERT_COALESCE_WINDOW_US, 60_000_000);
    assert_eq!(ALERT_EVENT_MIN_INTERVAL_MS, 1000);
    assert_eq!(MIN_ALERT_ACK_INTERVAL_MS, 1000);
    assert_eq!(MAX_ALERTS_ACK_FILE_BYTES, 256);
    assert_eq!(ALERTS_ACK_FILE_NAME, "remote-alerts.json");
    assert_eq!(MAX_LOCK_SCREEN_PROGRAMS, 4);
    assert_eq!(ACTION_ALERTS_ACK, "alerts-ack");
    assert_eq!(ALERTS_EPOCH_HEADER, "x-soos-alerts-epoch");
    assert_eq!(ALERTS_THROUGH_HEADER, "x-soos-alerts-through");
    assert_eq!(ALERTS_EPOCH_HEX_LEN, 16);
    assert_eq!(MAX_ALERTS_THROUGH_DIGITS, 20);
    assert_eq!(JOURNAL_READ_CHUNK_BYTES, 4096);
    assert_eq!(JOURNAL_IDLE_TICK_MS, 2000);
    assert_eq!(ALERTS_MARKER_FLUSH_MIN_INTERVAL_MS, 1000);
}

// ---------------------------------------------------------------------------------------
// Tests 13–15 — correlator
// ---------------------------------------------------------------------------------------

/// Test 13 (RMC49, RMC50, §4.2 scenario table): every row.
#[test]
fn test_rmc_alerts_correlator_scenarios() {
    // Row 1–2: lock screen check + trusted failure → 1; then a lone check 60 s later is
    // attributed to the lock screen through the anchor.
    let mut c = Correlator::new();
    assert!(c.push(chk(H::Owner, A::Owner, T)).is_empty());
    assert_eq!(c.oldest_pending_us(), Some(T));
    assert_eq!(
        c.push(fail(C::LockScreen, H::Owner, A::Owner, T + 5 * MS, true)),
        vec![wrong(C::LockScreen, T + 5 * MS)]
    );
    assert_eq!(c.oldest_pending_us(), None, "the check was paired");
    assert!(c.expire(T + 10 * S).is_empty());
    assert!(c.push(chk(H::Owner, A::Owner, T + 60 * S)).is_empty());
    assert!(c.expire(T + 60 * S + PAIR_WINDOW_US).is_empty(), "not yet");
    assert_eq!(
        c.expire(T + 60 * S + PAIR_WINDOW_US + 1),
        vec![wrong(C::LockScreen, T + 60 * S)]
    );
    assert_eq!(c.oldest_pending_us(), None);

    // Row 3: an owner-side check without an anchor is dropped.
    assert!(run(&[chk(H::Owner, A::Owner, T)]).is_empty());

    // Row 4: test binary — untrusted failure + its own check → nothing, in both orders.
    assert!(run(&[
        fail(C::LockScreen, H::Owner, A::Owner, T, false),
        chk(H::Owner, A::Owner, T + 3 * MS),
    ])
    .is_empty());
    assert!(run(&[
        chk(H::Owner, A::Owner, T),
        fail(C::LockScreen, H::Owner, A::Owner, T + 3 * MS, false),
    ])
    .is_empty());

    // Rows 5–6: sudo — trusted failure, then its check (logged 4 ms earlier, read later),
    // then a second check 7 s later on the same handle.
    let mut c = Correlator::new();
    assert_eq!(
        c.push(fail(C::Sudo, H::Root, A::Owner, T, true)),
        vec![wrong(C::Sudo, T)]
    );
    assert!(c.push(chk(H::Root, A::Owner, T - 4 * MS)).is_empty());
    assert!(c.expire(T + 5 * S).is_empty(), "paired, not pending");
    assert!(c.push(chk(H::Root, A::Owner, T + 7 * S)).is_empty());
    assert_eq!(c.expire(T + 10 * S), vec![wrong(C::Sudo, T + 7 * S)]);

    // Row 7: a root-side check without an anchor counts as `other`.
    assert_eq!(
        run(&[chk(H::Root, A::Other, T)]),
        vec![attempt(C::Other, A::Other, K::WrongPassword, T)]
    );

    // Row 8: gdm — trusted root failure + its check → 1 login.
    assert_eq!(
        run(&[
            fail(C::Login, H::Root, A::Owner, T, true),
            chk(H::Root, A::Owner, T + 2 * MS),
        ]),
        vec![wrong(C::Login, T)]
    );

    // Row 9: three attempts during a lockout → three `locked_out`.
    assert_eq!(
        run(&[
            locked(C::Login, A::Owner, T),
            locked(C::Login, A::Owner, T + 13 * S),
            locked(C::Login, A::Owner, T + 18 * S),
        ]),
        vec![
            attempt(C::Login, A::Owner, K::LockedOut, T),
            attempt(C::Login, A::Owner, K::LockedOut, T + 13 * S),
            attempt(C::Login, A::Owner, K::LockedOut, T + 18 * S),
        ]
    );
    // A lockout line never anchors: a later lone owner-side check is still dropped.
    assert!(run(&[
        locked(C::LockScreen, A::Owner, T),
        chk(H::Owner, A::Owner, T + 10 * S),
    ])
    .into_iter()
    .all(|a| a.kind == K::LockedOut));

    // Row 10: an owner-side check more than 1 h after the last anchor is dropped; exactly
    // 1 h is still attributed.
    let anchored = |delta: u64| {
        run(&[
            fail(C::LockScreen, H::Owner, A::Owner, T, true),
            chk(H::Owner, A::Owner, T + delta),
        ])
    };
    assert_eq!(
        anchored(ANCHOR_WINDOW_US + 1),
        vec![wrong(C::LockScreen, T)],
        "1 h + 1 µs: dropped"
    );
    assert_eq!(
        anchored(ANCHOR_WINDOW_US),
        vec![
            wrong(C::LockScreen, T),
            wrong(C::LockScreen, T + ANCHOR_WINDOW_US)
        ],
        "exactly 1 h: attributed"
    );
    // An attributed check refreshes the anchor: a chain of checks 50 min apart stays
    // attributed for longer than 1 h after the failure.
    let mut c = Correlator::new();
    assert_eq!(
        c.push(fail(C::LockScreen, H::Owner, A::Owner, T, true))
            .len(),
        1
    );
    assert!(c.push(chk(H::Owner, A::Owner, T + 3000 * S)).is_empty());
    assert_eq!(
        c.expire(T + 3003 * S),
        vec![wrong(C::LockScreen, T + 3000 * S)]
    );
    assert!(c.push(chk(H::Owner, A::Owner, T + 6000 * S)).is_empty());
    assert_eq!(
        c.expire(T + 6003 * S),
        vec![wrong(C::LockScreen, T + 6000 * S)],
        "the attributed check at +50 min refreshed the anchor"
    );
    // Anchors are per side and account: a root-side anchor never attributes an owner check.
    assert_eq!(
        run(&[
            fail(C::Sudo, H::Root, A::Owner, T, true),
            chk(H::Owner, A::Owner, T + 10 * S),
        ]),
        vec![wrong(C::Sudo, T)]
    );
    assert_eq!(
        run(&[
            fail(C::Sudo, H::Root, A::Owner, T, true),
            chk(H::Root, A::Root, T + 10 * S),
        ]),
        vec![
            wrong(C::Sudo, T),
            attempt(C::Other, A::Root, K::WrongPassword, T + 10 * S)
        ]
    );

    // Row 11: a 17th pending check resolves the oldest at once.
    let mut c = Correlator::new();
    for i in 0..16 {
        assert!(
            c.push(chk(H::Root, A::Other, T + i)).is_empty(),
            "check {i}"
        );
    }
    assert_eq!(
        c.push(chk(H::Root, A::Other, T + 16)),
        vec![attempt(C::Other, A::Other, K::WrongPassword, T)]
    );
    assert_eq!(c.oldest_pending_us(), Some(T + 1));
}

/// Test 14 (RMC49, §4.2 rules 1, 2, 5): pairing in both orders, 2 s inclusive; one check
/// pairs with at most one failure and vice versa; the expiry boundary is strict.
#[test]
fn test_rmc_alerts_correlator_pairs_in_both_orders() {
    for delta in [0, 1, PAIR_WINDOW_US] {
        // Check then failure.
        assert_eq!(
            run(&[
                chk(H::Owner, A::Owner, T),
                fail(C::LockScreen, H::Owner, A::Owner, T + delta, true),
            ]),
            vec![wrong(C::LockScreen, T + delta)],
            "check then failure, Δ = {delta}"
        );
        // Failure then check.
        assert_eq!(
            run(&[
                fail(C::LockScreen, H::Owner, A::Owner, T, true),
                chk(H::Owner, A::Owner, T + delta),
            ]),
            vec![wrong(C::LockScreen, T)],
            "failure then check, Δ = {delta}"
        );
        // Failure then an earlier check (read later).
        assert_eq!(
            run(&[
                fail(C::Sudo, H::Root, A::Owner, T + delta, true),
                chk(H::Root, A::Owner, T),
            ]),
            vec![wrong(C::Sudo, T + delta)],
            "failure then earlier check, Δ = {delta}"
        );
    }
    // 2 s + 1 µs: no pairing, the check becomes a second attempt through the anchor.
    let beyond = PAIR_WINDOW_US + 1;
    assert_eq!(
        run(&[
            chk(H::Owner, A::Owner, T),
            fail(C::LockScreen, H::Owner, A::Owner, T + beyond, true),
        ]),
        vec![wrong(C::LockScreen, T + beyond), wrong(C::LockScreen, T)]
    );
    assert_eq!(
        run(&[
            fail(C::LockScreen, H::Owner, A::Owner, T, true),
            chk(H::Owner, A::Owner, T + beyond),
        ]),
        vec![wrong(C::LockScreen, T), wrong(C::LockScreen, T + beyond)]
    );
    // Pairing needs the same side and account.
    assert_eq!(
        run(&[
            fail(C::Sudo, H::Root, A::Owner, T, true),
            chk(H::Root, A::Root, T + MS),
        ]),
        vec![
            wrong(C::Sudo, T),
            attempt(C::Other, A::Root, K::WrongPassword, T + MS)
        ]
    );
    // Rule 5: one check, two failures → two attempts (both from the failures).
    assert_eq!(
        run(&[
            chk(H::Root, A::Owner, T),
            fail(C::Sudo, H::Root, A::Owner, T + MS, true),
            fail(C::Sudo, H::Root, A::Owner, T + 2 * MS, true),
        ]),
        vec![wrong(C::Sudo, T + MS), wrong(C::Sudo, T + 2 * MS)]
    );
    // Rule 5: one failure, two checks → the second check is a separate attempt.
    assert_eq!(
        run(&[
            fail(C::Sudo, H::Root, A::Owner, T, true),
            chk(H::Root, A::Owner, T + MS),
            chk(H::Root, A::Owner, T + 2 * MS),
        ]),
        vec![wrong(C::Sudo, T), wrong(C::Sudo, T + 2 * MS)]
    );
    // An untrusted failure absorbs one owner check only.
    assert_eq!(
        run(&[
            fail(C::LockScreen, H::Owner, A::Owner, T - 30 * S, true),
            fail(C::LockScreen, H::Owner, A::Owner, T, false),
            chk(H::Owner, A::Owner, T + MS),
            chk(H::Owner, A::Owner, T + 2 * MS),
        ]),
        vec![
            wrong(C::LockScreen, T - 30 * S),
            wrong(C::LockScreen, T + 2 * MS)
        ]
    );
    // Expiry boundary: `at + PAIR_WINDOW_US < now`.
    let mut c = Correlator::new();
    assert!(c.push(chk(H::Root, A::Other, T)).is_empty());
    assert!(c.expire(T + PAIR_WINDOW_US).is_empty());
    assert_eq!(c.oldest_pending_us(), Some(T));
    assert_eq!(
        c.expire(T + PAIR_WINDOW_US + 1),
        vec![attempt(C::Other, A::Other, K::WrongPassword, T)]
    );
    assert!(c.expire(u64::MAX).is_empty());
    // Expiring with a time before the check (a clock step) resolves nothing and never
    // panics.
    let mut c = Correlator::new();
    assert!(c.push(chk(H::Root, A::Other, T)).is_empty());
    assert!(c.expire(0).is_empty());
    assert!(c.expire(T - 1).is_empty());
    // Times at the extremes never panic.
    let _ = run(&[
        chk(H::Root, A::Owner, u64::MAX),
        fail(C::Sudo, H::Root, A::Owner, 0, true),
        chk(H::Owner, A::Owner, 0),
        fail(C::LockScreen, H::Owner, A::Owner, u64::MAX, true),
    ]);
}

/// Test 15 (RMC49, RMC51, §3.1): at most 16 pending checks and 16 recent failures; the
/// bounds are observable through the attempts emitted.
#[test]
fn test_rmc_alerts_correlator_bounds() {
    // Pending checks: every push beyond 16 resolves the oldest at once.
    let mut c = Correlator::new();
    let mut emitted = Vec::new();
    for i in 0..40u64 {
        emitted.extend(c.push(chk(H::Root, A::Other, T + i * MS)));
    }
    assert_eq!(emitted.len(), 40 - MAX_PENDING_CHECKS);
    for (i, a) in emitted.iter().enumerate() {
        assert_eq!(a.at_us, T + i as u64 * MS, "oldest first");
    }
    assert_eq!(c.oldest_pending_us(), Some(T + 24 * MS));
    assert_eq!(c.expire(u64::MAX).len(), MAX_PENDING_CHECKS);

    // Recent failures: 17 trusted sudo failures 10 s apart; the oldest is evicted and can no
    // longer absorb its check (one extra attempt), the second one still can.
    let mut c = Correlator::new();
    let mut emitted = Vec::new();
    for i in 0..=MAX_RECENT_FAILURES as u64 {
        emitted.extend(c.push(fail(C::Sudo, H::Root, A::Owner, T + i * 10 * S, true)));
    }
    assert_eq!(emitted.len(), MAX_RECENT_FAILURES + 1);
    assert!(
        c.push(chk(H::Root, A::Owner, T + 10 * S + MS)).is_empty(),
        "the second failure is still kept and absorbs its check"
    );
    assert!(c.expire(u64::MAX).is_empty());
    assert!(c.push(chk(H::Root, A::Owner, T + MS)).is_empty());
    assert_eq!(
        c.expire(u64::MAX),
        vec![wrong(C::Sudo, T + MS)],
        "the evicted failure no longer absorbs its check: one extra attempt, never one fewer"
    );
}

// ---------------------------------------------------------------------------------------
// Tests 16–20, 52 — book
// ---------------------------------------------------------------------------------------

/// Test 16 (RMC51, §4.3): coalescing joins the newest record only (same source, account,
/// kind, acknowledgement state, within 60 s inclusive); 33 records evict the oldest, whose
/// unacknowledged attempts stay in the totals.
#[test]
fn test_rmc_alerts_book_coalescing_and_eviction() {
    let mut b = book();
    b.record(wrong(C::LockScreen, T)).unwrap();
    b.record(wrong(C::LockScreen, T + ALERT_COALESCE_WINDOW_US))
        .unwrap();
    let v = view(&b);
    assert_eq!(v.history.len(), 1);
    assert_eq!(v.history[0].count, 2);
    assert_eq!(v.history[0].id, 1);
    assert_eq!(v.history[0].first_unix_ms, T / 1000);
    assert_eq!(
        v.history[0].last_unix_ms,
        (T + ALERT_COALESCE_WINDOW_US) / 1000
    );
    assert_eq!(v.through, 2);

    // 60 s + 1 µs after the newest attempt: a new record.
    let t3 = T + 2 * ALERT_COALESCE_WINDOW_US + 1;
    b.record(wrong(C::LockScreen, t3)).unwrap();
    // Another kind, source or account: a new record each.
    b.record(attempt(C::LockScreen, A::Owner, K::LockedOut, t3 + S))
        .unwrap();
    b.record(wrong(C::Sudo, t3 + 2 * S)).unwrap();
    b.record(attempt(C::Sudo, A::Root, K::WrongPassword, t3 + 3 * S))
        .unwrap();
    // Only the newest record coalesces: a lock-screen attempt within 60 s of record 3 starts
    // a new record because the newest one is the root sudo record.
    b.record(wrong(C::LockScreen, t3 + 4 * S)).unwrap();
    // An attempt earlier than the newest record's last (late resolution) within the window
    // joins it and lowers `first`.
    b.record(wrong(C::LockScreen, t3 + 3 * S + 500 * MS))
        .unwrap();
    let v = view(&b);
    let summary: Vec<(u64, C, A, K, u32)> = v
        .history
        .iter()
        .map(|r| (r.id, r.source, r.account, r.kind, r.count))
        .collect();
    assert_eq!(
        summary,
        vec![
            (7, C::LockScreen, A::Owner, K::WrongPassword, 2),
            (6, C::Sudo, A::Root, K::WrongPassword, 1),
            (5, C::Sudo, A::Owner, K::WrongPassword, 1),
            (4, C::LockScreen, A::Owner, K::LockedOut, 1),
            (3, C::LockScreen, A::Owner, K::WrongPassword, 1),
            (1, C::LockScreen, A::Owner, K::WrongPassword, 2),
        ],
        "newest first"
    );
    assert_eq!(v.history[0].first_unix_ms, (t3 + 3 * S + 500 * MS) / 1000);
    assert_eq!(v.history[0].last_unix_ms, (t3 + 4 * S) / 1000);
    assert_eq!(v.unacknowledged_wrong_password, 7);
    assert_eq!(v.unacknowledged_locked_out, 1);
    assert_eq!(v.last_unix_ms, Some((t3 + 4 * S) / 1000));
    assert_eq!(v.last_source, Some(C::LockScreen));
    assert_eq!(v.through, 8);

    // Eviction: 33 non-coalescing records (alternating kinds, the oldest a wrong password).
    let mut b = book();
    for i in 0..=MAX_ALERT_HISTORY as u64 {
        let kind = if i % 2 == 0 {
            K::WrongPassword
        } else {
            K::LockedOut
        };
        b.record(attempt(C::Login, A::Owner, kind, T + i * S))
            .unwrap();
    }
    let v = view(&b);
    assert_eq!(v.history.len(), MAX_ALERT_HISTORY);
    assert_eq!(v.history.last().unwrap().id, 2, "the oldest record is gone");
    assert_eq!(v.history[0].id, 33);
    assert_eq!(
        v.unacknowledged_wrong_password, 17,
        "evicted attempts stay counted"
    );
    assert_eq!(v.unacknowledged_locked_out, 16);
    assert_eq!(v.through, 33);
    // Memory stays bounded however many attempts arrive.
    for i in 0..1000u64 {
        let kind = if i % 2 == 0 {
            K::WrongPassword
        } else {
            K::LockedOut
        };
        b.record(attempt(C::Login, A::Owner, kind, T + 100 * S + i * S))
            .unwrap();
    }
    let v = view(&b);
    assert_eq!(v.history.len(), MAX_ALERT_HISTORY);
    assert_eq!(v.unacknowledged_wrong_password, 517);
    assert_eq!(v.unacknowledged_locked_out, 516);
}

/// Test 17 (RMC51): counts and totals saturate at `u32::MAX` (seam `force_newest_count`).
#[test]
fn test_rmc_alerts_book_counts_saturate() {
    let mut b = book();
    b.record(wrong(C::Sudo, T)).unwrap();
    b.force_newest_count(u32::MAX - 1);
    b.record(wrong(C::Sudo, T + S)).unwrap();
    assert_eq!(view(&b).history[0].count, u32::MAX);
    b.record(wrong(C::Sudo, T + 2 * S)).unwrap();
    let v = view(&b);
    assert_eq!(v.history.len(), 1, "still one record");
    assert_eq!(v.history[0].count, u32::MAX, "saturated, not wrapped");
    assert_eq!(v.unacknowledged_wrong_password, u32::MAX);
    // A second saturated record: the total saturates too.
    b.record(wrong(C::Login, T + 3 * S)).unwrap();
    b.force_newest_count(u32::MAX);
    assert_eq!(view(&b).unacknowledged_wrong_password, u32::MAX);
    // Evicting both saturated records keeps the evicted total saturated.
    for i in 0..MAX_ALERT_HISTORY as u64 {
        b.record(attempt(
            C::Other,
            A::Other,
            K::LockedOut,
            T + 100 * S + i * 120 * S,
        ))
        .unwrap();
        b.record(attempt(
            C::Other,
            A::Root,
            K::LockedOut,
            T + 160 * S + i * 120 * S,
        ))
        .unwrap();
    }
    let v = view(&b);
    assert_eq!(v.history.len(), MAX_ALERT_HISTORY);
    assert!(v.history.iter().all(|r| r.kind == K::LockedOut));
    assert_eq!(v.unacknowledged_wrong_password, u32::MAX);
}

/// Test 18 (RMC54, §4.3 rule A): `through` semantics within one epoch.
#[test]
fn test_rmc_alerts_book_acknowledge_through() {
    let mut b = book();
    b.record(wrong(C::Sudo, T)).unwrap(); // seq 1, record 1
    let snapshot = view(&b);
    assert_eq!(snapshot.through, 1);
    b.record(wrong(C::Sudo, T + S)).unwrap(); // seq 2, coalesces into record 1
    b.record(attempt(C::Login, A::Owner, K::LockedOut, T + 2 * S))
        .unwrap(); // seq 3, record 3

    // No-op, beyond newest, stale epoch: nothing changes.
    let before = view(&b);
    assert_eq!(b.acknowledge(epoch(), 0), Ok(()));
    assert_eq!(view(&b), before);
    assert_eq!(b.acknowledge(epoch(), 4), Err(BookError::BeyondNewest));
    assert_eq!(view(&b), before);
    assert_eq!(
        b.acknowledge(AlertsEpoch::from_u64(7), 1),
        Err(BookError::StaleView)
    );
    assert_eq!(view(&b), before);
    assert_eq!(b.marker(None), 0, "nothing acknowledged yet");

    // The page displayed `through = 1`; record 1 grew to seq 2 since: it stays
    // unacknowledged as a whole.
    b.acknowledge(epoch(), snapshot.through).unwrap();
    let v = view(&b);
    assert!(v.history.iter().all(|r| !r.acknowledged), "{v:?}");
    assert_eq!(v.unacknowledged_wrong_password, 2);

    // `through = 2` covers record 1 (last seq 2) but not record 3.
    b.acknowledge(epoch(), 2).unwrap();
    let v = view(&b);
    assert_eq!(v.history[1].id, 1);
    assert!(v.history[1].acknowledged);
    assert!(!v.history[0].acknowledged);
    assert_eq!(v.unacknowledged_wrong_password, 0);
    assert_eq!(v.unacknowledged_locked_out, 1);
    assert_eq!(v.last_unix_ms, Some((T + 2 * S) / 1000));
    assert_eq!(v.last_source, Some(C::Login));
    assert_eq!(
        b.marker(None),
        T + S,
        "min(high water T + 1 s, unacknowledged T + 2 s − 1)"
    );

    // Everything.
    b.acknowledge(epoch(), 3).unwrap();
    let v = view(&b);
    assert!(v.history.iter().all(|r| r.acknowledged));
    assert_eq!(v.unacknowledged_wrong_password, 0);
    assert_eq!(v.unacknowledged_locked_out, 0);
    assert_eq!(v.last_unix_ms, None);
    assert_eq!(v.last_source, None);
    assert_eq!(b.marker(None), T + 2 * S, "the high water");

    // A live attempt never joins an acknowledged record, even within 60 s.
    b.record(attempt(C::Login, A::Owner, K::LockedOut, T + 3 * S))
        .unwrap();
    let v = view(&b);
    assert_eq!(v.history.len(), 3);
    assert!(!v.history[0].acknowledged);
    assert_eq!(v.history[0].id, 4);
    assert_eq!(v.unacknowledged_locked_out, 1);
    // The high water never decreases: re-acknowledging an old snapshot keeps it.
    b.acknowledge(epoch(), 1).unwrap();
    assert_eq!(b.marker(None), T + 2 * S);
    // An unacknowledged record older than the high water lowers the marker (fail-safe).
    b.record(wrong(C::Other, T + S + 500 * MS)).unwrap();
    assert_eq!(b.marker(None), T + S + 500 * MS - 1);
    // The acknowledged state survives in the view: acknowledged rows stay in the history.
    assert_eq!(
        view(&b).history.iter().filter(|r| r.acknowledged).count(),
        2
    );
}

/// Test 19 (RMC54, §4.3 rule R, F-3 b): the loaded marker acknowledges only replayed
/// attempts (journal time at or before the marker **and** before the service start).
#[test]
fn test_rmc_alerts_book_rebuild_respects_marker() {
    let marker = T + 50 * S;
    let started = T + 40 * S;
    let mut b = AlertBook::new(marker, epoch(), started);
    // Attempts far apart so that nothing coalesces.
    b.record(attempt(C::Sudo, A::Owner, K::WrongPassword, T))
        .unwrap();
    b.record(attempt(C::Login, A::Owner, K::WrongPassword, started - 1))
        .unwrap();
    b.record(attempt(C::Other, A::Owner, K::WrongPassword, started))
        .unwrap();
    b.record(attempt(C::LockScreen, A::Owner, K::WrongPassword, marker))
        .unwrap();
    b.record(attempt(C::Sudo, A::Root, K::WrongPassword, marker + 1))
        .unwrap();
    let v = view(&b);
    let acknowledged: Vec<(C, bool)> = v
        .history
        .iter()
        .rev()
        .map(|r| (r.source, r.acknowledged))
        .collect();
    assert_eq!(
        acknowledged,
        vec![
            (C::Sudo, true),
            (C::Login, true),
            (C::Other, false),
            (C::LockScreen, false),
            (C::Sudo, false),
        ]
    );
    assert_eq!(v.unacknowledged_wrong_password, 3);
    // A marker past the start never acknowledges live attempts.
    let mut b = AlertBook::new(u64::MAX, epoch(), started);
    b.record(wrong(C::Sudo, started + S)).unwrap();
    assert_eq!(view(&b).unacknowledged_wrong_password, 1);
    // Marker 0: only an attempt at journal time 0 (at the marker, before the start) is
    // acknowledged on arrival.
    let mut b = AlertBook::new(0, epoch(), started);
    b.record(wrong(C::Sudo, 0)).unwrap();
    b.record(wrong(C::Login, 1)).unwrap();
    let v = view(&b);
    assert_eq!(v.unacknowledged_wrong_password, 1);
    // The loaded marker is kept as the persisted value while nothing is unacknowledged
    // below it (no regression of the file across restarts, plan-evaluator G-1).
    let b = AlertBook::new(marker, epoch(), started);
    assert_eq!(b.marker(None), marker);
    let mut b = AlertBook::new(marker, epoch(), started);
    b.record(wrong(C::Sudo, T)).unwrap();
    assert_eq!(
        b.marker(None),
        marker,
        "replayed acknowledged attempts keep it"
    );
    b.record(wrong(C::Login, marker + 10 * S)).unwrap();
    assert_eq!(
        b.marker(None),
        marker,
        "a newer unacknowledged attempt keeps it"
    );
}

/// Test 20 (RMC51): the attempt counter overflows to `Err(Overflow)` without panicking and
/// without recording (seam `with_next_seq`).
#[test]
fn test_rmc_alerts_book_seq_overflow() {
    let mut b = book().with_next_seq(u64::MAX - 1);
    b.record(wrong(C::Sudo, T)).unwrap();
    assert_eq!(view(&b).through, u64::MAX - 1);
    assert_eq!(b.record(wrong(C::Sudo, T + S)), Err(BookError::Overflow));
    let v = view(&b);
    assert_eq!(v.through, u64::MAX - 1, "nothing recorded");
    assert_eq!(v.history.len(), 1);
    assert_eq!(v.history[0].count, 1);
    let mut b = book().with_next_seq(u64::MAX);
    assert_eq!(b.record(wrong(C::Sudo, T)), Err(BookError::Overflow));
    assert_eq!(view(&b).through, 0);
    assert!(view(&b).history.is_empty());
    // Acknowledging at the top of the range works.
    let mut b = book().with_next_seq(u64::MAX - 1);
    b.record(wrong(C::Sudo, T)).unwrap();
    assert_eq!(
        b.acknowledge(epoch(), u64::MAX),
        Err(BookError::BeyondNewest)
    );
    b.acknowledge(epoch(), u64::MAX - 1).unwrap();
    assert_eq!(view(&b).unacknowledged_wrong_password, 0);
}

/// Test 52 (RMC54, §4.3 rule M, F-3 b): the persisted marker is always strictly below
/// every attempt the service knows to be unacknowledged, pending checks included.
#[test]
fn test_rmc_alerts_book_marker_never_covers_unacknowledged() {
    // No record: the high water (0 at first).
    let b = book();
    assert_eq!(b.marker(None), 0);
    // An unacknowledged record at t: min(high water, t − 1).
    let mut b = book();
    b.record(wrong(C::Sudo, T)).unwrap();
    assert_eq!(b.marker(None), 0);
    b.acknowledge(epoch(), 1).unwrap();
    assert_eq!(b.marker(None), T);
    b.record(wrong(C::Login, T + 10 * S)).unwrap();
    assert_eq!(b.marker(None), T, "min(high water, t − 1)");
    // A pending check at t older than an acknowledged record ending at t + 1 s.
    let mut b = book();
    b.record(wrong(C::Sudo, T + S)).unwrap();
    b.acknowledge(epoch(), 1).unwrap();
    assert_eq!(b.marker(None), T + S);
    assert_eq!(b.marker(Some(T)), T - 1, "below the pending check");
    // Evicted unacknowledged attempts keep the marker below them until acknowledged.
    let mut b = book();
    b.record(wrong(C::Login, T)).unwrap();
    for i in 1..=MAX_ALERT_HISTORY as u64 {
        b.record(wrong(C::Sudo, T + i * 120 * S)).unwrap();
    }
    assert_eq!(view(&b).history.len(), MAX_ALERT_HISTORY);
    assert_eq!(view(&b).history.last().unwrap().id, 2, "record 1 evicted");
    assert_eq!(b.marker(None), 0, "nothing acknowledged yet");
    // A through covering the evicted seq resets the evicted totals.
    b.acknowledge(epoch(), 33).unwrap();
    let v = view(&b);
    assert_eq!(v.unacknowledged_wrong_password, 0);
    assert_eq!(b.marker(None), T + 32 * 120 * S);
    // Evicted unacknowledged attempt below the high water: marker below it.
    let mut b = book();
    b.record(wrong(C::Login, T + 5000 * S)).unwrap(); // seq 1
    b.acknowledge(epoch(), 1).unwrap(); // high water T + 5000 s
    b.record(wrong(C::Other, T)).unwrap(); // seq 2, late, unacknowledged, oldest time
    for i in 0..MAX_ALERT_HISTORY as u64 {
        b.record(attempt(
            C::Sudo,
            A::Owner,
            K::LockedOut,
            T + 6000 * S + i * 120 * S,
        ))
        .unwrap();
    }
    let v = view(&b);
    assert!(
        v.history.iter().all(|r| r.id > 2),
        "records 1 and 2 evicted"
    );
    assert_eq!(
        v.unacknowledged_wrong_password, 1,
        "the evicted one is still counted"
    );
    assert_eq!(
        b.marker(None),
        T - 1,
        "below the evicted unacknowledged attempt"
    );
    // Saturation at 0 for an attempt at time 0 (a book started at time 0, so that nothing
    // is a replayed attempt).
    let mut b = AlertBook::new(0, epoch(), 0);
    b.record(wrong(C::Sudo, 5 * S)).unwrap();
    b.acknowledge(epoch(), 1).unwrap();
    assert_eq!(b.marker(Some(0)), 0);
    b.record(wrong(C::Login, 0)).unwrap();
    assert_eq!(b.marker(None), 0);
    // A late attempt below the previous marker lowers it.
    let mut b = book();
    b.record(wrong(C::Sudo, T + 100 * S)).unwrap();
    b.acknowledge(epoch(), 1).unwrap();
    assert_eq!(b.marker(None), T + 100 * S);
    b.record(wrong(C::LockScreen, T + 50 * S)).unwrap();
    assert_eq!(b.marker(None), T + 50 * S - 1);
}

// ---------------------------------------------------------------------------------------
// Test 21 — view JSON
// ---------------------------------------------------------------------------------------

/// Test 21 (RMC48, §4.4): exact key sets of the view and of a record, enum spellings, the
/// epoch rendering, and the disabled view.
#[test]
fn test_rmc_alerts_view_json_shape() {
    let top = set(&[
        "state",
        "reason",
        "epoch",
        "lock_screen",
        "unacknowledged_wrong_password",
        "unacknowledged_locked_out",
        "last_unix_ms",
        "last_source",
        "through",
        "history",
    ]);
    let record_keys = set(&[
        "id",
        "first_unix_ms",
        "last_unix_ms",
        "source",
        "account",
        "kind",
        "count",
        "acknowledged",
    ]);
    let mut b = book();
    b.record(wrong(C::LockScreen, T)).unwrap();
    b.record(attempt(C::Login, A::Root, K::LockedOut, T + 90 * S))
        .unwrap();
    let json = serde_json::to_value(view(&b)).unwrap();
    assert_eq!(keys(&json), top);
    assert_eq!(json["state"], "active");
    assert_eq!(json["reason"], Value::Null);
    assert_eq!(json["epoch"], "0123456789abcdef");
    assert_eq!(json["lock_screen"], "monitored");
    assert_eq!(json["unacknowledged_wrong_password"], 1);
    assert_eq!(json["unacknowledged_locked_out"], 1);
    assert_eq!(json["last_unix_ms"], (T + 90 * S) / 1000);
    assert_eq!(json["last_source"], "login");
    assert_eq!(json["through"], 2);
    let history = json["history"].as_array().unwrap();
    assert_eq!(history.len(), 2);
    for record in history {
        assert_eq!(keys(record), record_keys);
    }
    assert_eq!(
        history[0],
        json!({
            "id": 2,
            "first_unix_ms": (T + 90 * S) / 1000,
            "last_unix_ms": (T + 90 * S) / 1000,
            "source": "login",
            "account": "root",
            "kind": "locked_out",
            "count": 1,
            "acknowledged": false,
        })
    );
    assert_eq!(history[1]["source"], "lock_screen");
    assert_eq!(history[1]["account"], "owner");
    assert_eq!(history[1]["kind"], "wrong_password");
    let json =
        serde_json::to_value(b.view(AlertsState::Starting, LockScreenCoverage::NotConfigured))
            .unwrap();
    assert_eq!(json["state"], "starting");
    assert_eq!(json["lock_screen"], "not_configured");

    // Enum spellings.
    let spell = |v: Value| v.as_str().unwrap().to_string();
    let states: Vec<String> = [
        AlertsState::Disabled,
        AlertsState::Starting,
        AlertsState::Active,
        AlertsState::Unavailable,
    ]
    .iter()
    .map(|s| spell(serde_json::to_value(s).unwrap()))
    .collect();
    assert_eq!(states, ["disabled", "starting", "active", "unavailable"]);
    let reasons: Vec<String> = [
        UnavailableReason::NoJournalAccess,
        UnavailableReason::JournalReaderFailed,
        UnavailableReason::OwnerUnresolved,
        UnavailableReason::Overflow,
        UnavailableReason::RngFailed,
    ]
    .iter()
    .map(|s| spell(serde_json::to_value(s).unwrap()))
    .collect();
    assert_eq!(
        reasons,
        [
            "no_journal_access",
            "journal_reader_failed",
            "owner_unresolved",
            "overflow",
            "rng_failed"
        ]
    );
    let sources: Vec<String> = [C::LockScreen, C::Sudo, C::Login, C::Other]
        .iter()
        .map(|s| spell(serde_json::to_value(s).unwrap()))
        .collect();
    assert_eq!(sources, ["lock_screen", "sudo", "login", "other"]);
    let accounts: Vec<String> = [A::Owner, A::Root, A::Other]
        .iter()
        .map(|s| spell(serde_json::to_value(s).unwrap()))
        .collect();
    assert_eq!(accounts, ["owner", "root", "other"]);
    let kinds: Vec<String> = [K::WrongPassword, K::LockedOut]
        .iter()
        .map(|s| spell(serde_json::to_value(s).unwrap()))
        .collect();
    assert_eq!(kinds, ["wrong_password", "locked_out"]);
    let coverage: Vec<String> = [
        LockScreenCoverage::Monitored,
        LockScreenCoverage::NotConfigured,
    ]
    .iter()
    .map(|s| spell(serde_json::to_value(s).unwrap()))
    .collect();
    assert_eq!(coverage, ["monitored", "not_configured"]);

    // Epoch rendering and parsing.
    assert_eq!(AlertsEpoch::from_u64(0).to_hex(), "0000000000000000");
    assert_eq!(AlertsEpoch::from_u64(0x1234).to_hex(), "0000000000001234");
    assert_eq!(AlertsEpoch::from_u64(u64::MAX).to_hex(), "ffffffffffffffff");
    assert_eq!(
        AlertsEpoch::parse_hex(b"0123456789abcdef"),
        Some(AlertsEpoch::from_u64(0x0123_4567_89ab_cdef))
    );
    for bad in [
        &b"0123456789ABCDEF"[..],
        b"0123456789abcde",
        b"0123456789abcdef0",
        b"",
        b"0123456789abcdeg",
        b" 123456789abcdef",
        b"+123456789abcdef",
    ] {
        assert_eq!(
            AlertsEpoch::parse_hex(bad),
            None,
            "{:?}",
            String::from_utf8_lossy(bad)
        );
    }
    let debug = format!("{:?}", AlertsEpoch::from_u64(0x0123_4567_89ab_cdef));
    assert!(debug.contains("redacted"), "{debug}");
    assert!(!debug.contains("0123456789abcdef") && !debug.contains("81985529216486895"));

    // The disabled view (every count 0, every option null, empty history).
    let disabled = AlertsView {
        state: AlertsState::Disabled,
        reason: None,
        epoch: None,
        lock_screen: None,
        unacknowledged_wrong_password: 0,
        unacknowledged_locked_out: 0,
        last_unix_ms: None,
        last_source: None,
        through: 0,
        history: Vec::new(),
    };
    assert_eq!(
        serde_json::to_value(&disabled).unwrap(),
        json!({
            "state": "disabled",
            "reason": null,
            "epoch": null,
            "lock_screen": null,
            "unacknowledged_wrong_password": 0,
            "unacknowledged_locked_out": 0,
            "last_unix_ms": null,
            "last_source": null,
            "through": 0,
            "history": [],
        })
    );
    // An unavailable view carries its reason.
    let unavailable = AlertsView {
        state: AlertsState::Unavailable,
        reason: Some(UnavailableReason::RngFailed),
        ..disabled.clone()
    };
    let json = serde_json::to_value(&unavailable).unwrap();
    assert_eq!(json["reason"], "rng_failed");
    assert_eq!(keys(&json), top);
    // The skipped record fields never serialize.
    let record = AlertRecord {
        id: 1,
        first_unix_ms: 2,
        last_unix_ms: 3,
        source: C::Sudo,
        account: A::Owner,
        kind: K::WrongPassword,
        count: 1,
        acknowledged: false,
        last_seq: 99,
        first_us: 2000,
        last_us: 3000,
    };
    assert_eq!(keys(&serde_json::to_value(&record).unwrap()), record_keys);
}

// ---------------------------------------------------------------------------------------
// Test 22 — ack file
// ---------------------------------------------------------------------------------------

fn own_uid() -> u32 {
    nix::unistd::getuid().as_raw()
}

fn write_mode(path: &Path, bytes: &[u8], mode: u32) {
    let _ = fs::remove_file(path);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)
        .unwrap();
    file.write_all(bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

/// Test 22 (RMC54, §6.1, A-9): written `0600` atomically with the exact content; read back;
/// absent → `None`; every invalid file (symlink, wrong mode, wrong owner, over 256 bytes, not
/// JSON, `version ≠ 1`, a FIFO) → `Err` (the caller falls back to marker 0, fail-safe).
#[test]
fn test_rmc_alerts_ack_file_round_trip_and_fail_safe() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(ALERTS_ACK_FILE_NAME);
    let uid = own_uid();

    assert_eq!(read_ack_file(&path, uid).ok(), Some(None), "absent");

    write_ack_file(&path, 1_699_990_000_123_456).unwrap();
    let meta = fs::symlink_metadata(&path).unwrap();
    assert!(meta.is_file());
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    let content: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        content,
        json!({"version": 1, "acknowledged_until_us": 1_699_990_000_123_456u64})
    );
    assert!(fs::read(&path).unwrap().len() <= MAX_ALERTS_ACK_FILE_BYTES);
    assert_eq!(
        read_ack_file(&path, uid).ok(),
        Some(Some(1_699_990_000_123_456))
    );
    // Overwrite; extreme values; no temporary file left behind.
    write_ack_file(&path, u64::MAX).unwrap();
    assert_eq!(read_ack_file(&path, uid).ok(), Some(Some(u64::MAX)));
    write_ack_file(&path, 0).unwrap();
    assert_eq!(read_ack_file(&path, uid).ok(), Some(Some(0)));
    let names: Vec<String> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec![ALERTS_ACK_FILE_NAME.to_string()]);
    // The write replaces a symlink planted at the path instead of following it.
    let victim = dir.path().join("victim");
    fs::write(&victim, b"keep").unwrap();
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&victim, &path).unwrap();
    let _ = write_ack_file(&path, 5);
    assert_eq!(
        fs::read(&victim).unwrap(),
        b"keep",
        "never written through a symlink"
    );
    fs::remove_file(&path).ok();
    fs::remove_file(&victim).ok();
    // A missing parent directory is an error, never a panic.
    assert!(write_ack_file(&dir.path().join("missing").join("f.json"), 1).is_err());

    let valid = br#"{"version":1,"acknowledged_until_us":42}"#;
    write_mode(&path, valid, 0o600);
    assert_eq!(read_ack_file(&path, uid).ok(), Some(Some(42)));
    // Exactly 256 bytes (padded with whitespace) is accepted; 257 is not.
    let mut padded = valid.to_vec();
    padded.resize(MAX_ALERTS_ACK_FILE_BYTES, b' ');
    write_mode(&path, &padded, 0o600);
    assert_eq!(read_ack_file(&path, uid).ok(), Some(Some(42)));
    padded.push(b' ');
    write_mode(&path, &padded, 0o600);
    assert!(read_ack_file(&path, uid).is_err(), "257 bytes");

    for (what, bytes) in [
        ("not JSON", &b"acknowledged"[..]),
        ("empty", b""),
        ("version 2", br#"{"version":2,"acknowledged_until_us":42}"#),
        ("no version", br#"{"acknowledged_until_us":42}"#),
        ("no marker", br#"{"version":1}"#),
        ("negative", br#"{"version":1,"acknowledged_until_us":-1}"#),
        (
            "string marker",
            br#"{"version":1,"acknowledged_until_us":"42"}"#,
        ),
        (
            "over u64",
            br#"{"version":1,"acknowledged_until_us":18446744073709551616}"#,
        ),
        ("array", b"[1,42]"),
    ] {
        write_mode(&path, bytes, 0o600);
        assert!(read_ack_file(&path, uid).is_err(), "{what}");
    }
    // Wrong mode.
    for mode in [0o644, 0o640, 0o400, 0o700] {
        write_mode(&path, valid, mode);
        assert!(read_ack_file(&path, uid).is_err(), "mode {mode:o}");
    }
    // Wrong owner (the file is ours, another uid is expected).
    write_mode(&path, valid, 0o600);
    assert!(read_ack_file(&path, uid.wrapping_add(1)).is_err());
    // A symlink to a valid file.
    let target = dir.path().join("target.json");
    write_mode(&target, valid, 0o600);
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    assert!(read_ack_file(&path, uid).is_err(), "symlink");
    fs::remove_file(&path).unwrap();
    // A FIFO never blocks the read.
    nix::unistd::mkfifo(&path, nix::sys::stat::Mode::from_bits_truncate(0o600)).unwrap();
    assert!(read_ack_file(&path, uid).is_err(), "FIFO");
    fs::remove_file(&path).unwrap();
    // A directory.
    fs::create_dir(&path).unwrap();
    assert!(read_ack_file(&path, uid).is_err(), "directory");
}

// ---------------------------------------------------------------------------------------
// Helpers used only for documentation of the signal / attempt types
// ---------------------------------------------------------------------------------------

/// The pure types carry no text: `Signal` and `Attempt` are `Copy`.
#[test]
fn test_rmc_alerts_signal_and_attempt_are_plain_values() {
    fn assert_copy<X: Copy>() {}
    assert_copy::<Signal>();
    assert_copy::<Attempt>();
    assert_copy::<AlertsEpoch>();
}
