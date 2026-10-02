//! Presence account guard (GitHub #323, owner answer Q1 2026-10-02).
//!
//! A presence unlock respects `pam_faillock` and account / password expiry: a locked,
//! expired **or undeterminable** account is never unlocked by presence (the password stays
//! the only path). The guard only reads: the PAM stack directories (policy options on
//! `pam_faillock.so` lines), `faillock.conf`, the user's tally file and `/etc/shadow`.
//! It never writes anything (no tally reset), never logs any file content and never keeps
//! the password hash or the tally `source` bytes.

use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};

use nix::fcntl::{Flock, FlockArg};
use zeroize::Zeroizing;

use super::{
    DEFAULT_FAILLOCK_CONF, DEFAULT_FAILLOCK_DENY, DEFAULT_FAILLOCK_DIR,
    DEFAULT_FAILLOCK_FAIL_INTERVAL_S, DEFAULT_FAILLOCK_UNLOCK_TIME_S, DEFAULT_PAM_DIRS,
    DEFAULT_SHADOW_PATH, MAX_FAILLOCK_CONF_BYTES, MAX_FAILLOCK_TIME_INTERVAL, MAX_PAM_DIR_ENTRIES,
    MAX_PAM_FILE_BYTES, MAX_SHADOW_BYTES, MAX_SHADOW_LINES, MAX_TALLY_BYTES, MAX_USER_NAME_LEN,
    TALLY_RECORD_BYTES, TALLY_STATUS_VALID, VENDOR_FAILLOCK_CONF,
};
use crate::error::DaemonError;

/// Maximum length in bytes of a `faillock.conf` `dir` value.
const MAX_FAILLOCK_DIR_LEN: usize = 4096;
/// Seconds per day (shadow fields are days since the epoch).
const SECONDS_PER_DAY: u64 = 86_400;
/// Number of `:` separated fields of a shadow line.
const SHADOW_FIELDS: usize = 9;
/// `pam_faillock.so` arguments that set policy (they override `faillock.conf` per stack).
const PAM_POLICY_ARGUMENT_PREFIXES: [&str; 7] = [
    "dir=",
    "deny=",
    "fail_interval=",
    "unlock_time=",
    "root_unlock_time=",
    "admin_group=",
    "conf=",
];
/// `faillock.conf` flags (no value).
const FAILLOCK_FLAGS: [&str; 6] = [
    "even_deny_root",
    "audit",
    "silent",
    "no_log_info",
    "local_users_only",
    "nodelay",
];

/// Validated user name from logind `Name`: 1..=`MAX_USER_NAME_LEN` bytes of
/// `[A-Za-z0-9._-]` plus a final optional `$`, not starting with `-` or `.` (it becomes a
/// path component of the tally file).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserName(String);

impl UserName {
    /// `None` for an invalid name.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        if raw.is_empty() || raw.len() > MAX_USER_NAME_LEN {
            return None;
        }
        if raw.starts_with('-') || raw.starts_with('.') {
            return None;
        }
        let body = raw.strip_suffix('$').unwrap_or(raw);
        if body.is_empty()
            || !body
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        {
            return None;
        }
        Some(Self(raw.to_string()))
    }

    /// The name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Why an account may not be unlocked by presence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountRefusal {
    /// `pam_faillock` would deny now.
    Faillocked,
    /// Shadow `expire` reached (`0` included).
    AccountExpired,
    /// Password expired for more than `inactive` days.
    AccountInactive,
    /// `lastchg + max` passed (a new password is required).
    PasswordExpired,
    /// `lastchg == 0` (a password change is forced).
    PasswordChangeForced,
    /// Password field starts with `!` or `*` (locked or no-login marker).
    PasswordLocked,
    /// UID 0: presence never unlocks a root session.
    RootAccount,
    /// Any read, parse, bound, clock or option problem.
    Undeterminable,
}

impl AccountRefusal {
    /// Stable, value-free code for logs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Faillocked => "faillocked",
            Self::AccountExpired => "account_expired",
            Self::AccountInactive => "account_inactive",
            Self::PasswordExpired => "password_expired",
            Self::PasswordChangeForced => "password_change_forced",
            Self::PasswordLocked => "password_locked",
            Self::RootAccount => "root_account",
            Self::Undeterminable => "undeterminable",
        }
    }
}

/// Result of an account check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountState {
    /// Presence may unlock.
    Usable,
    /// Presence must not unlock.
    Refused(AccountRefusal),
}

/// Mockable account guard. Synchronous, bounded file reads; the worker runs it on the
/// blocking pool under `ACCOUNT_CHECK_TIMEOUT_MS`. Never writes anything.
pub trait AccountGuard: Send + Sync + 'static {
    /// Checks the account of `user` (logind `Name`) / `uid` (logind `User`).
    fn check(&self, user: &UserName, uid: u32) -> AccountState;
}

/// Parsed `faillock.conf` policy (`root_unlock_time` resolved).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaillockPolicy {
    /// Tally directory.
    pub dir: PathBuf,
    /// Failures that lock (`0` never locks).
    pub deny: u16,
    /// Window of counted failures (seconds).
    pub fail_interval: u32,
    /// Lock duration (seconds, `0` = never unlocks).
    pub unlock_time: u32,
    /// Lock duration of administrators (defaults to `unlock_time`).
    pub root_unlock_time: u32,
    /// Administrator group (no group lookup is made; see [`faillock_denies`]).
    pub admin_group: Option<String>,
    /// `even_deny_root` flag.
    pub even_deny_root: bool,
}

impl Default for FaillockPolicy {
    fn default() -> Self {
        Self {
            dir: PathBuf::from(DEFAULT_FAILLOCK_DIR),
            deny: DEFAULT_FAILLOCK_DENY,
            fail_interval: DEFAULT_FAILLOCK_FAIL_INTERVAL_S,
            unlock_time: DEFAULT_FAILLOCK_UNLOCK_TIME_S,
            root_unlock_time: DEFAULT_FAILLOCK_UNLOCK_TIME_S,
            admin_group: None,
            even_deny_root: false,
        }
    }
}

/// One decoded `struct tally` record (the `source` bytes are never kept).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TallyRecord {
    /// `status` field (bit `TALLY_STATUS_VALID`).
    pub status: u16,
    /// `time` field (wall clock seconds).
    pub time: u64,
}

/// The fields of a shadow entry the guard needs (the password hash is only classified).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShadowEntry {
    /// Password field starts with `!` or `*` (`passwd -l`, `*`, `*LK*`, `*NP*`, ...).
    pub password_locked: bool,
    /// Day of the last password change.
    pub lastchg: Option<i64>,
    /// Maximum password age in days.
    pub max: Option<i64>,
    /// Inactivity period in days after password expiry.
    pub inactive: Option<i64>,
    /// Account expiry day.
    pub expire: Option<i64>,
}

/// Parses an unsigned decimal value (digits only, no sign, no prefix).
fn parse_decimal<T: std::str::FromStr>(value: &str) -> Option<T> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

/// Parses a time value bounded by `MAX_FAILLOCK_TIME_INTERVAL`; `never` = 0 when allowed.
fn parse_time(value: &str, allow_never: bool) -> Option<u32> {
    if allow_never && value == "never" {
        return Some(0);
    }
    let parsed: u32 = parse_decimal(value)?;
    if u64::from(parsed) > MAX_FAILLOCK_TIME_INTERVAL {
        return None;
    }
    Some(parsed)
}

/// Validates a `dir` value: absolute, no `..` component, bounded length.
fn parse_dir(value: &str) -> Option<PathBuf> {
    if value.is_empty() || value.len() > MAX_FAILLOCK_DIR_LEN || !value.starts_with('/') {
        return None;
    }
    let path = PathBuf::from(value);
    if path.components().any(|c| c == Component::ParentDir) {
        return None;
    }
    Some(path)
}

/// Parses `faillock.conf` with the grammar of `faillock_config.c` (`name [=] value`, bare
/// flags, `#` comments), but stricter: an unknown key, a malformed or out-of-range value,
/// non-UTF-8 content or more than `MAX_FAILLOCK_CONF_BYTES` is `None` (`Undeterminable`).
#[must_use]
pub fn parse_faillock_conf(content: &[u8]) -> Option<FaillockPolicy> {
    if content.len() > MAX_FAILLOCK_CONF_BYTES {
        return None;
    }
    let text = std::str::from_utf8(content).ok()?;
    let mut policy = FaillockPolicy::default();
    let mut root_unlock_time: Option<u32> = None;
    for raw_line in text.lines() {
        let line = raw_line
            .split_once('#')
            .map_or(raw_line, |(before, _)| before)
            .trim();
        if line.is_empty() {
            continue;
        }
        let name_end = line
            .find(|c: char| c.is_whitespace() || c == '=')
            .unwrap_or(line.len());
        let (name, rest) = line.split_at_checked(name_end)?;
        let rest = rest.trim_start();
        let value = rest.strip_prefix('=').unwrap_or(rest).trim();
        match name {
            "dir" => policy.dir = parse_dir(value)?,
            "deny" => policy.deny = parse_decimal(value)?,
            "fail_interval" => policy.fail_interval = parse_time(value, false)?,
            "unlock_time" => policy.unlock_time = parse_time(value, true)?,
            "root_unlock_time" => root_unlock_time = Some(parse_time(value, true)?),
            "admin_group" => {
                if value.is_empty() {
                    return None;
                }
                policy.admin_group = Some(value.to_string());
            }
            flag if FAILLOCK_FLAGS.contains(&flag) => {
                if !value.is_empty() {
                    return None;
                }
                if flag == "even_deny_root" {
                    policy.even_deny_root = true;
                }
            }
            _ => return None,
        }
    }
    policy.root_unlock_time = root_unlock_time.unwrap_or(policy.unlock_time);
    Some(policy)
}

/// Decodes native-endian `struct tally` records (`status` at 54, `time` at 56); `None` for
/// a size that is not a multiple of `TALLY_RECORD_BYTES` or above `MAX_TALLY_BYTES`.
#[must_use]
pub fn decode_tally_records(bytes: &[u8]) -> Option<Vec<TallyRecord>> {
    if bytes.len() > MAX_TALLY_BYTES || !bytes.len().is_multiple_of(TALLY_RECORD_BYTES) {
        return None;
    }
    let mut records = Vec::with_capacity(bytes.len() / TALLY_RECORD_BYTES);
    let (chunks, rest) = bytes.as_chunks::<TALLY_RECORD_BYTES>();
    if !rest.is_empty() {
        return None;
    }
    for chunk in chunks {
        let status = u16::from_ne_bytes(chunk.get(54..56)?.try_into().ok()?);
        let time = u64::from_ne_bytes(chunk.get(56..64)?.try_into().ok()?);
        records.push(TallyRecord { status, time });
    }
    Some(records)
}

/// Reads at most `limit` bytes of an opened regular file into a buffer allocated once
/// (never reallocated, zeroized on drop). `None` when more than `limit` bytes are present
/// (including a file that grew after `fstat`) or on a read error.
fn read_bounded(file: &File, size_hint: u64, limit: usize) -> Option<Zeroizing<Vec<u8>>> {
    let hint = usize::try_from(size_hint).unwrap_or(usize::MAX);
    let capacity = hint.min(limit).checked_add(1)?;
    let mut buffer = Zeroizing::new(vec![0u8; capacity]);
    let mut filled = 0usize;
    let mut reader = file;
    loop {
        let spare = buffer.get_mut(filled..)?;
        if spare.is_empty() {
            // The file holds at least `capacity` bytes: more than `limit` or grown.
            return None;
        }
        match reader.read(spare) {
            Ok(0) => break,
            Ok(n) => filled = filled.checked_add(n)?,
            Err(err) if err.kind() == ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    if filled > limit {
        return None;
    }
    buffer.truncate(filled);
    Some(buffer)
}

/// Opens `path` read-only and non-blocking (symlinks followed unless `nofollow`).
fn open_read_only(path: &Path, nofollow: bool) -> std::io::Result<File> {
    let mut flags = libc::O_NONBLOCK | libc::O_CLOEXEC;
    if nofollow {
        flags |= libc::O_NOFOLLOW;
    }
    OpenOptions::new().read(true).custom_flags(flags).open(path)
}

/// Outcome of reading one configuration source.
enum SourceRead {
    /// The file does not exist.
    Absent,
    /// The file is a directory.
    Directory,
    /// The bounded content of a regular file.
    Content(Zeroizing<Vec<u8>>),
}

/// Opens (following symlinks, non-blocking), `fstat`s the descriptor and reads a regular
/// file bounded by `limit`. `None` for any other error, a special file or an oversized file.
fn read_source(path: &Path, limit: usize) -> Option<SourceRead> {
    let file = match open_read_only(path, false) {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::NotFound => return Some(SourceRead::Absent),
        Err(_) => return None,
    };
    let metadata = file.metadata().ok()?;
    if metadata.is_dir() {
        return Some(SourceRead::Directory);
    }
    if !metadata.is_file() {
        return None;
    }
    read_bounded(&file, metadata.len(), limit).map(SourceRead::Content)
}

/// Reads the tally file at `path`: `O_NOFOLLOW`, non-blocking, regular file only, under a
/// shared non-blocking `flock`. A missing file is zero records; any other problem
/// (`EACCES`, symlink, FIFO, directory, held exclusive lock, bad size) is `None`.
#[must_use]
pub fn read_tally_records(path: &Path) -> Option<Vec<TallyRecord>> {
    let file = match open_read_only(path, true) {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::NotFound => return Some(Vec::new()),
        Err(_) => return None,
    };
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() {
        return None;
    }
    let locked = Flock::lock(file, FlockArg::LockSharedNonblock).ok()?;
    let content = read_bounded(&locked, metadata.len(), MAX_TALLY_BYTES)?;
    drop(locked);
    decode_tally_records(&content)
}

/// Replica of Linux-PAM `check_tally` for a non-admin user with `unlock_time`.
fn tally_locks(
    deny: u16,
    fail_interval: u32,
    unlock_time: u32,
    records: &[TallyRecord],
    now_s: u64,
) -> bool {
    let valid = records
        .iter()
        .filter(|r| r.status & TALLY_STATUS_VALID != 0)
        .map(|r| r.time);
    let latest = valid.clone().max().unwrap_or(0);
    let failures = valid
        .filter(|&time| latest.saturating_sub(time) < u64::from(fail_interval))
        .count();
    if deny == 0 || failures < usize::from(deny) {
        return false;
    }
    if unlock_time != 0 {
        match latest.checked_add(u64::from(unlock_time)) {
            Some(end) if end < now_s => return false,
            Some(_) => {}
            // Overflow: locked (fail closed).
            None => return true,
        }
    }
    true
}

/// Whether `pam_faillock` would deny now (`check_tally`). With `admin_group` set (no group
/// lookup is made) the user is evaluated both as administrator and as non-administrator and
/// refused if either reading locks.
#[must_use]
pub fn faillock_denies(policy: &FaillockPolicy, records: &[TallyRecord], now_s: u64) -> bool {
    let as_user = tally_locks(
        policy.deny,
        policy.fail_interval,
        policy.unlock_time,
        records,
        now_s,
    );
    if policy.admin_group.is_none() {
        return as_user;
    }
    as_user
        || tally_locks(
            policy.deny,
            policy.fail_interval,
            policy.root_unlock_time,
            records,
            now_s,
        )
}

/// Whether some `pam_faillock.so` line of a PAM stack file sets a policy option
/// (backslash continuations joined, `#` comments dropped). `true` = `Undeterminable`.
#[must_use]
pub fn scan_pam_faillock_options(content: &str) -> bool {
    let mut logical = String::new();
    for raw_line in content.lines() {
        let line = raw_line
            .split_once('#')
            .map_or(raw_line, |(before, _)| before)
            .trim_end();
        if let Some(continued) = line.strip_suffix('\\') {
            logical.push_str(continued);
            logical.push(' ');
            continue;
        }
        logical.push_str(line);
        if line_sets_faillock_policy(&logical) {
            return true;
        }
        logical.clear();
    }
    line_sets_faillock_policy(&logical)
}

/// Splits a logical PAM line into tokens like libpam (`_pam_mkargv`): a token starting with
/// `[` extends to the matching `]` (spaces included, `\]` is a literal `]`) and its brackets
/// are stripped; any other token ends at whitespace.
fn pam_tokens(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        let mut token = String::new();
        if c == '[' {
            chars.next();
            while let Some(c) = chars.next() {
                match c {
                    '\\' if chars.peek() == Some(&']') => {
                        token.push(']');
                        chars.next();
                    }
                    ']' => break,
                    other => token.push(other),
                }
            }
        } else {
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                token.push(c);
                chars.next();
            }
        }
        tokens.push(token);
    }
    tokens
}

/// One logical PAM line: a `pam_faillock.so` module token followed by a policy argument
/// (bracketed arguments included, as libpam strips their brackets; a bracketed control field
/// before the module token is never an argument).
fn line_sets_faillock_policy(line: &str) -> bool {
    let tokens = pam_tokens(line);
    let mut iter = tokens.iter();
    if !iter.any(|token| token.ends_with("pam_faillock.so")) {
        return false;
    }
    iter.any(|arg| {
        let arg = arg.trim_start_matches('[');
        arg == "even_deny_root"
            || PAM_POLICY_ARGUMENT_PREFIXES
                .iter()
                .any(|prefix| arg.starts_with(prefix))
    })
}

/// Parses one numeric shadow field: empty = not set; otherwise decimal `i64 >= 0`.
/// The outer `None` means malformed.
fn shadow_number(field: &str) -> Option<Option<i64>> {
    if field.is_empty() {
        return Some(None);
    }
    parse_decimal::<i64>(field).map(Some)
}

/// Finds the single shadow line of `user`. `None` (`Undeterminable`) when the content is
/// oversized, not UTF-8, has too many lines, holds zero or several lines for the user, or
/// the line is malformed. The password hash is only classified, never copied.
#[must_use]
pub fn find_shadow_entry(content: &[u8], user: &UserName) -> Option<ShadowEntry> {
    if content.len() > MAX_SHADOW_BYTES {
        return None;
    }
    let text = std::str::from_utf8(content).ok()?;
    let mut found: Option<&str> = None;
    for (count, line) in text.lines().enumerate() {
        if count >= MAX_SHADOW_LINES {
            return None;
        }
        if line.split(':').next() == Some(user.as_str()) {
            if found.is_some() {
                return None;
            }
            found = Some(line);
        }
    }
    let line = found?;
    if line.split(':').count() != SHADOW_FIELDS {
        return None;
    }
    let mut fields = line.split(':').skip(1);
    let password = fields.next()?;
    let lastchg = shadow_number(fields.next()?)?;
    let _min = shadow_number(fields.next()?)?;
    let max = shadow_number(fields.next()?)?;
    let _warn = shadow_number(fields.next()?)?;
    let inactive = shadow_number(fields.next()?)?;
    let expire = shadow_number(fields.next()?)?;
    Some(ShadowEntry {
        password_locked: password.starts_with('!') || password.starts_with('*'),
        lastchg,
        max,
        inactive,
        expire,
    })
}

/// The `pam_unix` account-phase refusal of a shadow entry at `now_s`, or `None` (usable).
/// Arithmetic overflow is `Undeterminable`.
#[must_use]
pub fn shadow_refusal(entry: &ShadowEntry, now_s: u64) -> Option<AccountRefusal> {
    let Ok(today) = i64::try_from(now_s / SECONDS_PER_DAY) else {
        return Some(AccountRefusal::Undeterminable);
    };
    if let Some(expire) = entry.expire {
        if today >= expire {
            return Some(AccountRefusal::AccountExpired);
        }
    }
    if entry.lastchg == Some(0) {
        return Some(AccountRefusal::PasswordChangeForced);
    }
    if let (Some(lastchg), Some(max)) = (entry.lastchg, entry.max) {
        let Some(password_end) = lastchg.checked_add(max) else {
            return Some(AccountRefusal::Undeterminable);
        };
        if let Some(inactive) = entry.inactive {
            let Some(account_end) = password_end.checked_add(inactive) else {
                return Some(AccountRefusal::Undeterminable);
            };
            if today > account_end {
                return Some(AccountRefusal::AccountInactive);
            }
        }
        if today > password_end {
            return Some(AccountRefusal::PasswordExpired);
        }
    }
    if entry.password_locked {
        return Some(AccountRefusal::PasswordLocked);
    }
    None
}

/// Wall-clock seconds from `CLOCK_REALTIME` (tally and shadow are wall-clock based).
fn realtime_seconds() -> Result<u64, DaemonError> {
    crate::pipeline::current_monotonic_nanos_from_clock(nix::time::ClockId::CLOCK_REALTIME)
        .map(|ns| ns / 1_000_000_000)
}

/// Production account guard; every path is injectable for tests.
#[derive(Debug, Clone)]
pub struct SystemAccountGuard {
    faillock_conf: PathBuf,
    vendor_faillock_conf: PathBuf,
    default_faillock_dir: PathBuf,
    pam_dirs: Vec<PathBuf>,
    shadow: PathBuf,
    realtime: fn() -> Result<u64, DaemonError>,
}

impl Default for SystemAccountGuard {
    fn default() -> Self {
        Self::new()
    }
}

/// `Err` carries the refusal of a failed step.
type StepResult<T> = Result<T, AccountRefusal>;

/// Undeterminable shortcut for `Option`s.
fn determinable<T>(value: Option<T>) -> StepResult<T> {
    value.ok_or(AccountRefusal::Undeterminable)
}

impl SystemAccountGuard {
    /// Guard over the production paths.
    #[must_use]
    pub fn new() -> Self {
        Self {
            faillock_conf: PathBuf::from(DEFAULT_FAILLOCK_CONF),
            vendor_faillock_conf: PathBuf::from(VENDOR_FAILLOCK_CONF),
            default_faillock_dir: PathBuf::from(DEFAULT_FAILLOCK_DIR),
            pam_dirs: DEFAULT_PAM_DIRS.iter().map(PathBuf::from).collect(),
            shadow: PathBuf::from(DEFAULT_SHADOW_PATH),
            realtime: realtime_seconds,
        }
    }

    /// Replaces the `faillock.conf` path.
    #[must_use]
    pub fn with_faillock_conf(mut self, path: PathBuf) -> Self {
        self.faillock_conf = path;
        self
    }

    /// Replaces the vendor `faillock.conf` path.
    #[must_use]
    pub fn with_vendor_faillock_conf(mut self, path: PathBuf) -> Self {
        self.vendor_faillock_conf = path;
        self
    }

    /// Replaces `DEFAULT_FAILLOCK_DIR` wherever a policy leaves `dir` at its default.
    #[must_use]
    pub fn with_default_faillock_dir(mut self, path: PathBuf) -> Self {
        self.default_faillock_dir = path;
        self
    }

    /// Replaces the PAM stack directories.
    #[must_use]
    pub fn with_pam_dirs(mut self, dirs: Vec<PathBuf>) -> Self {
        self.pam_dirs = dirs;
        self
    }

    /// Replaces the shadow path.
    #[must_use]
    pub fn with_shadow(mut self, path: PathBuf) -> Self {
        self.shadow = path;
        self
    }

    /// Replaces the wall clock (seconds).
    #[must_use]
    pub fn with_realtime_fn(mut self, realtime: fn() -> Result<u64, DaemonError>) -> Self {
        self.realtime = realtime;
        self
    }

    /// Step 3: no `pam_faillock.so` line of any PAM directory sets a policy option.
    fn check_pam_stacks(&self) -> StepResult<()> {
        for dir in &self.pam_dirs {
            let entries = match std::fs::read_dir(dir) {
                Ok(entries) => entries,
                Err(err) if err.kind() == ErrorKind::NotFound => continue,
                Err(_) => return Err(AccountRefusal::Undeterminable),
            };
            let mut paths = Vec::new();
            for (count, entry) in entries.enumerate() {
                if count >= MAX_PAM_DIR_ENTRIES {
                    return Err(AccountRefusal::Undeterminable);
                }
                let entry = entry.map_err(|_| AccountRefusal::Undeterminable)?;
                paths.push(entry.path());
            }
            for path in paths {
                // Symlinks are followed like libpam does (authselect stacks); the opened
                // descriptor decides: directory skipped, regular file scanned, else refused.
                match determinable(read_source(&path, MAX_PAM_FILE_BYTES))? {
                    SourceRead::Directory => {}
                    SourceRead::Absent => return Err(AccountRefusal::Undeterminable),
                    SourceRead::Content(content) => {
                        let text = determinable(std::str::from_utf8(&content).ok())?;
                        if scan_pam_faillock_options(text) {
                            return Err(AccountRefusal::Undeterminable);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Reads and parses one `faillock.conf`; `Ok(None)` when absent.
    fn read_conf(path: &Path) -> StepResult<Option<FaillockPolicy>> {
        match determinable(read_source(path, MAX_FAILLOCK_CONF_BYTES))? {
            SourceRead::Absent => Ok(None),
            SourceRead::Directory => Err(AccountRefusal::Undeterminable),
            SourceRead::Content(content) => determinable(parse_faillock_conf(&content)).map(Some),
        }
    }

    /// Applies the injected default tally directory.
    fn resolve_dir(&self, mut policy: FaillockPolicy) -> FaillockPolicy {
        if policy.dir == Path::new(DEFAULT_FAILLOCK_DIR) {
            policy.dir.clone_from(&self.default_faillock_dir);
        }
        policy
    }

    /// Step 4: the policies to evaluate (`/etc` alone, or defaults plus vendor).
    fn policies(&self) -> StepResult<Vec<FaillockPolicy>> {
        if let Some(policy) = Self::read_conf(&self.faillock_conf)? {
            return Ok(vec![self.resolve_dir(policy)]);
        }
        let mut policies = vec![self.resolve_dir(FaillockPolicy::default())];
        if let Some(vendor) = Self::read_conf(&self.vendor_faillock_conf)? {
            policies.push(self.resolve_dir(vendor));
        }
        Ok(policies)
    }

    /// The tally directory must be a real directory (not a symlink) that is not group- or
    /// other-writable, except a root-owned sticky directory. A missing directory holds no
    /// tally (zero records).
    fn tally_dir_is_safe(dir: &Path) -> StepResult<bool> {
        let metadata = match std::fs::symlink_metadata(dir) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
            Err(_) => return Err(AccountRefusal::Undeterminable),
        };
        if !metadata.file_type().is_dir() {
            return Err(AccountRefusal::Undeterminable);
        }
        let mode = metadata.mode();
        let writable_by_others = mode & 0o022 != 0;
        let root_sticky = metadata.uid() == 0 && mode & 0o1000 != 0;
        if writable_by_others && !root_sticky {
            return Err(AccountRefusal::Undeterminable);
        }
        Ok(true)
    }

    /// Step 5: the tally under every policy.
    fn check_tally(&self, user: &UserName, now_s: u64) -> StepResult<()> {
        for policy in self.policies()? {
            if !Self::tally_dir_is_safe(&policy.dir)? {
                continue;
            }
            let records = determinable(read_tally_records(&policy.dir.join(user.as_str())))?;
            if faillock_denies(&policy, &records, now_s) {
                return Err(AccountRefusal::Faillocked);
            }
        }
        Ok(())
    }

    /// Step 6: shadow expiry.
    fn check_shadow(&self, user: &UserName, now_s: u64) -> StepResult<()> {
        let content = match determinable(read_source(&self.shadow, MAX_SHADOW_BYTES))? {
            SourceRead::Content(content) => content,
            SourceRead::Absent | SourceRead::Directory => {
                return Err(AccountRefusal::Undeterminable)
            }
        };
        let entry = determinable(find_shadow_entry(&content, user))?;
        match shadow_refusal(&entry, now_s) {
            Some(refusal) => Err(refusal),
            None => Ok(()),
        }
    }

    /// Steps 1–7 (first refusal wins).
    fn evaluate(&self, user: &UserName, uid: u32) -> StepResult<()> {
        if uid == 0 {
            return Err(AccountRefusal::RootAccount);
        }
        let now_s = (self.realtime)().map_err(|_| AccountRefusal::Undeterminable)?;
        self.check_pam_stacks()?;
        self.check_tally(user, now_s)?;
        self.check_shadow(user, now_s)
    }
}

impl AccountGuard for SystemAccountGuard {
    fn check(&self, user: &UserName, uid: u32) -> AccountState {
        match self.evaluate(user, uid) {
            Ok(()) => AccountState::Usable,
            Err(refusal) => AccountState::Refused(refusal),
        }
    }
}
