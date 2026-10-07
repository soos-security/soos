//! Socket directory and listener preparation (spec §2.7).
//!
//! The directory is verified through an opened descriptor (`O_DIRECTORY | O_NOFOLLOW`,
//! `fstat`, `fchmod`), so a symlink or a foreign directory is refused before anything is
//! modified; a stale socket is unlinked only when it is a socket owned by the service user.
//! The `0700` parent (plus the unit's `UMask=0077`) closes the bind → chmod window.

use std::fs::{self, DirBuilder, OpenOptions, Permissions};
use std::io::ErrorKind;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

use nix::errno::Errno;
use nix::fcntl::OFlag;

/// Socket setup failure.
#[derive(Debug, thiserror::Error)]
pub enum SocketError {
    /// The parent exists and is not a directory, or is a symlink.
    #[error("socket directory is not a directory or is a symlink")]
    NotADirectory,
    /// The parent is not owned by the service user.
    #[error("socket directory is not owned by the service user")]
    WrongOwner,
    /// The socket path exists and is not a socket owned by the service user.
    #[error("socket path exists and is not a socket")]
    NotASocket,
    /// Any other I/O failure (restarted by systemd).
    #[error("socket setup failed: {0:?}")]
    Io(std::io::ErrorKind),
}

impl SocketError {
    /// Whether the failure is persistent (configuration-class, exit `EXIT_CONFIG`) rather
    /// than transient (`Io`, exit `EXIT_RUNTIME`, restarted by systemd).
    #[must_use]
    pub fn is_persistent(&self) -> bool {
        !matches!(self, Self::Io(_))
    }
}

/// Creates `parent` with mode 0700 when absent (one missing level at most: the runtime
/// directory itself must exist); when present, opens it with `O_DIRECTORY | O_NOFOLLOW`
/// (a symlink or a file is `NotADirectory`), checks the owner through `fstat`
/// (`WrongOwner`), then tightens it to `0700` through the descriptor.
///
/// # Errors
///
/// [`SocketError`].
pub fn prepare_socket_dir(parent: &Path, uid: u32) -> Result<(), SocketError> {
    match DirBuilder::new().mode(0o700).create(parent) {
        Ok(()) => return Ok(()),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
        Err(e) => return Err(SocketError::Io(e.kind())),
    }
    let dir = OpenOptions::new()
        .read(true)
        .custom_flags((OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC).bits())
        .open(parent)
        .map_err(|e| match e.raw_os_error().map(Errno::from_raw) {
            Some(Errno::ELOOP | Errno::ENOTDIR) => SocketError::NotADirectory,
            _ => SocketError::Io(e.kind()),
        })?;
    let metadata = dir.metadata().map_err(|e| SocketError::Io(e.kind()))?;
    if !metadata.is_dir() {
        return Err(SocketError::NotADirectory);
    }
    if metadata.uid() != uid {
        return Err(SocketError::WrongOwner);
    }
    dir.set_permissions(Permissions::from_mode(0o700))
        .map_err(|e| SocketError::Io(e.kind()))
}

/// Removes a stale socket at `path` only when `symlink_metadata` says it is a socket owned
/// by `uid` (anything else → `NotASocket`, never unlinked); binds the listener;
/// `set_permissions(0o600)` (on failure the fresh socket is unlinked again).
///
/// # Errors
///
/// [`SocketError`].
pub fn bind_listener(path: &Path, uid: u32) -> Result<tokio::net::UnixListener, SocketError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() && metadata.uid() == uid => {
            fs::remove_file(path).map_err(|e| SocketError::Io(e.kind()))?;
        }
        Ok(_) => return Err(SocketError::NotASocket),
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(SocketError::Io(e.kind())),
    }
    let listener =
        std::os::unix::net::UnixListener::bind(path).map_err(|e| SocketError::Io(e.kind()))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| SocketError::Io(e.kind()))?;
    if let Err(e) = fs::set_permissions(path, Permissions::from_mode(0o600)) {
        let _ = fs::remove_file(path);
        return Err(SocketError::Io(e.kind()));
    }
    tokio::net::UnixListener::from_std(listener).map_err(|e| SocketError::Io(e.kind()))
}
