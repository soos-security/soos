//! Contract tests of GitHub #323 for the pure, clockless presence lock tracker and candidate
//! selection (matrix PAU5, PAU6, PAU7, PAU14 tracker bound, PAU15, PAU27 session IDs).

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

use std::time::Duration;

use common::{bound_record, locked_session, session, sid, user, MS_NS, SECOND_NS};
use soos_daemon::presence::logind::{LogindSessionState, SessionId};
use soos_daemon::presence::tracker::{select_candidate, LockEntry, LockTracker};
use soos_daemon::presence::worker::SkipReason;
use soos_daemon::presence::{MAX_TRACKED_LOCKED_SESSIONS, UNLOCK_CONFIRM_TIMEOUT_MS};
use soos_daemon::session_policy::{SessionRecord, MAX_SESSION_ID_LEN};

const GRACE: Duration = Duration::from_millis(3000);
const INTERVAL: Duration = Duration::from_millis(2000);
const T0: u64 = 100 * SECOND_NS;

fn grace_ns() -> u64 {
    u64::try_from(GRACE.as_nanos()).unwrap()
}

fn due_ids(tracker: &LockTracker, now: u64) -> Vec<SessionId> {
    tracker
        .due(now, GRACE, INTERVAL)
        .into_iter()
        .map(|(id, _)| id)
        .collect()
}

fn with_record(id: &str, record: SessionRecord, locked: bool) -> LogindSessionState {
    LogindSessionState {
        id: sid(id),
        record,
        locked,
        user_name: Some(user("alice")),
    }
}

// ---------------------------------------------------------------------------------------
// Session IDs
// ---------------------------------------------------------------------------------------

/// Invalid logind session IDs are never representable (never sent to logind).
#[test]
fn test_pau_session_id_parse_accepts_only_logind_ids() {
    for valid in ["1", "4", "c2", "C17", &"a".repeat(MAX_SESSION_ID_LEN)] {
        let id = SessionId::parse(valid).unwrap_or_else(|| panic!("`{valid}` must parse"));
        assert_eq!(id.as_str(), valid);
    }
    for invalid in [
        "",
        " 2",
        "2 ",
        "a/b",
        "../2",
        "2\n",
        "c-2",
        "c_2",
        "é",
        &"a".repeat(MAX_SESSION_ID_LEN + 1),
    ] {
        assert!(
            SessionId::parse(invalid).is_none(),
            "`{invalid:?}` must be refused"
        );
    }
}

// ---------------------------------------------------------------------------------------
// PAU5 — binding predicate
// ---------------------------------------------------------------------------------------

/// PAU5: only bound (`check_local_seat_session_of(uid)`) **and** locked sessions are
/// tracked; every other case is never tracked, hence never scanned.
#[test]
fn test_pau_only_bound_and_locked_sessions_are_tracked() {
    let base = bound_record(1000);
    let cases: Vec<(&str, LogindSessionState)> = vec![
        (
            "remote",
            with_record(
                "10",
                SessionRecord {
                    remote: Some(true),
                    ..base.clone()
                },
                true,
            ),
        ),
        (
            "unknown remote (property missing or ill-typed)",
            with_record(
                "11",
                SessionRecord {
                    remote: None,
                    ..base.clone()
                },
                true,
            ),
        ),
        (
            "seatless",
            with_record(
                "12",
                SessionRecord {
                    seat: None,
                    ..base.clone()
                },
                true,
            ),
        ),
        (
            "inactive",
            with_record(
                "13",
                SessionRecord {
                    active: false,
                    ..base.clone()
                },
                true,
            ),
        ),
        (
            "greeter class",
            with_record(
                "14",
                SessionRecord {
                    class: Some("greeter".into()),
                    ..base.clone()
                },
                true,
            ),
        ),
        (
            "manager class",
            with_record(
                "15",
                SessionRecord {
                    class: Some("manager".into()),
                    ..base.clone()
                },
                true,
            ),
        ),
        (
            "no class",
            with_record(
                "16",
                SessionRecord {
                    class: None,
                    ..base.clone()
                },
                true,
            ),
        ),
        (
            "no uid",
            with_record(
                "17",
                SessionRecord {
                    uid: None,
                    ..base.clone()
                },
                true,
            ),
        ),
        ("unlocked", with_record("18", base.clone(), false)),
    ];
    for (label, state) in cases {
        let mut tracker = LockTracker::default();
        tracker.observe(T0, std::slice::from_ref(&state)).unwrap();
        assert!(
            tracker.get(&state.id).is_none(),
            "{label}: must never be tracked"
        );
        assert!(
            due_ids(&tracker, T0 + 10 * grace_ns()).is_empty(),
            "{label}: must never become due"
        );
    }

    let mut tracker = LockTracker::default();
    let eligible = locked_session("20", 1000, "alice");
    tracker
        .observe(T0, std::slice::from_ref(&eligible))
        .unwrap();
    let entry = tracker
        .get(&eligible.id)
        .expect("a bound, locked session is tracked");
    assert_eq!(entry.uid, 1000);
    assert_eq!(entry.locked_since_ns, T0);
    assert!(!entry.locker_ignored);
    assert_eq!(entry.last_scan_started_ns, None);
    assert_eq!(entry.unlock_requested_ns, None);
}

// ---------------------------------------------------------------------------------------
// PAU6 — grace
// ---------------------------------------------------------------------------------------

/// PAU6: `locked_since + grace - 1 ns` is not due; exactly `grace` is due (`>=`).
#[test]
fn test_pau_grace_boundary_is_inclusive() {
    let mut tracker = LockTracker::default();
    let s = locked_session("2", 1000, "alice");
    tracker.observe(T0, std::slice::from_ref(&s)).unwrap();
    assert!(due_ids(&tracker, T0).is_empty());
    assert!(
        due_ids(&tracker, T0 + grace_ns() - 1).is_empty(),
        "one nanosecond before the grace elapses nothing is due"
    );
    assert_eq!(
        due_ids(&tracker, T0 + grace_ns()),
        vec![s.id.clone()],
        "the grace boundary itself is eligible"
    );
}

/// PAU6: re-observing a still-locked session keeps its lock period (grace not restarted).
#[test]
fn test_pau_still_locked_session_keeps_its_lock_period() {
    let mut tracker = LockTracker::default();
    let s = locked_session("2", 1000, "alice");
    tracker.observe(T0, std::slice::from_ref(&s)).unwrap();
    let epoch = tracker.get(&s.id).unwrap().epoch;
    for k in 1..5u64 {
        tracker
            .observe(T0 + k * SECOND_NS, std::slice::from_ref(&s))
            .unwrap();
    }
    let entry = tracker.get(&s.id).unwrap();
    assert_eq!(entry.locked_since_ns, T0);
    assert_eq!(entry.epoch, epoch);
    assert_eq!(due_ids(&tracker, T0 + grace_ns()), vec![s.id.clone()]);
}

/// PAU6: unlocked, absent or inactive, then locked again ⇒ grace restarts with a new epoch.
#[test]
fn test_pau_relock_after_unlock_absence_or_inactivity_restarts_grace() {
    let s = locked_session("2", 1000, "alice");
    let unlocked = session("2", 1000, "alice", false);
    let inactive = with_record(
        "2",
        SessionRecord {
            active: false,
            ..bound_record(1000)
        },
        true,
    );
    let interruptions: Vec<(&str, Vec<LogindSessionState>)> = vec![
        ("unlocked", vec![unlocked]),
        ("absent", vec![]),
        ("inactive", vec![inactive]),
    ];
    for (label, interruption) in interruptions {
        let mut tracker = LockTracker::default();
        tracker.observe(T0, std::slice::from_ref(&s)).unwrap();
        let first_epoch = tracker.get(&s.id).unwrap().epoch;
        let t1 = T0 + 10 * SECOND_NS;
        tracker.observe(t1, &interruption).unwrap();
        assert!(
            tracker.get(&s.id).is_none(),
            "{label}: the lock period ends"
        );
        let t2 = t1 + SECOND_NS;
        tracker.observe(t2, std::slice::from_ref(&s)).unwrap();
        let entry = tracker.get(&s.id).unwrap();
        assert_eq!(
            entry.locked_since_ns, t2,
            "{label}: a new lock period starts"
        );
        assert!(
            entry.epoch > first_epoch,
            "{label}: a fresh epoch is issued"
        );
        assert!(
            due_ids(&tracker, t2 + grace_ns() - 1).is_empty(),
            "{label}: the grace restarts"
        );
        assert_eq!(due_ids(&tracker, t2 + grace_ns()), vec![s.id.clone()]);
    }
}

/// PAU6: a session whose owner UID changed is a new lock period.
#[test]
fn test_pau_uid_change_is_a_new_lock_period() {
    let mut tracker = LockTracker::default();
    tracker
        .observe(T0, &[locked_session("2", 1000, "alice")])
        .unwrap();
    let first_epoch = tracker.get(&sid("2")).unwrap().epoch;
    let t1 = T0 + 10 * SECOND_NS;
    tracker
        .observe(t1, &[locked_session("2", 1001, "bob")])
        .unwrap();
    let entry = tracker.get(&sid("2")).unwrap();
    assert_eq!(entry.uid, 1001);
    assert_eq!(entry.locked_since_ns, t1);
    assert!(entry.epoch > first_epoch);
    assert!(due_ids(&tracker, t1 + grace_ns() - 1).is_empty());
}

/// PAU6: after a presence unlock, the next lock period starts a fresh grace.
#[test]
fn test_pau_lock_after_presence_unlock_starts_a_fresh_grace() {
    let mut tracker = LockTracker::default();
    let s = locked_session("2", 1000, "alice");
    tracker.observe(T0, std::slice::from_ref(&s)).unwrap();
    let t_scan = T0 + grace_ns();
    tracker.mark_scan_started(&s.id, t_scan);
    tracker.mark_unlock_requested(&s.id, t_scan + 300 * MS_NS);
    let t_unlocked = t_scan + SECOND_NS;
    tracker
        .observe(t_unlocked, &[session("2", 1000, "alice", false)])
        .unwrap();
    assert!(tracker.get(&s.id).is_none());
    let t_relock = t_unlocked + 30 * SECOND_NS;
    tracker.observe(t_relock, std::slice::from_ref(&s)).unwrap();
    let entry = tracker.get(&s.id).unwrap();
    assert_eq!(entry.locked_since_ns, t_relock);
    assert_eq!(entry.unlock_requested_ns, None);
    assert_eq!(entry.last_scan_started_ns, None);
    assert!(due_ids(&tracker, t_relock + grace_ns() - 1).is_empty());
    assert_eq!(due_ids(&tracker, t_relock + grace_ns()), vec![s.id.clone()]);
}

/// Scan interval: a session scanned less than `scan_interval` ago is not due.
#[test]
fn test_pau_scan_interval_spaces_scans_of_one_session() {
    let mut tracker = LockTracker::default();
    let s = locked_session("2", 1000, "alice");
    tracker.observe(T0, std::slice::from_ref(&s)).unwrap();
    let t_scan = T0 + grace_ns();
    tracker.mark_scan_started(&s.id, t_scan);
    assert_eq!(
        tracker.get(&s.id).unwrap().last_scan_started_ns,
        Some(t_scan)
    );
    let interval_ns = u64::try_from(INTERVAL.as_nanos()).unwrap();
    assert!(due_ids(&tracker, t_scan + interval_ns - 1).is_empty());
    assert_eq!(due_ids(&tracker, t_scan + interval_ns), vec![s.id.clone()]);
}

/// Saturating arithmetic: a clock reading before `locked_since` is never due.
#[test]
fn test_pau_due_is_saturating_on_a_clock_going_backwards() {
    let mut tracker = LockTracker::default();
    tracker
        .observe(T0, &[locked_session("2", 1000, "alice")])
        .unwrap();
    assert!(due_ids(&tracker, 0).is_empty());
    assert!(due_ids(&tracker, T0 - 1).is_empty());
}

// ---------------------------------------------------------------------------------------
// PAU14 — tracker bound
// ---------------------------------------------------------------------------------------

/// PAU14: more than `MAX_TRACKED_LOCKED_SESSIONS` ⇒ `TrackerOverflow`, tracker cleared.
#[test]
fn test_pau_tracker_overflow_clears_and_fails_closed() {
    let mut tracker = LockTracker::default();
    let first = locked_session("1", 1000, "alice");
    tracker.observe(T0, std::slice::from_ref(&first)).unwrap();
    let many: Vec<LogindSessionState> = (0..=MAX_TRACKED_LOCKED_SESSIONS)
        .map(|i| locked_session(&format!("{}", 100 + i), 1000 + i as u32, "alice"))
        .collect();
    assert_eq!(many.len(), MAX_TRACKED_LOCKED_SESSIONS + 1);
    assert_eq!(
        tracker.observe(T0 + grace_ns(), &many),
        Err(SkipReason::TrackerOverflow)
    );
    assert!(tracker.get(&first.id).is_none(), "the tracker is cleared");
    for s in &many {
        assert!(tracker.get(&s.id).is_none());
    }
    assert!(due_ids(&tracker, T0 + 100 * grace_ns()).is_empty());

    // Exactly the bound is accepted.
    let mut tracker = LockTracker::default();
    tracker
        .observe(T0, &many[..MAX_TRACKED_LOCKED_SESSIONS])
        .unwrap();
    assert_eq!(
        due_ids(&tracker, T0 + grace_ns()).len(),
        MAX_TRACKED_LOCKED_SESSIONS
    );
}

// ---------------------------------------------------------------------------------------
// PAU15 — unconfirmed unlock
// ---------------------------------------------------------------------------------------

/// PAU15: still locked `UNLOCK_CONFIRM_TIMEOUT_MS` after the unlock call ⇒ `locker_ignored`
/// (reported once), never due again until observed unlocked and re-locked.
#[test]
fn test_pau_unconfirmed_unlock_marks_the_locker_ignored() {
    let mut tracker = LockTracker::default();
    let s = locked_session("2", 1000, "alice");
    tracker.observe(T0, std::slice::from_ref(&s)).unwrap();
    let t_unlock = T0 + grace_ns() + 500 * MS_NS;
    tracker.mark_scan_started(&s.id, T0 + grace_ns());
    tracker.mark_unlock_requested(&s.id, t_unlock);
    assert_eq!(
        tracker.get(&s.id).unwrap().unlock_requested_ns,
        Some(t_unlock)
    );
    let confirm_ns = UNLOCK_CONFIRM_TIMEOUT_MS * MS_NS;

    tracker
        .observe(t_unlock + confirm_ns - 1, std::slice::from_ref(&s))
        .unwrap();
    assert!(
        tracker
            .expire_unconfirmed_unlocks(t_unlock + confirm_ns - 1)
            .is_empty(),
        "not expired before the confirmation timeout"
    );
    assert!(!tracker.get(&s.id).unwrap().locker_ignored);

    let t_expired = t_unlock + confirm_ns + 1;
    tracker
        .observe(t_expired, std::slice::from_ref(&s))
        .unwrap();
    assert_eq!(
        tracker.expire_unconfirmed_unlocks(t_expired),
        vec![s.id.clone()]
    );
    assert!(tracker.get(&s.id).unwrap().locker_ignored);
    assert!(
        tracker
            .expire_unconfirmed_unlocks(t_expired + SECOND_NS)
            .is_empty(),
        "reported once (one warn)"
    );
    for k in 0..10u64 {
        let now = t_expired + k * 10 * SECOND_NS;
        tracker.observe(now, std::slice::from_ref(&s)).unwrap();
        assert!(
            due_ids(&tracker, now).is_empty(),
            "a locker_ignored session is never scanned again"
        );
    }

    // Observed unlocked, then re-locked: eligible again after a fresh grace.
    let t_unlocked = t_expired + 200 * SECOND_NS;
    tracker
        .observe(t_unlocked, &[session("2", 1000, "alice", false)])
        .unwrap();
    let t_relock = t_unlocked + SECOND_NS;
    tracker.observe(t_relock, std::slice::from_ref(&s)).unwrap();
    let entry = tracker.get(&s.id).unwrap();
    assert!(!entry.locker_ignored);
    assert_eq!(entry.unlock_requested_ns, None);
    assert_eq!(due_ids(&tracker, t_relock + grace_ns()), vec![s.id.clone()]);
}

/// PAU15: a session without an unlock request never becomes `locker_ignored`.
#[test]
fn test_pau_no_unlock_request_never_expires() {
    let mut tracker = LockTracker::default();
    let s = locked_session("2", 1000, "alice");
    tracker.observe(T0, std::slice::from_ref(&s)).unwrap();
    assert!(tracker
        .expire_unconfirmed_unlocks(T0 + 1000 * SECOND_NS)
        .is_empty());
    assert!(!tracker.get(&s.id).unwrap().locker_ignored);
}

// ---------------------------------------------------------------------------------------
// PAU7 — exactly one candidate
// ---------------------------------------------------------------------------------------

fn entry(uid: u32) -> LockEntry {
    LockEntry {
        uid,
        locked_since_ns: T0,
        epoch: 1,
        last_scan_started_ns: None,
        unlock_requested_ns: None,
        locker_ignored: false,
    }
}

/// PAU7: zero ⇒ `NoCandidate`, one ⇒ that one, two or more ⇒ `AmbiguousCandidates`.
#[test]
fn test_pau_select_candidate_requires_exactly_one() {
    assert_eq!(select_candidate(Vec::new()), Err(SkipReason::NoCandidate));
    assert_eq!(
        select_candidate(vec![(sid("2"), entry(1000))]),
        Ok((sid("2"), entry(1000)))
    );
    assert_eq!(
        select_candidate(vec![(sid("2"), entry(1000)), (sid("3"), entry(1001))]),
        Err(SkipReason::AmbiguousCandidates)
    );
    assert_eq!(
        select_candidate(vec![
            (sid("2"), entry(1000)),
            (sid("3"), entry(1000)),
            (sid("4"), entry(1002)),
        ]),
        Err(SkipReason::AmbiguousCandidates),
        "two sessions of the same UID are ambiguous too"
    );
}
