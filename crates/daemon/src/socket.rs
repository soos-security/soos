//! Unix Domain Socket lifecycle management, validation, and binding.

use std::fs;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use tokio::net::UnixListener;
use tracing::{debug, info, warn};

use crate::config::SocketConfig;
use crate::error::DaemonError;

/// Validates that the socket's parent directory satisfies security invariants:
/// - Not a symbolic link (prevents symlink race attacks).
/// - Is a genuine directory.
/// - Is not world-writable (`mode & 002 == 0`).
/// - Is owned by root (`uid == 0`) when `enforce_root_owner` is true.
pub fn validate_directory(dir: &Path, enforce_root_owner: bool) -> Result<(), DaemonError> {
    let meta = fs::symlink_metadata(dir).map_err(|e| {
        DaemonError::SocketDirValidation(format!(
            "Failed to stat directory '{}': {}",
            dir.display(),
            e
        ))
    })?;

    if meta.file_type().is_symlink() {
        return Err(DaemonError::SocketDirValidation(format!(
            "Directory is a symlink: '{}'",
            dir.display()
        )));
    }

    if !meta.file_type().is_dir() {
        return Err(DaemonError::SocketDirValidation(format!(
            "Path is not a directory: '{}'",
            dir.display()
        )));
    }

    let mode = meta.permissions().mode();
    if (mode & 0o002) != 0 {
        return Err(DaemonError::SocketDirValidation(format!(
            "Directory is world-writable (mode {:o}): '{}'",
            mode & 0o777,
            dir.display()
        )));
    }

    if enforce_root_owner {
        let owner_uid = meta.uid();
        if owner_uid != 0 {
            return Err(DaemonError::SocketDirValidation(format!(
                "Directory is not owned by root (uid {}): '{}'",
                owner_uid,
                dir.display()
            )));
        }
    }

    Ok(())
}

/// RAII guard ensuring the socket file is cleaned up when dropped or on shutdown.
#[derive(Debug)]
pub struct SocketGuard {
    path: PathBuf,
}

impl SocketGuard {
    /// Creates a new guard wrapping a socket path.
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Explicitly unlinks the socket file.
    pub fn unlink(&self) -> Result<(), DaemonError> {
        if self.path.exists() {
            fs::remove_file(&self.path)?;
            debug!(path = %self.path.display(), "Socket unlinked cleanly");
        }
        Ok(())
    }
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        if let Err(err) = self.unlink() {
            warn!(error = %err, path = %self.path.display(), "Failed to unlink socket on drop");
        }
    }
}

/// Prepares and binds the Unix domain socket according to `SocketConfig`.
///
/// Steps:
/// 1. Validates parent directory invariants.
/// 2. If socket file exists, checks that it is a socket (not symlink/file) and unlinks it.
/// 3. Binds the `tokio::net::UnixListener`.
/// 4. Sets permissions to `socket_mode` (0660).
pub async fn bind_socket(
    config: &SocketConfig,
) -> Result<(UnixListener, SocketGuard), DaemonError> {
    validate_directory(&config.socket_dir, config.enforce_root_owner)?;

    if config.socket_path.exists() {
        let meta = fs::symlink_metadata(&config.socket_path)?;
        if meta.file_type().is_symlink() {
            return Err(DaemonError::SocketDirValidation(format!(
                "Stale socket path is a symlink: '{}'",
                config.socket_path.display()
            )));
        }
        if !meta.file_type().is_socket() {
            return Err(DaemonError::SocketDirValidation(format!(
                "Existing file is not a socket: '{}'",
                config.socket_path.display()
            )));
        }
        fs::remove_file(&config.socket_path)?;
        info!(path = %config.socket_path.display(), "Removed stale socket node");
    }

    let listener = UnixListener::bind(&config.socket_path)?;

    // Enforce configured socket permissions (0660)
    fs::set_permissions(
        &config.socket_path,
        fs::Permissions::from_mode(config.socket_mode),
    )?;

    info!(
        path = %config.socket_path.display(),
        mode = format!("{:o}", config.socket_mode),
        "Bound daemon Unix domain socket"
    );

    let guard = SocketGuard::new(config.socket_path.clone());
    Ok((listener, guard))
}
