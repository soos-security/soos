//! Failed-password alerts: correlation, bounded history, acknowledgement and the journal
//! follower task (ADR 2026-10-06 "Failed-Password Alerts in `soos-remote` From the System
//! Journal", architect spec `AI/architect_spec_remote_auth_alerts.md` §4, §5, §6.1).
//!
//! - [`Correlator`]: pure state machine turning journal [`Signal`]s into [`Attempt`]s (one
//!   attempt per real password check, pairing window, anchors, bounded buffers).
//! - [`AlertBook`]: bounded history (coalescing, eviction, saturating counts, per-start
//!   epoch, acknowledgement, replay-only marker).
//! - [`read_ack_file`] / [`write_ack_file`]: the `0600` acknowledgement marker file.
//! - The follower task and the shared runtime used by `server.rs`.
//!
//! O-2: nothing here carries, stores, logs or displays typed text, any part, length or hash
//! of it; the view holds only time, source class, account class, kind and count. This
//! module never logs through `tracing`: state transitions go through the fixed-text audit
//! events of `audit.rs`.

use std::collections::VecDeque;
use std::fmt;
use std::fs::OpenOptions;
use std::io::{ErrorKind, Read};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use nix::fcntl::OFlag;
use tokio::sync::watch;
use tokio::time::Instant;

use crate::audit;
use crate::auth::{system_random, RandomError, RandomSource};
use crate::credentials::write_atomic;
use crate::journal::{
    classify_entry, parse_entry, AccountClass, FollowStart, HelperSide, JournalCursor,
    JournalError, JournalLines, JournalSource, LineRead, OwnerLogin, Signal, SourceClass,
    TrustContext,
};
use crate::server::UnixClock;
use crate::{
    ALERTS_EPOCH_HEX_LEN, ALERTS_MARKER_FLUSH_MIN_INTERVAL_MS, ALERT_COALESCE_WINDOW_US,
    ANCHOR_WINDOW_US, HISTORY_REBUILD_WINDOW_S, JOURNAL_BATCH_PAUSE_MS, JOURNAL_IDLE_TICK_MS,
    JOURNAL_LINES_PER_BATCH, JOURNAL_RESTART_MAX_MS, JOURNAL_RESTART_MIN_MS, JOURNAL_STABLE_RUN_MS,
    MAX_ALERTS_ACK_FILE_BYTES, MAX_ALERT_HISTORY, MAX_LOCK_SCREEN_PROGRAMS, MAX_PENDING_CHECKS,
    MAX_RECENT_FAILURES, PAIR_WINDOW_US, PUSH_MAX_ATTEMPT_AGE_MS,
};

/// What happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptKind {
    /// A wrong password was checked.
    WrongPassword,
    /// An attempt while `pam_faillock` had locked the account.
    LockedOut,
}

/// One failed attempt (no text).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attempt {
    /// Where.
    pub class: SourceClass,
    /// Which account class.
    pub account: AccountClass,
    /// What happened.
    pub kind: AttemptKind,
    /// Journal time (µs).
    pub at_us: u64,
}

/// An unpaired password check.
#[derive(Clone, Copy)]
struct PendingCheck {
    side: HelperSide,
    account: AccountClass,
    at_us: u64,
}

/// A recent `pam_unix` failure kept for pairing.
#[derive(Clone, Copy)]
struct RecentFailure {
    side: HelperSide,
    account: AccountClass,
    at_us: u64,
    paired: bool,
}

/// The most recent trusted failure of a side and account.
#[derive(Clone, Copy)]
struct Anchor {
    side: HelperSide,
    account: AccountClass,
    class: SourceClass,
    at_us: u64,
}

/// Pure state machine (no clock, no I/O). Bounded by `MAX_PENDING_CHECKS`,
/// `MAX_RECENT_FAILURES` and at most 6 anchors (one per side and account class).
pub struct Correlator {
    pending: VecDeque<PendingCheck>,
    recent: VecDeque<RecentFailure>,
    anchors: Vec<Anchor>,
}

impl Default for Correlator {
    fn default() -> Self {
        Self::new()
    }
}

impl Correlator {
    /// An empty correlator.
    #[must_use]
    pub fn new() -> Self {
        Self {
            pending: VecDeque::with_capacity(MAX_PENDING_CHECKS.saturating_add(1)),
            recent: VecDeque::with_capacity(MAX_RECENT_FAILURES.saturating_add(1)),
            anchors: Vec::with_capacity(6),
        }
    }

    /// Rule 3: an anchor of the same side and account at most `ANCHOR_WINDOW_US` older →
    /// its class (and the anchor is refreshed); else root side → `Other`; else dropped.
    fn resolve(&mut self, check: PendingCheck) -> Option<Attempt> {
        let anchor = self
            .anchors
            .iter_mut()
            .find(|a| a.side == check.side && a.account == check.account)
            .filter(|a| check.at_us.saturating_sub(a.at_us) <= ANCHOR_WINDOW_US);
        let class = match (anchor, check.side) {
            (Some(anchor), _) => {
                anchor.at_us = anchor.at_us.max(check.at_us);
                anchor.class
            }
            (None, HelperSide::Root) => SourceClass::Other,
            (None, HelperSide::Owner) => return None,
        };
        Some(Attempt {
            class,
            account: check.account,
            kind: AttemptKind::WrongPassword,
            at_us: check.at_us,
        })
    }

    /// Feeds one signal; returns the attempts it completes (0 or 1, plus up to one resolved
    /// by overflow of the pending checks).
    pub fn push(&mut self, signal: Signal) -> Vec<Attempt> {
        let mut out = Vec::new();
        match signal {
            Signal::Failure {
                class,
                side,
                account,
                at_us,
                trusted,
            } => {
                let paired_check = self.pending.iter().position(|p| {
                    p.side == side
                        && p.account == account
                        && p.at_us.abs_diff(at_us) <= PAIR_WINDOW_US
                });
                let paired = paired_check.is_some();
                if let Some(index) = paired_check {
                    self.pending.remove(index);
                }
                self.recent.push_back(RecentFailure {
                    side,
                    account,
                    at_us,
                    paired,
                });
                while self.recent.len() > MAX_RECENT_FAILURES {
                    self.recent.pop_front();
                }
                if trusted {
                    out.push(Attempt {
                        class,
                        account,
                        kind: AttemptKind::WrongPassword,
                        at_us,
                    });
                    let anchor = Anchor {
                        side,
                        account,
                        class,
                        at_us,
                    };
                    match self
                        .anchors
                        .iter_mut()
                        .find(|a| a.side == side && a.account == account)
                    {
                        Some(existing) => *existing = anchor,
                        None => self.anchors.push(anchor),
                    }
                }
            }
            Signal::Check {
                side,
                account,
                at_us,
            } => {
                if let Some(failure) = self.recent.iter_mut().find(|f| {
                    !f.paired
                        && f.side == side
                        && f.account == account
                        && f.at_us.abs_diff(at_us) <= PAIR_WINDOW_US
                }) {
                    failure.paired = true;
                    return out;
                }
                self.pending.push_back(PendingCheck {
                    side,
                    account,
                    at_us,
                });
                if self.pending.len() > MAX_PENDING_CHECKS {
                    if let Some(oldest) = self.pending.pop_front() {
                        out.extend(self.resolve(oldest));
                    }
                }
            }
            Signal::LockedOut {
                class,
                account,
                at_us,
            } => out.push(Attempt {
                class,
                account,
                kind: AttemptKind::LockedOut,
                at_us,
            }),
        }
        out
    }

    /// Resolves every pending check with `at_us + PAIR_WINDOW_US < now_us`.
    pub fn expire(&mut self, now_us: u64) -> Vec<Attempt> {
        let mut out = Vec::new();
        let mut kept = VecDeque::with_capacity(self.pending.len());
        while let Some(check) = self.pending.pop_front() {
            if check.at_us.saturating_add(PAIR_WINDOW_US) < now_us {
                out.extend(self.resolve(check));
            } else {
                kept.push_back(check);
            }
        }
        self.pending = kept;
        out
    }

    /// Journal time of the oldest pending (unresolved) check, if any.
    #[must_use]
    pub fn oldest_pending_us(&self) -> Option<u64> {
        self.pending.iter().map(|p| p.at_us).min()
    }
}

/// Per-start view identity: 64 random bits, rendered as `ALERTS_EPOCH_HEX_LEN` lowercase
/// hex. `Debug` is redacted; never logged, never persisted.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AlertsEpoch(u64);

impl fmt::Debug for AlertsEpoch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AlertsEpoch(<redacted>)")
    }
}

impl AlertsEpoch {
    /// Draws 8 bytes from the crate's `RandomSource`.
    ///
    /// # Errors
    ///
    /// [`RandomError`] when the source fails.
    pub fn draw(random: &RandomSource) -> Result<Self, RandomError> {
        let mut bytes = [0u8; 8];
        random(&mut bytes)?;
        Ok(Self(u64::from_be_bytes(bytes)))
    }

    /// Pure constructor (tests).
    #[must_use]
    pub fn from_u64(value: u64) -> Self {
        Self(value)
    }

    /// Exactly 16 lowercase hex characters (zero-padded).
    #[must_use]
    pub fn to_hex(self) -> String {
        format!("{:016x}", self.0)
    }

    /// Exactly 16 characters of `[0-9a-f]`; anything else (uppercase included) → `None`.
    #[must_use]
    pub fn parse_hex(raw: &[u8]) -> Option<Self> {
        if raw.len() != ALERTS_EPOCH_HEX_LEN
            || !raw
                .iter()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
        {
            return None;
        }
        let text = std::str::from_utf8(raw).ok()?;
        u64::from_str_radix(text, 16).ok().map(Self)
    }
}

/// One history record (several coalesced attempts).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AlertRecord {
    /// Seq of the first attempt of the record.
    pub id: u64,
    /// `first_us / 1000`.
    pub first_unix_ms: u64,
    /// `last_us / 1000`.
    pub last_unix_ms: u64,
    /// Where.
    pub source: SourceClass,
    /// Which account class.
    pub account: AccountClass,
    /// What happened.
    pub kind: AttemptKind,
    /// Attempts in the record (saturating).
    pub count: u32,
    /// Acknowledged by the owner.
    pub acknowledged: bool,
    /// Seq of the newest attempt of the record.
    #[serde(skip)]
    pub last_seq: u64,
    /// Smallest journal time of the record (µs).
    #[serde(skip)]
    pub first_us: u64,
    /// Largest journal time of the record (µs).
    #[serde(skip)]
    pub last_us: u64,
}

/// Book failure (fixed text).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BookError {
    /// The attempt seq counter cannot advance.
    #[error("alert counter overflow")]
    Overflow,
    /// `through` exceeds the highest recorded seq.
    #[error("through is beyond the newest attempt")]
    BeyondNewest,
    /// The acknowledged view belongs to another start (epoch mismatch).
    #[error("the acknowledged view belongs to an earlier start")]
    StaleView,
}

/// Evicted unacknowledged attempts, per kind.
#[derive(Default, Clone, Copy)]
struct Evicted {
    wrong_password: u32,
    locked_out: u32,
    max_seq: u64,
    first_us: Option<u64>,
}

/// Bounded alert history of one service start.
pub struct AlertBook {
    epoch: AlertsEpoch,
    started_us: u64,
    loaded_marker_us: u64,
    records: VecDeque<AlertRecord>,
    next_seq: u64,
    highest_seq: u64,
    ack_high_water_us: u64,
    evicted: Evicted,
}

impl AlertBook {
    /// `acknowledged_until_us`: the marker read from the ack file (0 when absent or
    /// invalid); `started_us`: wall-clock time (µs) of the service start.
    #[must_use]
    pub fn new(acknowledged_until_us: u64, epoch: AlertsEpoch, started_us: u64) -> Self {
        Self {
            epoch,
            started_us,
            loaded_marker_us: acknowledged_until_us,
            records: VecDeque::with_capacity(MAX_ALERT_HISTORY.saturating_add(1)),
            next_seq: 1,
            highest_seq: 0,
            ack_high_water_us: acknowledged_until_us,
            evicted: Evicted::default(),
        }
    }

    /// Wall-clock time (µs) of the service start.
    #[must_use]
    pub fn started_us(&self) -> u64 {
        self.started_us
    }

    /// The epoch of this book.
    #[must_use]
    pub fn epoch(&self) -> AlertsEpoch {
        self.epoch
    }

    /// Records one attempt (rule R: acknowledged on arrival only when replayed from before
    /// the start and covered by the loaded marker; coalescing into the newest record).
    ///
    /// # Errors
    ///
    /// [`BookError::Overflow`] when the seq counter cannot advance (nothing recorded).
    pub fn record(&mut self, attempt: Attempt) -> Result<(), BookError> {
        let seq = self.next_seq;
        let next = seq.checked_add(1).ok_or(BookError::Overflow)?;
        self.next_seq = next;
        self.highest_seq = seq;
        let acknowledged =
            attempt.at_us <= self.loaded_marker_us && attempt.at_us < self.started_us;
        if let Some(newest) = self.records.back_mut() {
            if newest.source == attempt.class
                && newest.account == attempt.account
                && newest.kind == attempt.kind
                && newest.acknowledged == acknowledged
                && attempt.at_us.abs_diff(newest.last_us) <= ALERT_COALESCE_WINDOW_US
            {
                newest.count = newest.count.saturating_add(1);
                newest.first_us = newest.first_us.min(attempt.at_us);
                newest.last_us = newest.last_us.max(attempt.at_us);
                newest.last_seq = seq;
                newest.first_unix_ms = newest.first_us / 1000;
                newest.last_unix_ms = newest.last_us / 1000;
                return Ok(());
            }
        }
        self.records.push_back(AlertRecord {
            id: seq,
            first_unix_ms: attempt.at_us / 1000,
            last_unix_ms: attempt.at_us / 1000,
            source: attempt.class,
            account: attempt.account,
            kind: attempt.kind,
            count: 1,
            acknowledged,
            last_seq: seq,
            first_us: attempt.at_us,
            last_us: attempt.at_us,
        });
        while self.records.len() > MAX_ALERT_HISTORY {
            let Some(oldest) = self.records.pop_front() else {
                break;
            };
            if oldest.acknowledged {
                continue;
            }
            match oldest.kind {
                AttemptKind::WrongPassword => {
                    self.evicted.wrong_password =
                        self.evicted.wrong_password.saturating_add(oldest.count);
                }
                AttemptKind::LockedOut => {
                    self.evicted.locked_out = self.evicted.locked_out.saturating_add(oldest.count);
                }
            }
            self.evicted.max_seq = self.evicted.max_seq.max(oldest.last_seq);
            self.evicted.first_us = Some(
                self.evicted
                    .first_us
                    .map_or(oldest.first_us, |first| first.min(oldest.first_us)),
            );
        }
        Ok(())
    }

    /// Acknowledges every record whose `last_seq <= through`, for the current epoch only.
    ///
    /// # Errors
    ///
    /// [`BookError::StaleView`] when `epoch` differs, [`BookError::BeyondNewest`] when
    /// `through` exceeds the highest recorded seq; nothing changes on either.
    pub fn acknowledge(&mut self, epoch: AlertsEpoch, through: u64) -> Result<(), BookError> {
        if epoch != self.epoch {
            return Err(BookError::StaleView);
        }
        if through == 0 {
            return Ok(());
        }
        if through > self.highest_seq {
            return Err(BookError::BeyondNewest);
        }
        for record in &mut self.records {
            if record.last_seq <= through && !record.acknowledged {
                record.acknowledged = true;
                self.ack_high_water_us = self.ack_high_water_us.max(record.last_us);
            }
        }
        if through >= self.evicted.max_seq {
            self.evicted = Evicted {
                max_seq: self.evicted.max_seq,
                ..Evicted::default()
            };
        }
        Ok(())
    }

    /// The marker to persist (rule M): strictly below every attempt known to be
    /// unacknowledged (records, evicted attempts, pending checks), at most the high water.
    #[must_use]
    pub fn marker(&self, oldest_pending_us: Option<u64>) -> u64 {
        let earliest_unacknowledged = self
            .records
            .iter()
            .filter(|r| !r.acknowledged)
            .map(|r| r.first_us)
            .chain(self.evicted.first_us)
            .chain(oldest_pending_us)
            .min();
        match earliest_unacknowledged {
            Some(earliest) => self.ack_high_water_us.min(earliest.saturating_sub(1)),
            None => self.ack_high_water_us,
        }
    }

    /// The view of this book (`reason` is `None`; the runtime fills it for `Unavailable`).
    #[must_use]
    pub fn view(&self, state: AlertsState, lock_screen: LockScreenCoverage) -> AlertsView {
        self.view_with(state, None, Some(lock_screen))
    }

    fn view_with(
        &self,
        state: AlertsState,
        reason: Option<UnavailableReason>,
        lock_screen: Option<LockScreenCoverage>,
    ) -> AlertsView {
        let unacknowledged = |kind: AttemptKind| {
            self.records
                .iter()
                .filter(|r| !r.acknowledged && r.kind == kind)
                .fold(0u32, |total, r| total.saturating_add(r.count))
        };
        let newest = self
            .records
            .iter()
            .filter(|r| !r.acknowledged)
            .max_by_key(|r| r.last_us);
        AlertsView {
            state,
            reason,
            epoch: Some(self.epoch.to_hex()),
            lock_screen,
            unacknowledged_wrong_password: unacknowledged(AttemptKind::WrongPassword)
                .saturating_add(self.evicted.wrong_password),
            unacknowledged_locked_out: unacknowledged(AttemptKind::LockedOut)
                .saturating_add(self.evicted.locked_out),
            last_unix_ms: newest.map(|r| r.last_unix_ms),
            last_source: newest.map(|r| r.source),
            through: self.highest_seq,
            history: self.records.iter().rev().cloned().collect(),
        }
    }

    /// Test seam: the seq the next recorded attempt receives.
    #[doc(hidden)]
    #[must_use]
    pub fn with_next_seq(mut self, next_seq: u64) -> Self {
        self.next_seq = next_seq;
        self
    }

    /// Test seam: overwrites the `count` of the newest record (no-op without a record).
    #[doc(hidden)]
    pub fn force_newest_count(&mut self, count: u32) {
        if let Some(newest) = self.records.back_mut() {
            newest.count = count;
        }
    }
}

/// Feature state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertsState {
    /// `password_alerts` is off.
    Disabled,
    /// The journal backlog is being read.
    Starting,
    /// Caught up and following.
    Active,
    /// Not working (see the reason); never shown as "no attempts".
    Unavailable,
}

/// Why the feature is unavailable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnavailableReason {
    /// The account cannot read the system journal.
    NoJournalAccess,
    /// `journalctl` missing, failed to start, or stopped.
    JournalReaderFailed,
    /// The owner's login could not be resolved.
    OwnerUnresolved,
    /// The attempt counter overflowed.
    Overflow,
    /// The CSPRNG failed when drawing the epoch.
    RngFailed,
}

/// Lock-screen coverage, evaluated at every follower start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LockScreenCoverage {
    /// At least one configured lock-screen program exists as a regular file.
    Monitored,
    /// None does: lock-screen attempts are not monitored.
    NotConfigured,
}

/// JSON of `GET /api/alerts` and of `event: alerts`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AlertsView {
    /// Feature state.
    pub state: AlertsState,
    /// `Some` iff `state == Unavailable`.
    pub reason: Option<UnavailableReason>,
    /// 16 lowercase hex characters; `None` when disabled or without an epoch.
    pub epoch: Option<String>,
    /// `None` when disabled or before the first follower start.
    pub lock_screen: Option<LockScreenCoverage>,
    /// Unacknowledged wrong-password attempts (saturating, evicted included).
    pub unacknowledged_wrong_password: u32,
    /// Unacknowledged attempts while locked out.
    pub unacknowledged_locked_out: u32,
    /// Newest unacknowledged attempt (ms).
    pub last_unix_ms: Option<u64>,
    /// Source of the newest unacknowledged attempt.
    pub last_source: Option<SourceClass>,
    /// Highest attempt seq recorded (0 = none).
    pub through: u64,
    /// Newest first, at most `MAX_ALERT_HISTORY`.
    pub history: Vec<AlertRecord>,
}

impl AlertsView {
    /// A view without a book (disabled, or unavailable before a book exists).
    #[must_use]
    pub fn empty(state: AlertsState, reason: Option<UnavailableReason>) -> Self {
        Self {
            state,
            reason,
            epoch: None,
            lock_screen: None,
            unacknowledged_wrong_password: 0,
            unacknowledged_locked_out: 0,
            last_unix_ms: None,
            last_source: None,
            through: 0,
            history: Vec::new(),
        }
    }

    /// The view of a disabled feature.
    #[must_use]
    pub fn disabled() -> Self {
        Self::empty(AlertsState::Disabled, None)
    }
}

/// Settings resolved by `main` (production) or a test.
pub struct AlertSettings {
    /// `None` → `unavailable` / `owner_unresolved`, nothing spawned.
    pub owner_login: Option<OwnerLogin>,
    /// Absolute paths of the trusted lock-screen programs.
    pub lock_screen_programs: Vec<String>,
    /// `None` → acknowledgements live in memory only.
    pub ack_path: Option<PathBuf>,
}

/// Acknowledgement file failure (fixed text; the caller falls back to marker 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AckFileError {
    /// Unreadable, insecure (symlink, mode, owner, not a regular file) or too large.
    #[error("acknowledgement file refused")]
    Refused,
    /// Not the expected JSON object.
    #[error("acknowledgement file malformed")]
    Malformed,
    /// The write failed.
    #[error("acknowledgement file not written")]
    Write,
}

/// On-disk shape of the acknowledgement file.
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct AckFile {
    version: u32,
    acknowledged_until_us: u64,
}

/// The only acknowledgement file version.
const ACK_FILE_VERSION: u32 = 1;

/// Reads the acknowledgement marker: `O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC`, a regular file
/// owned by `owner_uid` with mode exactly `0600`, at most `MAX_ALERTS_ACK_FILE_BYTES`, a JSON
/// object `{"version":1,"acknowledged_until_us":<u64>}`.
///
/// # Errors
///
/// [`AckFileError`] for every invalid file (`Ok(None)` when absent).
pub fn read_ack_file(path: &Path, owner_uid: u32) -> Result<Option<u64>, AckFileError> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC).bits())
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(AckFileError::Refused),
    };
    let metadata = file.metadata().map_err(|_| AckFileError::Refused)?;
    if !metadata.is_file() || metadata.uid() != owner_uid || metadata.mode() & 0o777 != 0o600 {
        return Err(AckFileError::Refused);
    }
    let limit = u64::try_from(MAX_ALERTS_ACK_FILE_BYTES).unwrap_or(u64::MAX);
    if metadata.len() > limit {
        return Err(AckFileError::Refused);
    }
    let mut bytes = Vec::with_capacity(MAX_ALERTS_ACK_FILE_BYTES.saturating_add(1));
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| AckFileError::Refused)?;
    if bytes.len() > MAX_ALERTS_ACK_FILE_BYTES {
        return Err(AckFileError::Refused);
    }
    if bytes.iter().find(|b| !b.is_ascii_whitespace()) != Some(&b'{') {
        return Err(AckFileError::Malformed);
    }
    let parsed: AckFile = serde_json::from_slice(&bytes).map_err(|_| AckFileError::Malformed)?;
    if parsed.version != ACK_FILE_VERSION {
        return Err(AckFileError::Malformed);
    }
    Ok(Some(parsed.acknowledged_until_us))
}

/// Writes the acknowledgement marker atomically (temporary `0600` file with a random
/// suffix, `O_NOFOLLOW`, `fsync`, `rename`, directory `fsync`); a symlink planted at the
/// path is replaced, never written through.
///
/// # Errors
///
/// [`AckFileError::Write`].
pub fn write_ack_file(path: &Path, acknowledged_until_us: u64) -> Result<(), AckFileError> {
    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .ok_or(AckFileError::Write)?;
    let bytes = serde_json::to_vec(&AckFile {
        version: ACK_FILE_VERSION,
        acknowledged_until_us,
    })
    .map_err(|_| AckFileError::Write)?;
    write_atomic(path, dir, &bytes, &system_random()).map_err(|_| AckFileError::Write)
}

// ---------------------------------------------------------------------------------------
// Runtime shared with `server.rs` and the follower task (spec §5)
// ---------------------------------------------------------------------------------------

/// Mutable state behind the runtime's mutex (never held across an await or a file write).
struct RuntimeState {
    book: Option<AlertBook>,
    correlator: Correlator,
    state: AlertsState,
    reason: Option<UnavailableReason>,
    coverage: Option<LockScreenCoverage>,
}

/// Marker persistence state (its own mutex: compute, write and record form one section).
struct FlushState {
    last_written: u64,
    write_failed: bool,
    pending: bool,
    last_flush: Option<Instant>,
}

/// Why an acknowledgement was refused after the header checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AckRefusal {
    /// No book (RNG failure) or a poisoned mutex → `503 unavailable`.
    Unavailable,
    /// Epoch mismatch → `409 stale_view`.
    StaleView,
    /// `through` beyond the newest attempt → `400 bad_request`.
    BeyondNewest,
}

/// Pure (Web Push trigger rule, ADR 2026-10-06 "Web Push Notifications for Failed-Password
/// Alerts Through a Separate Sender Unit"): an attempt is *live* when its journal time is at
/// or after the service start and it is at most `PUSH_MAX_ATTEMPT_AGE_MS` old when recorded.
#[must_use]
pub fn is_live(at_us: u64, started_us: u64, now_us: u64) -> bool {
    at_us >= started_us
        && now_us.saturating_sub(at_us) <= PUSH_MAX_ATTEMPT_AGE_MS.saturating_mul(1000)
}

/// Receives each live attempt the book recorded. Called under the alerts runtime mutex: it
/// must not block, await or do I/O (lock order: alerts mutex → push scheduler mutex).
pub(crate) type LiveAttemptSink = Arc<dyn Fn(Attempt) + Send + Sync>;

/// The alert state shared by the server routes, the event streams and the follower.
pub(crate) struct AlertsRuntime {
    /// Web Push trigger (only when push is active).
    live_sink: Option<LiveAttemptSink>,
    inner: Mutex<RuntimeState>,
    flush: Mutex<FlushState>,
    /// Bumped on every visible change; streams subscribe.
    version: watch::Sender<u64>,
    /// Last acknowledgement that reached the rate gate.
    pub(crate) ack_gate: tokio::sync::Mutex<Option<Instant>>,
    ack_path: Option<PathBuf>,
    clock: UnixClock,
}

impl AlertsRuntime {
    /// Set-up at service start: draws the epoch, reads the ack file, creates the book.
    /// Returns the runtime and, when a follower must run, its plan. `wired == false`
    /// (enabled without a journal source) is `unavailable` / `journal_reader_failed`.
    pub(crate) fn setup(
        settings: Option<&AlertSettings>,
        random: &RandomSource,
        clock: UnixClock,
        file_owner_uid: u32,
        owner_uid: u32,
        live_sink: Option<LiveAttemptSink>,
    ) -> (Arc<Self>, Option<TrustContext>) {
        let mut inner = RuntimeState {
            book: None,
            correlator: Correlator::new(),
            state: AlertsState::Starting,
            reason: None,
            coverage: None,
        };
        let mut last_written = 0;
        let mut context = None;
        let ack_path = settings.and_then(|s| s.ack_path.clone());
        match AlertsEpoch::draw(random) {
            Err(_) => {
                inner.state = AlertsState::Unavailable;
                inner.reason = Some(UnavailableReason::RngFailed);
                audit::alerts_unavailable();
            }
            Ok(epoch) => {
                let marker = match ack_path
                    .as_deref()
                    .map(|p| read_ack_file(p, file_owner_uid))
                {
                    Some(Ok(Some(marker))) => marker,
                    Some(Err(_)) => {
                        audit::alerts_ack_not_persisted();
                        0
                    }
                    Some(Ok(None)) | None => 0,
                };
                last_written = marker;
                let started_us = clock().saturating_mul(1000);
                inner.book = Some(AlertBook::new(marker, epoch, started_us));
                let owner = settings.and_then(|s| s.owner_login.clone().map(|login| (s, login)));
                match (settings, owner) {
                    (None, _) => {
                        inner.state = AlertsState::Unavailable;
                        inner.reason = Some(UnavailableReason::JournalReaderFailed);
                        audit::alerts_unavailable();
                    }
                    (Some(_), None) => {
                        inner.state = AlertsState::Unavailable;
                        inner.reason = Some(UnavailableReason::OwnerUnresolved);
                        audit::alerts_unavailable();
                    }
                    (Some(_), Some((settings, owner_login))) => {
                        context = Some(TrustContext {
                            owner_uid,
                            owner_login,
                            lock_screen_programs: settings
                                .lock_screen_programs
                                .iter()
                                .take(MAX_LOCK_SCREEN_PROGRAMS)
                                .cloned()
                                .collect(),
                        });
                    }
                }
            }
        }
        let runtime = Arc::new(Self {
            live_sink,
            inner: Mutex::new(inner),
            flush: Mutex::new(FlushState {
                last_written,
                write_failed: false,
                pending: false,
                last_flush: None,
            }),
            version: watch::Sender::new(0),
            ack_gate: tokio::sync::Mutex::new(None),
            ack_path,
            clock,
        });
        (runtime, context)
    }

    fn now_us(&self) -> u64 {
        (self.clock)().saturating_mul(1000)
    }

    fn lock_inner(&self) -> Option<MutexGuard<'_, RuntimeState>> {
        self.inner.lock().ok()
    }

    /// A receiver of the version counter (streams).
    pub(crate) fn subscribe(&self) -> watch::Receiver<u64> {
        self.version.subscribe()
    }

    fn bump(&self) {
        self.version.send_modify(|v| *v = v.wrapping_add(1));
    }

    /// The current view, cloned under the mutex; a poisoned mutex is `unavailable`.
    pub(crate) fn view(&self) -> AlertsView {
        let Some(inner) = self.lock_inner() else {
            return AlertsView::empty(
                AlertsState::Unavailable,
                Some(UnavailableReason::JournalReaderFailed),
            );
        };
        let reason = if inner.state == AlertsState::Unavailable {
            inner.reason
        } else {
            None
        };
        match &inner.book {
            Some(book) => book.view_with(inner.state, reason, inner.coverage),
            None => {
                let mut view = AlertsView::empty(inner.state, reason);
                view.lock_screen = inner.coverage;
                view
            }
        }
    }

    /// Changes the state; audit events once per transition; bumps the version.
    fn set_state(
        &self,
        inner: &mut RuntimeState,
        state: AlertsState,
        reason: Option<UnavailableReason>,
    ) {
        if inner.state == state && inner.reason == reason {
            return;
        }
        let was = inner.state;
        inner.state = state;
        inner.reason = reason;
        if state == AlertsState::Unavailable && was != AlertsState::Unavailable {
            audit::alerts_unavailable();
        }
        if state == AlertsState::Active && was != AlertsState::Active {
            audit::alerts_active();
        }
        self.bump();
    }

    fn set_state_locked(&self, state: AlertsState, reason: Option<UnavailableReason>) {
        if let Some(mut inner) = self.lock_inner() {
            self.set_state(&mut inner, state, reason);
        }
    }

    /// Rule P: computes the marker and writes it when it differs from the last written
    /// value (`force`: at once; otherwise at most once per
    /// `ALERTS_MARKER_FLUSH_MIN_INTERVAL_MS`). The book mutex is released before the write;
    /// compute, write and record are serialised by the flush mutex (no await inside).
    fn flush(&self, force: bool) {
        let Ok(mut flush) = self.flush.lock() else {
            return;
        };
        let marker = {
            let Some(inner) = self.lock_inner() else {
                return;
            };
            let Some(book) = inner.book.as_ref() else {
                return;
            };
            book.marker(inner.correlator.oldest_pending_us())
        };
        if marker == flush.last_written {
            flush.pending = false;
            return;
        }
        let Some(path) = self.ack_path.as_deref() else {
            flush.last_written = marker;
            flush.pending = false;
            return;
        };
        let now = Instant::now();
        let throttled = flush.last_flush.is_some_and(|last| {
            now.duration_since(last) < Duration::from_millis(ALERTS_MARKER_FLUSH_MIN_INTERVAL_MS)
        });
        if !force && throttled {
            flush.pending = true;
            return;
        }
        flush.last_flush = Some(now);
        flush.pending = false;
        match write_ack_file(path, marker) {
            Ok(()) => {
                flush.last_written = marker;
                flush.write_failed = false;
            }
            Err(_) => {
                if !flush.write_failed {
                    audit::alerts_ack_not_persisted();
                }
                flush.write_failed = true;
            }
        }
    }

    fn flush_pending(&self) -> bool {
        self.flush.lock().is_ok_and(|flush| flush.pending)
    }

    /// Applies an acknowledgement and persists the marker (a write failure keeps the
    /// in-memory acknowledgement).
    pub(crate) fn acknowledge(
        &self,
        epoch: AlertsEpoch,
        through: u64,
    ) -> Result<AlertsView, AckRefusal> {
        {
            let Some(mut inner) = self.lock_inner() else {
                return Err(AckRefusal::Unavailable);
            };
            let Some(book) = inner.book.as_mut() else {
                return Err(AckRefusal::Unavailable);
            };
            match book.acknowledge(epoch, through) {
                Ok(()) => {}
                Err(BookError::StaleView) => return Err(AckRefusal::StaleView),
                Err(BookError::BeyondNewest | BookError::Overflow) => {
                    return Err(AckRefusal::BeyondNewest)
                }
            }
        }
        self.flush(true);
        self.bump();
        Ok(self.view())
    }

    /// Records attempts; `true` on seq overflow (the state is then `unavailable` /
    /// `overflow`). The live attempts are returned instead of being handed to the live sink
    /// here: the caller passes them to [`AlertsRuntime::deliver_live`] after releasing the
    /// `inner` mutex, so the sink (which takes the push scheduler lock) never runs under it.
    fn record(&self, inner: &mut RuntimeState, attempts: Vec<Attempt>) -> (bool, Vec<Attempt>) {
        let mut changed = false;
        let mut overflow = false;
        let now_us = self.now_us();
        let mut live = Vec::new();
        for attempt in attempts {
            let Some(book) = inner.book.as_mut() else {
                break;
            };
            if book.record(attempt).is_err() {
                overflow = true;
                break;
            }
            changed = true;
            if self.live_sink.is_some() && is_live(attempt.at_us, book.started_us(), now_us) {
                live.push(attempt);
            }
        }
        if overflow {
            self.set_state(
                inner,
                AlertsState::Unavailable,
                Some(UnavailableReason::Overflow),
            );
        } else if changed {
            self.bump();
        }
        (overflow, live)
    }

    /// Hands live attempts to the live sink; called with no runtime mutex held.
    fn deliver_live(&self, live: Vec<Attempt>) {
        if let Some(sink) = &self.live_sink {
            for attempt in live {
                sink(attempt);
            }
        }
    }

    fn set_coverage(&self, coverage: LockScreenCoverage) {
        if let Some(mut inner) = self.lock_inner() {
            if inner.coverage != Some(coverage) {
                inner.coverage = Some(coverage);
                self.bump();
            }
        }
    }
}

/// Lock-screen coverage: at least one configured path is a regular file (no path is ever
/// echoed; a stat failure counts as absent).
fn coverage_of(programs: &[String]) -> LockScreenCoverage {
    let monitored = programs
        .iter()
        .take(MAX_LOCK_SCREEN_PROGRAMS)
        .any(|path| std::fs::metadata(path).is_ok_and(|m| m.is_file()));
    if monitored {
        LockScreenCoverage::Monitored
    } else {
        LockScreenCoverage::NotConfigured
    }
}

/// The unavailable reason of a journal failure.
fn reason_of(err: JournalError) -> UnavailableReason {
    match err {
        JournalError::NoAccess => UnavailableReason::NoJournalAccess,
        JournalError::NotFound | JournalError::Spawn | JournalError::ProbeTimeout => {
            UnavailableReason::JournalReaderFailed
        }
    }
}

/// Doubles the backoff up to `JOURNAL_RESTART_MAX_MS`.
fn next_backoff(backoff: Duration) -> Duration {
    backoff
        .saturating_mul(2)
        .min(Duration::from_millis(JOURNAL_RESTART_MAX_MS))
}

/// `from + after`, or `from` when not representable (fail-closed: fires at once).
fn later(from: Instant, after: Duration) -> Instant {
    from.checked_add(after).unwrap_or(from)
}

/// Where the next follower starts, and which entries it skips.
struct Resume {
    first: bool,
    cursor: Option<JournalCursor>,
    last_seen_us: Option<u64>,
}

impl Resume {
    fn next_start(&self, now_us: u64) -> (FollowStart, Option<u64>) {
        if self.first {
            let since = (now_us / 1_000_000).saturating_sub(HISTORY_REBUILD_WINDOW_S);
            return (FollowStart::Since { unix_s: since }, None);
        }
        if let Some(cursor) = self.cursor.clone() {
            return (FollowStart::AfterCursor(cursor), None);
        }
        let last = self.last_seen_us.unwrap_or(0);
        (
            FollowStart::Since {
                unix_s: last / 1_000_000,
            },
            self.last_seen_us,
        )
    }
}

/// How one follower run ended.
enum RunEnd {
    /// The child ended (EOF or read error).
    Ended { got_line: bool },
    /// The attempt counter overflowed: the follower stops for good.
    Overflow,
}

/// Processes one parsed line; returns `true` on overflow.
fn process_line(
    runtime: &AlertsRuntime,
    context: &TrustContext,
    line: &[u8],
    follow: &mut FollowRun,
    resume: &mut Resume,
) -> bool {
    let Ok(entry) = parse_entry(line) else {
        return false;
    };
    if follow
        .skip_through_us
        .is_some_and(|skip| entry.realtime_us <= skip)
    {
        return false;
    }
    resume.last_seen_us = Some(
        resume
            .last_seen_us
            .map_or(entry.realtime_us, |last| last.max(entry.realtime_us)),
    );
    if let Some(cursor) = entry.cursor.clone() {
        resume.cursor = Some(cursor);
    }
    follow.newest_us = follow.newest_us.max(entry.realtime_us);
    let (overflow, live) = {
        let Some(mut inner) = runtime.lock_inner() else {
            return false;
        };
        if entry.realtime_us >= follow.started_us && inner.state == AlertsState::Starting {
            runtime.set_state(&mut inner, AlertsState::Active, None);
        }
        let mut attempts = Vec::new();
        if let Some(signal) = classify_entry(&entry, context) {
            attempts.extend(inner.correlator.push(signal));
        }
        attempts.extend(inner.correlator.expire(follow.newest_us));
        runtime.record(&mut inner, attempts)
    };
    runtime.deliver_live(live);
    drop(entry);
    if !overflow {
        runtime.flush(false);
    }
    overflow
}

/// The idle tick: catch-up (`Starting` → `Active`), expiry with the wall clock, pending
/// marker flush. Returns `true` on overflow.
fn idle_tick(runtime: &AlertsRuntime) -> bool {
    let now_us = runtime.now_us();
    let (overflow, live) = {
        let Some(mut inner) = runtime.lock_inner() else {
            return false;
        };
        if inner.state == AlertsState::Starting {
            runtime.set_state(&mut inner, AlertsState::Active, None);
        }
        let attempts = inner.correlator.expire(now_us);
        runtime.record(&mut inner, attempts)
    };
    runtime.deliver_live(live);
    if !overflow {
        let force = runtime.flush_pending();
        runtime.flush(force);
    }
    overflow
}

/// Per-run state of one follower.
struct FollowRun {
    started_us: u64,
    skip_through_us: Option<u64>,
    newest_us: u64,
}

/// Reads one follower until it ends (bounded batches, idle ticks).
async fn run_lines(
    runtime: &AlertsRuntime,
    context: &TrustContext,
    lines: &mut dyn JournalLines,
    follow: &mut FollowRun,
    resume: &mut Resume,
) -> RunEnd {
    let idle = Duration::from_millis(JOURNAL_IDLE_TICK_MS);
    let mut idle_at = later(Instant::now(), idle);
    let mut got_line = false;
    let mut in_batch = 0usize;
    loop {
        let read = tokio::select! {
            read = lines.next_line() => Some(read),
            () = tokio::time::sleep_until(idle_at) => None,
        };
        let Some(read) = read else {
            idle_at = later(Instant::now(), idle);
            if idle_tick(runtime) {
                return RunEnd::Overflow;
            }
            continue;
        };
        idle_at = later(Instant::now(), idle);
        match read {
            LineRead::End => return RunEnd::Ended { got_line },
            LineRead::Overlong => got_line = true,
            LineRead::Line(bytes) => {
                got_line = true;
                if process_line(runtime, context, &bytes, follow, resume) {
                    return RunEnd::Overflow;
                }
            }
        }
        in_batch = in_batch.saturating_add(1);
        if in_batch >= JOURNAL_LINES_PER_BATCH {
            in_batch = 0;
            tokio::time::sleep(Duration::from_millis(JOURNAL_BATCH_PAUSE_MS)).await;
            idle_at = later(Instant::now(), idle);
        }
    }
}

/// The follower task: probe, follow, catch up, restart with backoff. Never returns while
/// the server runs (after a seq overflow it parks forever); aborted at shutdown, which
/// drops (kills) the child.
pub(crate) async fn run_follower(
    runtime: Arc<AlertsRuntime>,
    source: Arc<dyn JournalSource>,
    context: TrustContext,
) {
    let min_backoff = Duration::from_millis(JOURNAL_RESTART_MIN_MS);
    let mut backoff = min_backoff;
    let mut resume = Resume {
        first: true,
        cursor: None,
        last_seen_us: None,
    };
    loop {
        runtime.set_coverage(coverage_of(&context.lock_screen_programs));
        if let Err(err) = source.probe().await {
            runtime.set_state_locked(AlertsState::Unavailable, Some(reason_of(err)));
            tokio::time::sleep(backoff).await;
            backoff = next_backoff(backoff);
            continue;
        }
        runtime.set_state_locked(AlertsState::Starting, None);
        let (start, skip_through_us) = resume.next_start(runtime.now_us());
        let by_cursor = matches!(start, FollowStart::AfterCursor(_));
        let mut lines = match source.follow(start).await {
            Ok(lines) => lines,
            Err(err) => {
                runtime.set_state_locked(AlertsState::Unavailable, Some(reason_of(err)));
                tokio::time::sleep(backoff).await;
                backoff = next_backoff(backoff);
                continue;
            }
        };
        resume.first = false;
        let mut follow = FollowRun {
            started_us: runtime.now_us(),
            skip_through_us,
            newest_us: 0,
        };
        let run_started = Instant::now();
        let end = run_lines(&runtime, &context, lines.as_mut(), &mut follow, &mut resume).await;
        drop(lines);
        let got_line = match end {
            RunEnd::Overflow => {
                // The state is already `unavailable` / `overflow`; stop for good without
                // ending `serve`.
                std::future::pending::<()>().await;
                return;
            }
            RunEnd::Ended { got_line } => got_line,
        };
        let ran = run_started.elapsed();
        if by_cursor && !got_line && ran < Duration::from_secs(1) {
            resume.cursor = None;
        }
        runtime.set_state_locked(
            AlertsState::Unavailable,
            Some(UnavailableReason::JournalReaderFailed),
        );
        if ran >= Duration::from_millis(JOURNAL_STABLE_RUN_MS) {
            backoff = min_backoff;
        }
        tokio::time::sleep(backoff).await;
        backoff = next_backoff(backoff);
    }
}
