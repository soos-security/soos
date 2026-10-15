//! Lock-screen state snapshot (GitHub #380, PR 1 of #352; ADR 2026-10-13 "Read-Only
//! Lock-Screen State Channel").
//!
//! The PAM `Auth` path and the presence worker publish the coarse progress of each face scan
//! into a [`LockStateBoard`]; `RequestKind::LockState` reads it. Only three states exist
//! (`Idle`, `Scanning`, `NotRecognized`): every failure reason collapses into `NotRecognized`
//! and a success reads `Idle` (the session unlocking is the success signal).
//!
//! Uniform reveal rule: the end of a scan, whatever it is, becomes visible only at
//! `max(end, start + NOT_RECOGNIZED_REVEAL_FLOOR_MS)`. Before that the scan reads `Scanning`
//! with the sequence of its begin, so an early end (a presentation-attack veto stops at the
//! first capture, a non-match only at the decision budget) is not observable at the locked
//! screen, and a superseding or preempting scan advances the sequence identically whether the
//! earlier scan had already ended or not.
//!
//! The board holds per UID only counters and monotonic stamps: nothing biometric, no reason,
//! no identifier other than the numeric UID. It is clockless (callers pass `now_ns`), never
//! performs I/O and never logs.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::sync::Arc;

use tokio::sync::watch;

use crate::consensus::MAX_CAMERA_WAKE_WAIT_MS;
use crate::inference::{DEFAULT_INFERENCE_ESTIMATE_MS, RESPONSE_WRITE_MARGIN_MS};
use crate::pipeline::{DECISION_BUDGET_MS, FRAME_POLL_INTERVAL_MS};
use crate::presence::PRESENCE_WAKE_SETTLE_MS;
use soos_protocol::types::LockWidgetState;

/// UIDs held by the board; a new UID beyond it evicts the least recently active one.
pub const MAX_LOCK_STATE_UIDS: usize = 32;

/// Slack above the nominal longest non-matching scan: an inference admitted just inside the
/// decision budget may finish up to one nominal inference (`DEFAULT_INFERENCE_ESTIMATE_MS`) after
/// its admission estimate, and the consensus loop notices the end within one frame poll
/// (`FRAME_POLL_INTERVAL_MS`): 90 ms.
pub const REVEAL_FLOOR_MARGIN_MS: u64 =
    DEFAULT_INFERENCE_ESTIMATE_MS.saturating_add(FRAME_POLL_INTERVAL_MS);

/// Earliest visibility of a scan end after the scan start: the longest regular non-matching
/// presence scan (camera wake + settle + decision budget + response write margin) plus
/// [`REVEAL_FLOOR_MARGIN_MS`], 3240 ms. The presence worker also holds its next scan until
/// this instant after every scan that did not unlock, so the next scan's start does not
/// depend on the previous end either.
pub const NOT_RECOGNIZED_REVEAL_FLOOR_MS: u64 = MAX_CAMERA_WAKE_WAIT_MS
    .saturating_add(PRESENCE_WAKE_SETTLE_MS)
    .saturating_add(DECISION_BUDGET_MS)
    .saturating_add(RESPONSE_WRITE_MARGIN_MS)
    .saturating_add(REVEAL_FLOOR_MARGIN_MS);

/// How long a revealed `NotRecognized` stays visible before it reads `Idle`.
pub const NOT_RECOGNIZED_HOLD_MS: u64 = 3000;

/// A scan still running after this long reads `Idle` (defence in depth against a lost end).
pub const MAX_SCANNING_MS: u64 = 10_000;

/// Per-peer-UID `LockState` requests per window (the bridge polls at most 4 Hz).
pub const LOCK_STATE_MAX_REQUESTS_PER_SEC: u32 = 8;

/// Window of the `LockState` quota (1 s).
pub const LOCK_STATE_RATE_WINDOW_NS: u64 = 1_000_000_000;

/// Peer UIDs tracked by the `LockState` quota (least recently used evicted beyond it).
pub const LOCK_STATE_MAX_TRACKED_UIDS: usize = 64;

/// Bound of the logind sessions-directory scan of one `LockState` request, run on the blocking
/// pool; expiry (or a failed job) answers the neutral view.
pub const LOCK_STATE_SESSION_CHECK_TIMEOUT_MS: u64 = 250;

const NANOS_PER_MS: u64 = 1_000_000;
/// [`NOT_RECOGNIZED_REVEAL_FLOOR_MS`] in nanoseconds.
pub const REVEAL_FLOOR_NS: u64 = NOT_RECOGNIZED_REVEAL_FLOOR_MS.saturating_mul(NANOS_PER_MS);
const HOLD_NS: u64 = NOT_RECOGNIZED_HOLD_MS.saturating_mul(NANOS_PER_MS);
const MAX_SCANNING_NS: u64 = MAX_SCANNING_MS.saturating_mul(NANOS_PER_MS);

/// How a published scan ended (daemon-internal; never on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanEnd {
    /// The face was verified: reads `Idle` once revealed.
    Recognized,
    /// Any failure after the scan started: reads `NotRecognized` once revealed.
    NotRecognized,
}

/// What a reader sees for one UID at a given instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LockStateView {
    /// Collapsed state.
    pub state: LockWidgetState,
    /// Per-UID visible transition counter (`0`: nothing ever published for the UID).
    pub sequence: u64,
}

impl LockStateView {
    /// `Idle` with sequence `0`: the neutral answer that carries no information.
    #[must_use]
    pub const fn neutral() -> Self {
        Self {
            state: LockWidgetState::Idle,
            sequence: 0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct End {
    state: LockWidgetState,
    at_ns: u64,
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    scans: u64,
    scan_id: u64,
    started_ns: u64,
    end: Option<End>,
}

impl Entry {
    fn last_activity_ns(&self) -> u64 {
        self.end
            .map_or(self.started_ns, |end| end.at_ns.max(self.started_ns))
    }

    fn project(&self, now_ns: u64) -> LockStateView {
        let hidden_sequence = self.scans.saturating_mul(2).saturating_sub(1);
        let floor_ns = self.started_ns.saturating_add(REVEAL_FLOOR_NS);
        if let Some(end) = self.end {
            let reveal_ns = end.at_ns.max(floor_ns);
            if now_ns >= reveal_ns {
                let state = if end.state == LockWidgetState::NotRecognized
                    && now_ns >= reveal_ns.saturating_add(HOLD_NS)
                {
                    LockWidgetState::Idle
                } else {
                    end.state
                };
                return LockStateView {
                    state,
                    sequence: self.scans.saturating_mul(2),
                };
            }
        }
        let state = if now_ns.saturating_sub(self.started_ns) > MAX_SCANNING_NS {
            LockWidgetState::Idle
        } else {
            LockWidgetState::Scanning
        };
        LockStateView {
            state,
            sequence: hidden_sequence,
        }
    }
}

#[derive(Debug, Default)]
struct Board {
    entries: BTreeMap<u32, Entry>,
    next_scan_id: u64,
}

/// Shared, bounded lock-state snapshot (clones share one board).
#[derive(Debug, Clone)]
pub struct LockStateBoard {
    tx: Arc<watch::Sender<Board>>,
}

impl Default for LockStateBoard {
    fn default() -> Self {
        Self::new()
    }
}

impl LockStateBoard {
    /// An empty board.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tx: Arc::new(watch::Sender::new(Board::default())),
        }
    }

    /// Publishes `Scanning` for `uid`: a new scan replaces the current one of the UID.
    pub fn begin_scan(&self, uid: u32, now_ns: u64) -> ScanGuard {
        let mut scan_id = 0;
        self.tx.send_modify(|board| {
            board.next_scan_id = board.next_scan_id.saturating_add(1);
            scan_id = board.next_scan_id;
            if !board.entries.contains_key(&uid) && board.entries.len() >= MAX_LOCK_STATE_UIDS {
                let oldest = board
                    .entries
                    .iter()
                    .min_by_key(|(key, entry)| (entry.last_activity_ns(), **key))
                    .map(|(key, _)| *key);
                if let Some(oldest) = oldest {
                    board.entries.remove(&oldest);
                }
            }
            let entry = board.entries.entry(uid).or_insert(Entry {
                scans: 0,
                scan_id,
                started_ns: now_ns,
                end: None,
            });
            entry.scans = entry.scans.saturating_add(1);
            entry.scan_id = scan_id;
            entry.started_ns = now_ns;
            entry.end = None;
        });
        ScanGuard {
            board: self.clone(),
            uid,
            scan_id,
            settled: false,
        }
    }

    /// What `uid` reads at `now_ns`. Pure: never changes the board.
    #[must_use]
    pub fn view(&self, uid: u32, now_ns: u64) -> LockStateView {
        let entry = self.tx.borrow().entries.get(&uid).copied();
        entry.map_or_else(LockStateView::neutral, |entry| entry.project(now_ns))
    }

    /// Number of UIDs currently held (at most [`MAX_LOCK_STATE_UIDS`]).
    #[must_use]
    pub fn tracked_uids(&self) -> usize {
        self.tx.borrow().entries.len()
    }

    /// Records the end of scan `scan_id` of `uid`, unless a later scan superseded it.
    fn settle(&self, uid: u32, scan_id: u64, state: LockWidgetState, at_ns: Option<u64>) {
        self.tx.send_if_modified(|board| {
            let Some(entry) = board.entries.get_mut(&uid) else {
                return false;
            };
            if entry.scan_id != scan_id || entry.end.is_some() {
                return false;
            }
            entry.end = Some(End {
                state,
                at_ns: at_ns.unwrap_or(entry.started_ns),
            });
            true
        });
    }
}

/// One published scan; settle it with [`ScanGuard::finish`]. Dropping it unsettled publishes
/// an abandoned end (`Idle`), which never claims anything.
#[must_use = "a dropped scan guard publishes an abandoned scan"]
#[derive(Debug)]
pub struct ScanGuard {
    board: LockStateBoard,
    uid: u32,
    scan_id: u64,
    settled: bool,
}

impl ScanGuard {
    /// Publishes the end of the scan: `Recognized` reads `Idle`, `NotRecognized` reads
    /// `NotRecognized`; without a clock reading (`None`) the end reads `Idle`.
    pub fn finish(mut self, end: ScanEnd, now_ns: Option<u64>) {
        let state = match (end, now_ns) {
            (ScanEnd::NotRecognized, Some(_)) => LockWidgetState::NotRecognized,
            (ScanEnd::Recognized, _) | (ScanEnd::NotRecognized, None) => LockWidgetState::Idle,
        };
        self.board.settle(self.uid, self.scan_id, state, now_ns);
        self.settled = true;
    }
}

impl Drop for ScanGuard {
    fn drop(&mut self) {
        if !self.settled {
            self.board
                .settle(self.uid, self.scan_id, LockWidgetState::Idle, None);
        }
    }
}
