//! Logind session validator verifying active user graphical/local sessions.
//!
//! Enforces ARCHITECTURE.md §2.3 (Invariant 3) and §4:
//! "The daemon never trusts the username, PID, PAM service, or UID declared in
//! payload messages: it strictly cross-references SO_PEERCRED and active logind
//! sessions." (No user database lookup is made: the target is a numeric UID.)
//!
//! Queries `/run/systemd/sessions/` to confirm that the asserted target UID owns
//! an active session (`ACTIVE=1` or `STATE=active`) that logind does not flag as
//! remote (`REMOTE=1`), and whether it owns a local seat session (the preview path,
//! ADR 2026-10-07 LC-3a). Facial `Auth` requests go through the stricter
//! [`crate::session_policy::LocalSessionPolicy`] (GitHub #160).

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

use crate::session_policy::SessionRecord;

/// Default systemd logind runtime sessions directory.
pub const DEFAULT_LOGIND_SESSIONS_DIR: &str = "/run/systemd/sessions";

/// Maximum allowable session file size to read into memory (4KB).
const MAX_SESSION_FILE_SIZE: u64 = 4096;

/// Validator checking whether a given UID owns an active logind session.
#[derive(Debug, Clone)]
pub struct SessionValidator {
    sessions_dir: PathBuf,
    enforce: bool,
}

impl Default for SessionValidator {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionValidator {
    /// Creates a new `SessionValidator` pointing to standard `/run/systemd/sessions` with enforcement enabled.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sessions_dir: PathBuf::from(DEFAULT_LOGIND_SESSIONS_DIR),
            enforce: true,
        }
    }

    /// Creates a new `SessionValidator` using a custom sessions directory.
    #[must_use]
    pub fn with_sessions_dir(sessions_dir: PathBuf) -> Self {
        Self {
            sessions_dir,
            enforce: true,
        }
    }

    /// Creates a disabled `SessionValidator` that permits all sessions (e.g. for mock test setups).
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            sessions_dir: PathBuf::from(DEFAULT_LOGIND_SESSIONS_DIR),
            enforce: false,
        }
    }

    /// Access the configured sessions directory path.
    #[must_use]
    pub fn sessions_dir(&self) -> &Path {
        &self.sessions_dir
    }

    /// Returns whether active session validation is enforced.
    #[must_use]
    pub const fn is_enforced(&self) -> bool {
        self.enforce
    }

    /// Checks whether the target UID currently owns an active session in `sessions_dir`.
    ///
    /// Fails closed: if `enforce` is `true` and the directory is absent, unreadable,
    /// or contains no active session file for `uid`, returns `false`.
    #[must_use]
    pub fn is_active_session(&self, uid: u32) -> bool {
        self.any_session_matches(uid, SessionRecord::is_active_non_remote_of)
    }

    /// Checks whether the target UID currently owns a local seat session
    /// ([`SessionRecord::is_local_seat_session_of`]: active, `REMOTE=0`, a seat, `CLASS=user`),
    /// the predicate of every unprivileged preview peer (ADR 2026-10-07 LC-3a).
    ///
    /// Same directory scan and fail-closed rules as [`Self::is_active_session`]; `true` when
    /// enforcement is disabled (mock harnesses only).
    #[must_use]
    pub fn has_local_seat_session(&self, uid: u32) -> bool {
        self.any_session_matches(uid, SessionRecord::is_local_seat_session_of)
    }

    /// Scans `sessions_dir` for a session record of `uid` satisfying `predicate`.
    fn any_session_matches(&self, uid: u32, predicate: fn(&SessionRecord, u32) -> bool) -> bool {
        if !self.enforce {
            debug!(
                uid = uid,
                "Session enforcement is disabled; permitting request"
            );
            return true;
        }

        if !self.sessions_dir.exists() || !self.sessions_dir.is_dir() {
            warn!(
                sessions_dir = %self.sessions_dir.display(),
                "Logind sessions directory does not exist or is not a directory; rejecting session check"
            );
            return false;
        }

        let read_dir = match fs::read_dir(&self.sessions_dir) {
            Ok(entries) => entries,
            Err(err) => {
                warn!(
                    sessions_dir = %self.sessions_dir.display(),
                    error = %err,
                    "Failed to read logind sessions directory; failing closed"
                );
                return false;
            }
        };

        for entry in read_dir {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            let path = entry.path();

            // Skip directories and reference FIFOs / pipes (e.g. "*.ref")
            if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                if file_name.ends_with(".ref") {
                    continue;
                }
            }

            let file_type = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };

            if !file_type.is_file() {
                continue;
            }

            if Self::read_session_file(&path).is_some_and(|record| predicate(&record, uid)) {
                debug!(
                    uid = uid,
                    session_file = %path.display(),
                    "Found matching logind session for UID"
                );
                return true;
            }
        }

        debug!(
            uid = uid,
            sessions_dir = %self.sessions_dir.display(),
            "No matching logind session found for UID"
        );
        false
    }

    /// Reads and parses one logind session file (bounded); `None` when it cannot be read.
    fn read_session_file(path: &Path) -> Option<SessionRecord> {
        let file = fs::File::open(path).ok()?;

        // Bound maximum read size to prevent memory exhaustion
        let mut content = String::new();
        let mut reader = file.take(MAX_SESSION_FILE_SIZE);
        reader.read_to_string(&mut content).ok()?;
        Some(SessionRecord::parse(&content))
    }
}
