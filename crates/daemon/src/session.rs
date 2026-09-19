//! Logind session validator verifying active user graphical/local sessions.
//!
//! Enforces ARCHITECTURE.md §2.3 (Invariant 3) and §4:
//! "The daemon never trusts the username, PID, PAM service, or UID declared in
//! payload messages: it strictly cross-references SO_PEERCRED, /etc/passwd, and
//! active logind sessions."
//!
//! Queries `/run/systemd/sessions/` to confirm that the asserted target UID owns
//! an active local session (`ACTIVE=1` or `STATE=active`).

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

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

            if self.is_session_file_active_for_uid(&path, uid) {
                debug!(
                    uid = uid,
                    session_file = %path.display(),
                    "Found matching active logind session for UID"
                );
                return true;
            }
        }

        debug!(
            uid = uid,
            sessions_dir = %self.sessions_dir.display(),
            "No active logind session found for UID"
        );
        false
    }

    /// Reads and parses an individual logind session file to determine if it belongs
    /// to `uid` and is currently active.
    fn is_session_file_active_for_uid(&self, path: &Path, uid: u32) -> bool {
        let file = match fs::File::open(path) {
            Ok(f) => f,
            Err(_) => return false,
        };

        // Bound maximum read size to prevent memory exhaustion
        let mut content = String::new();
        let mut reader = file.take(MAX_SESSION_FILE_SIZE);
        if reader.read_to_string(&mut content).is_err() {
            return false;
        }

        let mut file_uid = None;
        let mut is_active = false;

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            if let Some(val) = trimmed.strip_prefix("UID=") {
                if let Ok(parsed) = val.trim().parse::<u32>() {
                    file_uid = Some(parsed);
                }
            } else if trimmed == "ACTIVE=1" || trimmed == "STATE=active" {
                is_active = true;
            }
        }

        file_uid == Some(uid) && is_active
    }
}
