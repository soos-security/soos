//! Local-session authorization policy for facial `Auth` requests (GitHub #160, DMN-09).
//!
//! RED-PHASE STUB: API only, the policy is not implemented yet.

use std::fmt::Debug;
use std::path::PathBuf;
use std::sync::Arc;

use crate::peercred::PeerCredentials;
use crate::session::SessionValidator;

/// Default procfs mount point used to map a peer PID to its logind session.
pub const DEFAULT_PROC_ROOT: &str = "/proc";

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
    /// Parses a logind session record.
    #[must_use]
    pub fn parse(_content: &str) -> Self {
        Self::default()
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
}

impl SessionDenial {
    /// Stable, non-sensitive reason code for logs.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        "stub"
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
}

/// Extracts the logind session ID from a `/proc/<pid>/cgroup` file.
#[must_use]
pub fn parse_session_id_from_cgroup(_content: &str) -> Option<String> {
    None
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
}

impl LogindSource for SystemLogind {
    fn session_id_of_pid(&self, _pid: i32) -> Result<Option<String>, LogindError> {
        let _ = (&self.sessions_dir, &self.proc_root);
        Ok(None)
    }

    fn session(&self, _session_id: &str) -> Result<Option<SessionRecord>, LogindError> {
        Ok(None)
    }

    fn sessions(&self) -> Result<Vec<SessionRecord>, LogindError> {
        Ok(Vec::new())
    }
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

    /// Builds the policy matching a [`SessionValidator`] configuration.
    #[must_use]
    pub fn from_validator(_validator: &SessionValidator) -> Self {
        Self::disabled()
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
        _peer: &PeerCredentials,
        _target_uid: u32,
    ) -> Result<(), SessionDenial> {
        Ok(())
    }
}
