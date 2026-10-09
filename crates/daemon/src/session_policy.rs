//! Local-session authorization policy for facial `Auth` requests (GitHub #160, DMN-09).
//!
//! Enforces ARCHITECTURE.md §2.3 (Invariant 3) and §4: face verification runs only for
//! a target user who is physically present on a local seat, never for a remote caller.
//!
//! Policy (ADR 2026-09-30 "Local Session Binding"):
//! - **Root peer** (`su`, `sudo`, `sshd`, display-manager workers, polkit helper): the
//!   request is tied to the caller's own logind session, resolved from the kernel
//!   `SO_PEERCRED` PID through `/proc/<pid>/cgroup` (`session-<id>.scope`, the same
//!   mapping `sd_pid_get_session` uses). That session must belong to the target UID,
//!   be active, have `REMOTE=0`, be attached to a seat and have `CLASS=user`.
//!   When the caller is in no session scope but runs under the systemd user manager
//!   `user.slice/user-<uid>.slice/user@<uid>.service/` (GNOME/KDE terminals, the shell's
//!   polkit agent), `<uid>` must be strictly parsed and equal to the target, the target
//!   must own such a local seat session, and the target must own no remote session.
//! - **Unprivileged peer**: `SO_PEERCRED` already proved `peer.uid == target_uid`, so no
//!   identity is crossed; the target must still own at least one active session that
//!   logind does not flag as remote.
//! - Every lookup failure (missing PID, no session, unreadable or oversized state)
//!   denies face verification, so PAM falls back to the password. Never an allow.
//!
//! All logind access goes through [`LogindSource`] so tests stay systemd-free.

use std::ffi::OsString;
use std::fmt::Debug;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use crate::peercred::PeerCredentials;
use crate::session::SessionValidator;

/// Default procfs mount point used to map a peer PID to its logind session.
pub const DEFAULT_PROC_ROOT: &str = "/proc";

/// Maximum size of a logind session record read into memory.
pub const MAX_SESSION_RECORD_SIZE: u64 = 4096;

/// Maximum size of a `/proc/<pid>/cgroup` file read into memory. Larger files fail closed.
pub const MAX_CGROUP_FILE_SIZE: u64 = 16 * 1024;

/// Maximum length of a logind session ID (IDs are short ASCII alphanumerics, e.g. `3`, `c2`).
pub const MAX_SESSION_ID_LEN: usize = 64;

/// Maximum number of directory entries scanned in the sessions directory. More fails closed.
pub const MAX_SCANNED_SESSIONS: usize = 1024;

/// Required logind class of the caller's session for a root peer.
const REQUIRED_SESSION_CLASS: &str = "user";

/// Parsed subset of a logind runtime session record (`/run/systemd/sessions/<id>`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionRecord {
    /// Owner UID (`UID=`).
    pub uid: Option<u32>,
    /// Session is the foreground session of its seat (`ACTIVE=1` or `STATE=active`).
    pub active: bool,
    /// Remote flag (`REMOTE=0` / `REMOTE=1`); `None` when absent or malformed.
    pub remote: Option<bool>,
    /// Seat name (`SEAT=`); `None` when absent or empty.
    pub seat: Option<String>,
    /// Session class (`CLASS=`); `None` when absent or empty.
    pub class: Option<String>,
}

impl SessionRecord {
    /// Parses a logind session record. Unknown keys are ignored; malformed values stay unset.
    #[must_use]
    pub fn parse(content: &str) -> Self {
        let mut record = Self::default();
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let Some((key, value)) = trimmed.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key {
                "UID" => record.uid = value.parse::<u32>().ok(),
                "ACTIVE" if value == "1" => record.active = true,
                "STATE" if value == "active" => record.active = true,
                "REMOTE" => {
                    record.remote = match value {
                        "0" => Some(false),
                        "1" => Some(true),
                        _ => None,
                    };
                }
                "SEAT" => record.seat = non_empty(value),
                "CLASS" => record.class = non_empty(value),
                _ => {}
            }
        }
        record
    }

    /// Checks that this record is an active, local, seat-attached user session of `uid`.
    ///
    /// # Errors
    /// Returns the first failed requirement as a [`SessionDenial`].
    pub fn check_local_seat_session_of(&self, uid: u32) -> Result<(), SessionDenial> {
        if self.uid != Some(uid) {
            return Err(SessionDenial::CallerSessionForeign);
        }
        if !self.active {
            return Err(SessionDenial::CallerSessionInactive);
        }
        if self.remote != Some(false) {
            return Err(SessionDenial::CallerSessionRemote);
        }
        if self.seat.is_none() {
            return Err(SessionDenial::CallerSessionSeatless);
        }
        if self.class.as_deref() != Some(REQUIRED_SESSION_CLASS) {
            return Err(SessionDenial::CallerSessionClass);
        }
        Ok(())
    }

    /// Returns whether this record is a local seat session of `uid` that may receive preview
    /// frames (ADR 2026-10-07 LC-3a, amended): exactly
    /// `self.check_local_seat_session_of(uid).is_ok()`, i.e. `UID == uid`, active (`ACTIVE=1`
    /// or `STATE=active`), `REMOTE=0` (an absent or malformed `REMOTE` refuses), a non-empty
    /// `SEAT` and `CLASS=user`. A locked seat session qualifies (logind does not serialize
    /// `LockedHint`).
    #[must_use]
    pub fn is_local_seat_session_of(&self, uid: u32) -> bool {
        self.check_local_seat_session_of(uid).is_ok()
    }

    /// Returns whether this record is an active session of `uid` not flagged as remote.
    #[must_use]
    pub fn is_active_non_remote_of(&self, uid: u32) -> bool {
        self.uid == Some(uid) && self.active && self.remote != Some(true)
    }
}

pub(crate) fn non_empty(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// Reason a facial `Auth` request is refused by the local-session policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionDenial {
    /// Kernel `SO_PEERCRED` did not provide a usable peer PID.
    MissingPeerPid,
    /// The peer PID does not belong to any logind session.
    CallerSessionUnresolved,
    /// logind runtime state could not be read.
    LogindUnavailable,
    /// The caller's session (or the peer) belongs to another UID.
    CallerSessionForeign,
    /// The caller's session is not the active session of its seat.
    CallerSessionInactive,
    /// The caller's session is remote, or its remote flag is unknown.
    CallerSessionRemote,
    /// The caller's session is not attached to a seat.
    CallerSessionSeatless,
    /// The caller's session class is not `user`.
    CallerSessionClass,
    /// The target UID owns no active, non-remote session.
    TargetNoLocalActiveSession,
    /// The caller's cgroup names a systemd user manager with a malformed or inconsistent UID.
    UserManagerCgroupMalformed,
    /// The caller runs under the systemd user manager of a UID other than the target.
    UserManagerUidMismatch,
    /// The caller runs under the target's user manager while the target owns a remote session.
    UserManagerCallerRemoteSessionActive,
    /// The caller runs under the target's user manager but the target owns no local seat session.
    UserManagerNoLocalSeatSession,
}

impl SessionDenial {
    /// Stable, non-sensitive reason code for logs.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::MissingPeerPid => "missing_peer_pid",
            Self::CallerSessionUnresolved => "caller_session_unresolved",
            Self::LogindUnavailable => "logind_unavailable",
            Self::CallerSessionForeign => "caller_session_foreign",
            Self::CallerSessionInactive => "caller_session_inactive",
            Self::CallerSessionRemote => "caller_session_remote",
            Self::CallerSessionSeatless => "caller_session_seatless",
            Self::CallerSessionClass => "caller_session_class",
            Self::TargetNoLocalActiveSession => "target_no_local_active_session",
            Self::UserManagerCgroupMalformed => "user_manager_cgroup_malformed",
            Self::UserManagerUidMismatch => "user_manager_uid_mismatch",
            Self::UserManagerCallerRemoteSessionActive => {
                "user_manager_caller_remote_session_active"
            }
            Self::UserManagerNoLocalSeatSession => "user_manager_no_local_seat_session",
        }
    }
}

/// Error raised by a [`LogindSource`] when logind state cannot be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogindError(pub String);

/// Read-only access to logind session state (mockable).
pub trait LogindSource: Send + Sync + Debug {
    /// Returns the logind session ID that `pid` belongs to, if any.
    ///
    /// # Errors
    /// Returns [`LogindError`] when the state cannot be read.
    fn session_id_of_pid(&self, pid: i32) -> Result<Option<String>, LogindError>;

    /// Returns the session record for `session_id`, if it exists.
    ///
    /// # Errors
    /// Returns [`LogindError`] when the state cannot be read.
    fn session(&self, session_id: &str) -> Result<Option<SessionRecord>, LogindError>;

    /// Returns every session record currently known to logind (bounded).
    ///
    /// # Errors
    /// Returns [`LogindError`] when the state cannot be read.
    fn sessions(&self) -> Result<Vec<SessionRecord>, LogindError>;

    /// Returns the raw `/proc/<pid>/cgroup` content of `pid`, if the process exists.
    ///
    /// The default returns `Ok(None)`, which denies the user-manager path (fail closed).
    ///
    /// # Errors
    /// Returns [`LogindError`] when the state cannot be read.
    fn cgroup_of_pid(&self, pid: i32) -> Result<Option<String>, LogindError> {
        let _ = pid;
        Ok(None)
    }
}

/// Returns whether `id` is a syntactically valid logind session ID (ASCII alphanumeric).
pub(crate) fn is_valid_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_SESSION_ID_LEN
        && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Returns the first unit component of a cgroup path (the first non-empty component
/// that is not a `.slice`), as `cg_path_get_unit` does in systemd.
fn first_unit_component(path: &str) -> Option<&str> {
    path.split('/')
        .find(|component| !component.is_empty() && !component.ends_with(".slice"))
}

/// Returns the path of a `/proc/<pid>/cgroup` line when it belongs to a hierarchy systemd
/// manages: the unified cgroup v2 hierarchy (empty controller list) or the cgroup v1
/// `name=systemd` hierarchy. Every other line yields `None`.
pub(crate) fn systemd_hierarchy_path(line: &str) -> Option<&str> {
    let mut fields = line.splitn(3, ':');
    let (Some(_hierarchy), Some(controllers), Some(path)) =
        (fields.next(), fields.next(), fields.next())
    else {
        return None;
    };
    (controllers.is_empty() || controllers == "name=systemd").then_some(path)
}

/// Extracts the logind session ID from a `/proc/<pid>/cgroup` file.
///
/// Only the hierarchies systemd manages are read (GitHub #276): the unified cgroup v2 line
/// (`0::`, empty controller list) and the cgroup v1 `name=systemd` line, exactly like
/// [`parse_user_manager_uid_from_cgroup`]. Other cgroup v1 controller hierarchies do not
/// define the logind session and are ignored. On each considered line, only the first unit
/// below the slices counts (the rule `sd_pid_get_session` applies): a `session-<id>.scope`
/// nested inside a user manager or a service is not a session. Returns `None` when no
/// considered line carries a session scope, when an ID is malformed, or when lines disagree.
#[must_use]
pub fn parse_session_id_from_cgroup(content: &str) -> Option<String> {
    let mut found: Option<&str> = None;
    for line in content.lines() {
        let Some(path) = systemd_hierarchy_path(line) else {
            continue;
        };
        let Some(id) = first_unit_component(path).and_then(|unit| {
            unit.strip_prefix("session-")
                .and_then(|rest| rest.strip_suffix(".scope"))
        }) else {
            continue;
        };
        if !is_valid_session_id(id) {
            return None;
        }
        match found {
            Some(previous) if previous != id => return None,
            _ => found = Some(id),
        }
    }
    found.map(str::to_string)
}

/// Maximum number of decimal digits of a UID (`u32::MAX` has 10).
const MAX_UID_DIGITS: usize = 10;

/// Parses a UID strictly: ASCII decimal digits only, no sign, no leading zero, fits `u32`.
pub(crate) fn parse_strict_uid(text: &str) -> Option<u32> {
    if text.is_empty()
        || text.len() > MAX_UID_DIGITS
        || !text.bytes().all(|b| b.is_ascii_digit())
        || (text.len() > 1 && text.starts_with('0'))
    {
        return None;
    }
    text.parse::<u32>().ok()
}

/// Classifies one cgroup path: `Ok(Some(uid))` for a process below
/// `/user.slice/user-<uid>.slice/user@<uid>.service/<unit>`, `Ok(None)` for a path that is
/// not under a user manager, `Err(())` for a malformed user-manager path.
fn user_manager_uid_of_path(path: &str) -> Result<Option<u32>, ()> {
    let mut components = path.split('/');
    if components.next() != Some("") || components.next() != Some("user.slice") {
        return Ok(None);
    }
    let (Some(slice), Some(service)) = (components.next(), components.next()) else {
        return Ok(None);
    };
    if !service.starts_with("user@") {
        return Ok(None);
    }
    let slice_uid = slice
        .strip_prefix("user-")
        .and_then(|rest| rest.strip_suffix(".slice"))
        .and_then(parse_strict_uid)
        .ok_or(())?;
    let service_uid = service
        .strip_prefix("user@")
        .and_then(|rest| rest.strip_suffix(".service"))
        .and_then(parse_strict_uid)
        .ok_or(())?;
    if slice_uid != service_uid {
        return Err(());
    }
    match components.next() {
        Some(child) if !child.is_empty() => Ok(Some(service_uid)),
        _ => Err(()),
    }
}

/// Extracts the UID of the systemd user manager (`user@<uid>.service`) a process runs under.
///
/// Only the hierarchies systemd manages are considered: the unified cgroup v2 line
/// (`0::`) and the cgroup v1 `name=systemd` line. The path must be
/// `/user.slice/user-<uid>.slice/user@<uid>.service/<child>...` with both UIDs strictly
/// decimal and equal. `/user.slice` is root-owned, so an unprivileged process cannot
/// place itself under another user's manager. Returns `Ok(None)` when the process is
/// not under a user manager.
///
/// # Errors
/// Returns [`SessionDenial::UserManagerCgroupMalformed`] for a malformed user-manager
/// path or when the considered hierarchies disagree.
pub fn parse_user_manager_uid_from_cgroup(content: &str) -> Result<Option<u32>, SessionDenial> {
    let mut found: Option<Option<u32>> = None;
    for line in content.lines() {
        let Some(path) = systemd_hierarchy_path(line) else {
            continue;
        };
        let uid = user_manager_uid_of_path(path)
            .map_err(|()| SessionDenial::UserManagerCgroupMalformed)?;
        match found {
            Some(previous) if previous != uid => {
                return Err(SessionDenial::UserManagerCgroupMalformed);
            }
            _ => found = Some(uid),
        }
    }
    Ok(found.flatten())
}

/// Reads at most `max` bytes of `path` as UTF-8; `Ok(None)` if the file does not exist.
/// A file reaching the bound is reported as an error (fail closed on truncation).
pub(crate) fn read_bounded(path: &Path, max: u64) -> Result<Option<String>, LogindError> {
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(LogindError(format!("open failed: {err}"))),
    };
    let mut content = String::new();
    let read = file
        .take(max)
        .read_to_string(&mut content)
        .map_err(|err| LogindError(format!("read failed: {err}")))?;
    if u64::try_from(read).map_or(true, |n| n >= max) {
        return Err(LogindError("file exceeds bounded size".into()));
    }
    Ok(Some(content))
}

/// Production [`LogindSource`] reading `/run/systemd/sessions` and `/proc`.
#[derive(Debug, Clone)]
pub struct SystemLogind {
    sessions_dir: PathBuf,
    proc_root: PathBuf,
}

impl SystemLogind {
    /// Creates a source over explicit sessions and procfs directories.
    #[must_use]
    pub fn with_paths(sessions_dir: PathBuf, proc_root: PathBuf) -> Self {
        Self {
            sessions_dir,
            proc_root,
        }
    }

    /// Reads one session record file if it is a regular file (symlinks are not followed).
    fn read_record(path: &Path) -> Result<Option<SessionRecord>, LogindError> {
        match fs::symlink_metadata(path) {
            Ok(meta) if meta.file_type().is_file() => {}
            Ok(_) => return Ok(None),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(LogindError(format!("stat failed: {err}"))),
        }
        Ok(read_bounded(path, MAX_SESSION_RECORD_SIZE)?.map(|c| SessionRecord::parse(&c)))
    }
}

impl LogindSource for SystemLogind {
    fn session_id_of_pid(&self, pid: i32) -> Result<Option<String>, LogindError> {
        if pid <= 0 {
            return Ok(None);
        }
        let path = self.proc_root.join(pid.to_string()).join("cgroup");
        Ok(read_bounded(&path, MAX_CGROUP_FILE_SIZE)?
            .as_deref()
            .and_then(parse_session_id_from_cgroup))
    }

    fn cgroup_of_pid(&self, pid: i32) -> Result<Option<String>, LogindError> {
        if pid <= 0 {
            return Ok(None);
        }
        let path = self.proc_root.join(pid.to_string()).join("cgroup");
        read_bounded(&path, MAX_CGROUP_FILE_SIZE)
    }

    fn session(&self, session_id: &str) -> Result<Option<SessionRecord>, LogindError> {
        if !is_valid_session_id(session_id) {
            return Ok(None);
        }
        Self::read_record(&self.sessions_dir.join(session_id))
    }

    fn sessions(&self) -> Result<Vec<SessionRecord>, LogindError> {
        let entries = fs::read_dir(&self.sessions_dir)
            .map_err(|err| LogindError(format!("sessions directory unreadable: {err}")))?;
        collect_session_records(
            entries.map(|entry| entry.map(|entry| (entry.file_name(), entry.path()))),
        )
    }
}

/// Reads the session records of a sessions-directory listing given as `(name, path)` pairs.
///
/// Fails closed (GitHub #276): the user-manager path relies on this scan to prove that the
/// target owns no remote session, so an entry that cannot be read is an error, never a skip.
/// Names that are not valid session IDs (`*.ref` FIFOs, non-UTF-8 names) and entries that
/// vanished or are not regular files are skipped. More than [`MAX_SCANNED_SESSIONS`]
/// entries is an error.
///
/// # Errors
/// Returns [`LogindError`] when an entry or a record cannot be read, or the listing is too long.
pub fn collect_session_records<I>(entries: I) -> Result<Vec<SessionRecord>, LogindError>
where
    I: IntoIterator<Item = std::io::Result<(OsString, PathBuf)>>,
{
    let mut records = Vec::new();
    for (index, entry) in entries.into_iter().enumerate() {
        if index >= MAX_SCANNED_SESSIONS {
            return Err(LogindError("too many session entries".into()));
        }
        let (name, path) =
            entry.map_err(|err| LogindError(format!("session entry unreadable: {err}")))?;
        let Some(name) = name.to_str() else {
            continue;
        };
        if !is_valid_session_id(name) {
            // Skips `*.ref` FIFOs and any non-session entry.
            continue;
        }
        if let Some(record) = SystemLogind::read_record(&path)? {
            records.push(record);
        }
    }
    Ok(records)
}

/// Local-session policy applied to every facial `Auth` request.
#[derive(Debug, Clone)]
pub struct LocalSessionPolicy {
    source: Option<Arc<dyn LogindSource>>,
}

impl LocalSessionPolicy {
    /// Creates an enforcing policy over `source`.
    #[must_use]
    pub fn new(source: Arc<dyn LogindSource>) -> Self {
        Self {
            source: Some(source),
        }
    }

    /// Creates a policy that permits every request (`enforce_active_session = false`).
    #[must_use]
    pub fn disabled() -> Self {
        Self { source: None }
    }

    /// Builds the policy matching a [`SessionValidator`] configuration: same sessions
    /// directory and enforcement flag, procfs at [`DEFAULT_PROC_ROOT`].
    #[must_use]
    pub fn from_validator(validator: &SessionValidator) -> Self {
        if validator.is_enforced() {
            Self::new(Arc::new(SystemLogind::with_paths(
                validator.sessions_dir().to_path_buf(),
                PathBuf::from(DEFAULT_PROC_ROOT),
            )))
        } else {
            Self::disabled()
        }
    }

    /// Returns whether the policy is enforced.
    #[must_use]
    pub fn is_enforced(&self) -> bool {
        self.source.is_some()
    }

    /// Authorizes a facial `Auth` request from `peer` for `target_uid`.
    ///
    /// # Errors
    /// Returns the [`SessionDenial`] reason when face verification must not run.
    pub fn authorize_auth(
        &self,
        peer: &PeerCredentials,
        target_uid: u32,
    ) -> Result<(), SessionDenial> {
        let Some(source) = self.source.as_deref() else {
            debug!(
                target_uid,
                "Session enforcement is disabled; permitting request"
            );
            return Ok(());
        };
        if peer.uid == 0 {
            Self::authorize_root_peer(source, peer.pid, target_uid)
        } else if peer.uid == target_uid {
            Self::authorize_same_uid_peer(source, target_uid)
        } else {
            Err(SessionDenial::CallerSessionForeign)
        }
    }

    fn authorize_root_peer(
        source: &dyn LogindSource,
        pid: Option<i32>,
        target_uid: u32,
    ) -> Result<(), SessionDenial> {
        let pid = pid
            .filter(|p| *p > 0)
            .ok_or(SessionDenial::MissingPeerPid)?;
        let Some(session_id) = source
            .session_id_of_pid(pid)
            .map_err(|_| SessionDenial::LogindUnavailable)?
        else {
            return Self::authorize_user_manager_caller(source, pid, target_uid);
        };
        let record = source
            .session(&session_id)
            .map_err(|_| SessionDenial::LogindUnavailable)?
            .ok_or(SessionDenial::CallerSessionUnresolved)?;
        record.check_local_seat_session_of(target_uid)
    }

    /// Root peer outside any session scope: allowed only under the target's own systemd
    /// user manager, while the target owns a local seat session and no remote session
    /// (an SSH session of the same account could reach the user manager through
    /// `systemd-run --user`). A record without UID counts as possibly remote.
    fn authorize_user_manager_caller(
        source: &dyn LogindSource,
        pid: i32,
        target_uid: u32,
    ) -> Result<(), SessionDenial> {
        let content = source
            .cgroup_of_pid(pid)
            .map_err(|_| SessionDenial::LogindUnavailable)?
            .ok_or(SessionDenial::CallerSessionUnresolved)?;
        let manager_uid = parse_user_manager_uid_from_cgroup(&content)?
            .ok_or(SessionDenial::CallerSessionUnresolved)?;
        if manager_uid != target_uid {
            return Err(SessionDenial::UserManagerUidMismatch);
        }
        let records = source
            .sessions()
            .map_err(|_| SessionDenial::LogindUnavailable)?;
        if records
            .iter()
            .filter(|r| r.uid.is_none() || r.uid == Some(target_uid))
            .any(|r| r.remote != Some(false))
        {
            return Err(SessionDenial::UserManagerCallerRemoteSessionActive);
        }
        if records
            .iter()
            .any(|r| r.check_local_seat_session_of(target_uid).is_ok())
        {
            Ok(())
        } else {
            Err(SessionDenial::UserManagerNoLocalSeatSession)
        }
    }

    fn authorize_same_uid_peer(
        source: &dyn LogindSource,
        target_uid: u32,
    ) -> Result<(), SessionDenial> {
        let records = source
            .sessions()
            .map_err(|_| SessionDenial::LogindUnavailable)?;
        if records
            .iter()
            .any(|r| r.is_active_non_remote_of(target_uid))
        {
            Ok(())
        } else {
            Err(SessionDenial::TargetNoLocalActiveSession)
        }
    }
}
