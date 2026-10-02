//! Pure, clockless lock-period tracker and candidate selection (GitHub #323).
//!
//! Every timestamp is a monotonic nanosecond reading passed in by the caller; nothing here
//! reads a clock or performs I/O.

use std::collections::BTreeMap;
use std::time::Duration;

use super::logind::{LogindSessionState, SessionId};
use super::worker::SkipReason;
use super::{MAX_TRACKED_LOCKED_SESSIONS, UNLOCK_CONFIRM_TIMEOUT_MS};

/// One observed lock period of a bound session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockEntry {
    /// Session owner UID.
    pub uid: u32,
    /// Monotonic ns of the first tick that observed this lock period.
    pub locked_since_ns: u64,
    /// Incremented each time a session is (re)observed transitioning to locked.
    pub epoch: u64,
    /// Start of the last scan of this lock period.
    pub last_scan_started_ns: Option<u64>,
    /// Time of the last successful `UnlockSession` call of this lock period.
    pub unlock_requested_ns: Option<u64>,
    /// The locker ignored an unlock request: never scanned again in this lock period.
    pub locker_ignored: bool,
}

/// Converts a duration to nanoseconds, saturating.
fn duration_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

/// Tracker of the bound, locked sessions (bounded by [`MAX_TRACKED_LOCKED_SESSIONS`]).
#[derive(Debug, Default)]
pub struct LockTracker {
    entries: BTreeMap<SessionId, LockEntry>,
    next_epoch: u64,
}

/// Owner UID of a session that is bound (`check_local_seat_session_of`) and locked.
fn bound_locked_uid(state: &LogindSessionState) -> Option<u32> {
    let uid = state.record.uid?;
    if state.locked && state.record.check_local_seat_session_of(uid).is_ok() {
        Some(uid)
    } else {
        None
    }
}

impl LockTracker {
    /// Folds one snapshot in.
    ///
    /// Bound **and** locked sessions are inserted (new lock period) or kept; every other
    /// session, and every tracked session absent from the snapshot, is removed. A session
    /// whose UID changed starts a new lock period.
    ///
    /// # Errors
    ///
    /// [`SkipReason::TrackerOverflow`] when more than [`MAX_TRACKED_LOCKED_SESSIONS`] would
    /// be tracked; the tracker is cleared first (fail closed: every grace restarts).
    pub fn observe(
        &mut self,
        now_ns: u64,
        snapshot: &[LogindSessionState],
    ) -> Result<(), SkipReason> {
        let mut next = BTreeMap::new();
        for state in snapshot {
            let Some(uid) = bound_locked_uid(state) else {
                continue;
            };
            let kept = self
                .entries
                .get(&state.id)
                .filter(|entry| entry.uid == uid)
                .cloned();
            let entry = if let Some(entry) = kept {
                entry
            } else {
                self.next_epoch = self.next_epoch.saturating_add(1);
                LockEntry {
                    uid,
                    locked_since_ns: now_ns,
                    epoch: self.next_epoch,
                    last_scan_started_ns: None,
                    unlock_requested_ns: None,
                    locker_ignored: false,
                }
            };
            next.insert(state.id.clone(), entry);
            if next.len() > MAX_TRACKED_LOCKED_SESSIONS {
                self.clear();
                return Err(SkipReason::TrackerOverflow);
            }
        }
        self.entries = next;
        Ok(())
    }

    /// Removes every entry (kill switch, logind failure, overflow).
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Whether nothing is tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether some scannable entry is still inside its grace at `now_ns`.
    #[must_use]
    pub fn any_in_grace(&self, now_ns: u64, lock_grace: Duration) -> bool {
        let grace = duration_ns(lock_grace);
        self.entries.values().any(|entry| {
            !entry.locker_ignored
                && entry.unlock_requested_ns.is_none()
                && now_ns.saturating_sub(entry.locked_since_ns) < grace
        })
    }

    /// Sessions whose grace has elapsed (inclusive boundary, saturating), that are neither
    /// `locker_ignored` nor waiting for an unlock confirmation, and whose last scan started
    /// at least `scan_interval` ago (or never).
    #[must_use]
    pub fn due(
        &self,
        now_ns: u64,
        lock_grace: Duration,
        scan_interval: Duration,
    ) -> Vec<(SessionId, LockEntry)> {
        let grace = duration_ns(lock_grace);
        let interval = duration_ns(scan_interval);
        self.entries
            .iter()
            .filter(|(_, entry)| {
                !entry.locker_ignored
                    && entry.unlock_requested_ns.is_none()
                    && now_ns >= entry.locked_since_ns
                    && now_ns.saturating_sub(entry.locked_since_ns) >= grace
                    && entry.last_scan_started_ns.is_none_or(|started| {
                        now_ns >= started && now_ns.saturating_sub(started) >= interval
                    })
            })
            .map(|(id, entry)| (id.clone(), entry.clone()))
            .collect()
    }

    /// Records the start of a scan.
    pub fn mark_scan_started(&mut self, id: &SessionId, now_ns: u64) {
        if let Some(entry) = self.entries.get_mut(id) {
            entry.last_scan_started_ns = Some(now_ns);
        }
    }

    /// Records a successful `UnlockSession` call.
    pub fn mark_unlock_requested(&mut self, id: &SessionId, now_ns: u64) {
        if let Some(entry) = self.entries.get_mut(id) {
            entry.unlock_requested_ns = Some(now_ns);
        }
    }

    /// Marks entries still locked [`UNLOCK_CONFIRM_TIMEOUT_MS`] after their unlock request
    /// as `locker_ignored`; returns the newly marked IDs (one warning each).
    pub fn expire_unconfirmed_unlocks(&mut self, now_ns: u64) -> Vec<SessionId> {
        let timeout_ns = UNLOCK_CONFIRM_TIMEOUT_MS.saturating_mul(1_000_000);
        let mut expired = Vec::new();
        for (id, entry) in &mut self.entries {
            if entry.locker_ignored {
                continue;
            }
            if let Some(requested) = entry.unlock_requested_ns {
                if now_ns.saturating_sub(requested) >= timeout_ns {
                    entry.locker_ignored = true;
                    expired.push(id.clone());
                }
            }
        }
        expired
    }

    /// Entry of one session.
    #[must_use]
    pub fn get(&self, id: &SessionId) -> Option<&LockEntry> {
        self.entries.get(id)
    }
}

/// Exactly-one rule (D5) over the due sessions already filtered for enrollment.
///
/// # Errors
///
/// [`SkipReason::NoCandidate`] for none, [`SkipReason::AmbiguousCandidates`] for two or more.
pub fn select_candidate(
    due: Vec<(SessionId, LockEntry)>,
) -> Result<(SessionId, LockEntry), SkipReason> {
    let mut iter = due.into_iter();
    match (iter.next(), iter.next()) {
        (None, _) => Err(SkipReason::NoCandidate),
        (Some(only), None) => Ok(only),
        (Some(_), Some(_)) => Err(SkipReason::AmbiguousCandidates),
    }
}
