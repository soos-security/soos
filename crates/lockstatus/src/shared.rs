//! Per-user coordination of every lock-status client: one daemon exchange in flight at a time, a
//! short shared cache and a shared error back-off (ADR 2026-10-15, spec §9 R1 and §10).
//!
//! A UID has two regular daemon connections and one auth-reserved connection that a third
//! connection falls back to; uncoordinated pollers (one hyprlock label per monitor, one widget per
//! output) could therefore hold the connection `pam_soos.so` needs. Every client of this crate
//! takes an exclusive `flock` on `$XDG_RUNTIME_DIR/soos-lockstatus.lock` before contacting the
//! daemon, and reuses an answer stored in that file while it is fresh.
//!
//! Trust boundary: the file is private to the user (directory and file `mode & 0o077 == 0`,
//! owned by the user, checked on the opened descriptor). A process of the same user can make the
//! clients show a wrong coarse state or nothing; it can never produce a success state (none
//! exists) or reach another user.

use std::fs::{self, File, OpenOptions};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::time::Duration;

use nix::errno::Errno;
use nix::fcntl::{Flock, FlockArg, OFlag};
use nix::time::ClockId;

use crate::client::{clock_now_ns, fetch_view, Deadline};
use crate::view::{LockView, ViewState};
use crate::ERROR_BACKOFF_MS;

/// Lock and cache file name inside `$XDG_RUNTIME_DIR`.
pub const LOCK_FILE_NAME: &str = "soos-lockstatus.lock";
/// Age up to which a successful answer is reused (= `MIN_POLL_INTERVAL_MS`).
pub const CACHE_FRESH_MS: u64 = 250;
/// Bytes read from the cache file at most (the longest record, `not_recognized` with the largest sequence and stamp, is 79 bytes).
pub const MAX_CACHE_BYTES: usize = 128;
/// Retry period of the non-blocking lock attempt.
pub const LOCK_RETRY_MS: u64 = 5;
/// Smallest budget left after taking the lock for a daemon exchange to be attempted; with less,
/// the call is [`Source::LockBusy`] (never counted as a daemon failure).
pub const MIN_EXCHANGE_BUDGET_MS: u64 = 50;
/// Saturation value of [`Record::failures`] (the last back-off step).
pub const MAX_RECORDED_FAILURES: u8 = 3;

/// Where a [`SharedOutcome`] came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A daemon exchange made by this call.
    Daemon,
    /// A fresh record written by another call.
    Cache,
    /// The lock could not be taken within the budget (no daemon contact).
    LockBusy,
    /// `$XDG_RUNTIME_DIR` or the lock file is absent or unsafe (no daemon contact).
    NoRuntimeDir,
}

/// The result of one coordinated fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedOutcome {
    /// The view to show.
    pub view: LockView,
    /// Where it came from.
    pub source: Source,
}

/// One cache record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Record {
    /// The cached view.
    pub view: LockView,
    /// Consecutive daemon failures (0 after a success), saturating at [`MAX_RECORDED_FAILURES`].
    pub failures: u8,
    /// `CLOCK_BOOTTIME` milliseconds when the exchange completed.
    pub stamp_ms: u64,
    /// [`socket_hash`] of the socket the answer came from.
    pub socket_hash: u64,
}

/// The user's lock-state view, coordinated with every other client of the same user.
///
/// `runtime_dir` must be an absolute, real directory owned by `uid` with no group or other
/// permission bit; the lock file must be a private regular file of `uid` with one link. Otherwise
/// the daemon is not contacted ([`Source::NoRuntimeDir`]). `now_ms` reads `CLOCK_BOOTTIME` in
/// milliseconds (injected for tests); `None` disables the cache for this call. `budget` bounds the
/// lock wait and the exchange together.
#[must_use]
pub fn shared_fetch(
    runtime_dir: Option<&Path>,
    socket: &Path,
    uid: u32,
    budget: Duration,
    now_ms: &dyn Fn() -> Option<u64>,
) -> SharedOutcome {
    let deadline = Deadline::start(budget);
    let Some(file) = open_lock_file(runtime_dir, uid) else {
        return outcome(LockView::UNAVAILABLE, Source::NoRuntimeDir);
    };
    let Some(locked) = lock_before_deadline(file, deadline) else {
        return outcome(LockView::UNAVAILABLE, Source::LockBusy);
    };
    let file: &File = &locked;
    let hash = socket_hash(socket);
    let cache_enabled = now_ms();
    let previous = cache_enabled.and_then(|_| read_record(file));
    if let (Some(now), Some(record)) = (cache_enabled, previous) {
        if is_reusable(&record, now, hash) {
            return outcome(record.view, Source::Cache);
        }
    }
    let Some(left) = deadline
        .remaining()
        .filter(|left| *left >= Duration::from_millis(MIN_EXCHANGE_BUDGET_MS))
    else {
        return outcome(LockView::UNAVAILABLE, Source::LockBusy);
    };
    let view = fetch_view(socket, uid, left);
    if cache_enabled.is_some() {
        if let Some(stamp_ms) = now_ms() {
            let failures = if view.available() {
                0
            } else {
                previous
                    .filter(|record| record.socket_hash == hash)
                    .map_or(0, |record| record.failures)
                    .saturating_add(1)
                    .min(MAX_RECORDED_FAILURES)
            };
            write_record(
                file,
                &Record {
                    view,
                    failures,
                    stamp_ms,
                    socket_hash: hash,
                },
            );
        }
    }
    outcome(view, Source::Daemon)
}

fn outcome(view: LockView, source: Source) -> SharedOutcome {
    SharedOutcome { view, source }
}

/// Whether `runtime_dir` is usable for the coordination file: absolute, a real directory (not a
/// symlink) owned by `uid`, without any group or other permission bit.
#[must_use]
pub fn runtime_dir_is_private(runtime_dir: Option<&Path>, uid: u32) -> bool {
    let Some(dir) = runtime_dir.filter(|dir| dir.is_absolute()) else {
        return false;
    };
    fs::symlink_metadata(dir).is_ok_and(|meta| {
        meta.file_type().is_dir() && meta.uid() == uid && meta.mode() & 0o077 == 0
    })
}

/// Opens the private lock file, or `None` when the directory or the file is unsafe.
fn open_lock_file(runtime_dir: Option<&Path>, uid: u32) -> Option<File> {
    if !runtime_dir_is_private(runtime_dir, uid) {
        return None;
    }
    let dir = runtime_dir?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(OFlag::O_NOFOLLOW.bits() | OFlag::O_CLOEXEC.bits())
        .open(dir.join(LOCK_FILE_NAME))
        .ok()?;
    // Authoritative check on the opened descriptor (fstat).
    let meta = file.metadata().ok()?;
    let private = meta.file_type().is_file()
        && meta.uid() == uid
        && meta.nlink() == 1
        && meta.mode() & 0o077 == 0;
    private.then_some(file)
}

/// Takes the exclusive lock, retrying a busy lock every [`LOCK_RETRY_MS`] until `deadline`.
fn lock_before_deadline(file: File, deadline: Deadline) -> Option<Flock<File>> {
    let mut file = file;
    loop {
        match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
            Ok(locked) => return Some(locked),
            Err((back, Errno::EWOULDBLOCK | Errno::EINTR)) => {
                let left = deadline.remaining()?;
                std::thread::sleep(Duration::from_millis(LOCK_RETRY_MS).min(left));
                file = back;
            }
            Err(_) => return None,
        }
    }
}

fn read_record(file: &File) -> Option<Record> {
    let mut buf = [0u8; MAX_CACHE_BYTES + 1];
    let mut len = 0usize;
    loop {
        let rest = buf.get_mut(len..)?;
        if rest.is_empty() {
            return None;
        }
        let offset = u64::try_from(len).ok()?;
        match file.read_at(rest, offset) {
            Ok(0) => break,
            Ok(n) => len = len.saturating_add(n),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    decode_record(buf.get(..len)?)
}

fn write_record(file: &File, record: &Record) {
    // A failed write only costs a cache miss later: the record is never a decision.
    if file.set_len(0).is_ok() {
        let _ignored = file.write_all_at(encode_record(record).as_bytes(), 0);
    }
}

/// `CLOCK_BOOTTIME` in milliseconds (keeps counting across suspend), `None` when unreadable.
#[must_use]
pub fn boottime_now_ms() -> Option<u64> {
    clock_now_ns(ClockId::CLOCK_BOOTTIME).map(|ns| ns / 1_000_000)
}

/// FNV-1a 64-bit hash of the socket path bytes.
#[must_use]
pub fn socket_hash(socket: &Path) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0100_0000_01b3;
    socket
        .as_os_str()
        .as_bytes()
        .iter()
        .fold(OFFSET, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(PRIME)
        })
}

fn state_word(state: ViewState) -> &'static str {
    match state {
        ViewState::Unavailable => "unavailable",
        ViewState::Idle => "idle",
        ViewState::Scanning => "scanning",
        ViewState::NotRecognized => "not_recognized",
    }
}

/// Encodes one record:
/// `v1 <idle|scanning|not_recognized|unavailable> <sequence> <failures> <stamp_ms> <hash:016x>\n`.
#[must_use]
pub fn encode_record(record: &Record) -> String {
    format!(
        "v1 {} {} {} {} {:016x}\n",
        state_word(record.view.state),
        record.view.sequence,
        record.failures,
        record.stamp_ms,
        record.socket_hash
    )
}

fn parse_decimal<T: std::str::FromStr>(field: &str) -> Option<T> {
    if field.is_empty() || !field.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    field.parse().ok()
}

/// Decodes one record; anything malformed is `None` (a miss, never a decision).
#[must_use]
pub fn decode_record(bytes: &[u8]) -> Option<Record> {
    if bytes.len() > MAX_CACHE_BYTES {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;
    let line = text.strip_suffix('\n')?;
    let fields: Vec<&str> = line.split(' ').collect();
    let [version, state, sequence, failures, stamp, hash] = fields.as_slice() else {
        return None;
    };
    if *version != "v1" {
        return None;
    }
    let state = match *state {
        "unavailable" => ViewState::Unavailable,
        "idle" => ViewState::Idle,
        "scanning" => ViewState::Scanning,
        "not_recognized" => ViewState::NotRecognized,
        _ => return None,
    };
    let sequence: u64 = parse_decimal(sequence)?;
    let failures: u8 = parse_decimal(failures)?;
    let stamp_ms: u64 = parse_decimal(stamp)?;
    if hash.len() != 16 || !hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    let socket_hash = u64::from_str_radix(hash, 16).ok()?;
    let consistent = if state == ViewState::Unavailable {
        sequence == 0 && (1..=MAX_RECORDED_FAILURES).contains(&failures)
    } else {
        failures == 0
    };
    consistent.then_some(Record {
        view: LockView { state, sequence },
        failures,
        stamp_ms,
        socket_hash,
    })
}

/// How long `record` may be reused: [`CACHE_FRESH_MS`] after a success, the back-off step of its
/// failure count after a failure.
#[must_use]
pub fn freshness_ms(record: &Record) -> u64 {
    match record.failures {
        0 => CACHE_FRESH_MS,
        n => {
            let step = usize::from(n.min(MAX_RECORDED_FAILURES)).saturating_sub(1);
            ERROR_BACKOFF_MS
                .get(step)
                .copied()
                .unwrap_or(CACHE_FRESH_MS)
        }
    }
}

/// Whether `record` may be reused at `now_ms` for `socket_hash`: same socket, stamp not in the
/// future, age at most [`freshness_ms`].
#[must_use]
pub fn is_reusable(record: &Record, now_ms: u64, socket_hash: u64) -> bool {
    record.socket_hash == socket_hash
        && now_ms
            .checked_sub(record.stamp_ms)
            .is_some_and(|age| age <= freshness_ms(record))
}
