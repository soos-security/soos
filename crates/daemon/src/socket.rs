//! Unix Domain Socket lifecycle management, validation, and binding.

use std::fs;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
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
    let file = open_and_validate_directory(dir, enforce_root_owner)?;
    drop(file);
    Ok(())
}

/// Opens the parent directory using `O_NOFOLLOW` and validates directory invariants
/// directly on the resulting file descriptor to eliminate TOCTOU directory swap races.
pub fn open_and_validate_directory(
    dir: &Path,
    enforce_root_owner: bool,
) -> Result<std::fs::File, DaemonError> {
    let sym_meta = fs::symlink_metadata(dir).map_err(|e| {
        DaemonError::SocketDirValidation(format!(
            "Failed to stat directory '{}': {e}",
            dir.display()
        ))
    })?;

    if sym_meta.file_type().is_symlink() {
        return Err(DaemonError::SocketDirValidation(format!(
            "Directory is a symlink: '{}'",
            dir.display()
        )));
    }

    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(dir)
        .map_err(|e| {
            if e.raw_os_error() == Some(libc::ELOOP) || e.raw_os_error() == Some(libc::ENOTDIR) {
                DaemonError::SocketDirValidation(format!(
                    "Directory is a symlink: '{}'",
                    dir.display()
                ))
            } else {
                DaemonError::SocketDirValidation(format!(
                    "Failed to open directory '{}': {e}",
                    dir.display()
                ))
            }
        })?;

    let stat = nix::sys::stat::fstat(file.as_raw_fd()).map_err(|e| {
        DaemonError::SocketDirValidation(format!(
            "Failed to stat directory fd for '{}': {e}",
            dir.display()
        ))
    })?;

    if (stat.st_mode & libc::S_IFMT) != libc::S_IFDIR {
        return Err(DaemonError::SocketDirValidation(format!(
            "Path is not a directory: '{}'",
            dir.display()
        )));
    }

    let mode = stat.st_mode;
    if (mode & 0o002) != 0 {
        return Err(DaemonError::SocketDirValidation(format!(
            "Directory is world-writable (mode {:o}): '{}'",
            mode & 0o777,
            dir.display()
        )));
    }

    if enforce_root_owner && stat.st_uid != 0 {
        return Err(DaemonError::SocketDirValidation(format!(
            "Directory is not owned by root (uid {}): '{}'",
            stat.st_uid,
            dir.display()
        )));
    }

    Ok(file)
}

/// Resolves the GID for the given group name.
/// If running as root and the group does not exist, attempts to create it via `groupadd --system`.
pub fn resolve_socket_group(group_name: &str) -> Result<u32, DaemonError> {
    if let Ok(Some(group)) = nix::unistd::Group::from_name(group_name) {
        return Ok(group.gid.as_raw());
    }

    if nix::unistd::geteuid().is_root() {
        debug!(group = %group_name, "Attempting to create system group");
        if let Ok(status) = std::process::Command::new("groupadd")
            .args(["--system", group_name])
            .status()
        {
            if status.success() {
                if let Ok(Some(group)) = nix::unistd::Group::from_name(group_name) {
                    info!(group = %group_name, gid = group.gid.as_raw(), "Created system group");
                    return Ok(group.gid.as_raw());
                }
            }
        }
    }

    Err(DaemonError::SocketDirValidation(format!(
        "Group '{}' not found",
        group_name
    )))
}

// DirLock removed in favor of nix::fcntl::Flock

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
/// 1. Opens and validates parent directory invariants (`O_DIRECTORY | O_NOFOLLOW`).
/// 2. Acquires exclusive lock (`flock`) on the parent directory descriptor to serialize binding.
/// 3. Checks stale socket node via `fstatat` with `AT_SYMLINK_NOFOLLOW`:
///    - If symlink: rejects immediately (prevents symlink race attacks).
///    - If non-socket: rejects immediately.
///    - If socket: unlinks descriptor-relative via `unlinkat`.
/// 4. Binds the `tokio::net::UnixListener`.
/// 5. Validates bound node is genuine socket via `fstatat`.
/// 6. Sets permissions to `socket_mode` (0660) via `fchmodat` with `NoFollowSymlink`.
/// 7. Sets group ownership (`root:soos` or configured group) via `fchownat`.
/// 8. Releases directory lock and returns listener with `SocketGuard`.
pub async fn bind_socket(
    config: &SocketConfig,
) -> Result<(UnixListener, SocketGuard), DaemonError> {
    let dir_file = open_and_validate_directory(&config.socket_dir, config.enforce_root_owner)?;
    let dir_lock = nix::fcntl::Flock::lock(dir_file, nix::fcntl::FlockArg::LockExclusiveNonblock)
        .map_err(|(_, e)| {
        DaemonError::SocketDirValidation(format!(
            "Failed to lock socket directory (another daemon may be binding): {e}"
        ))
    })?;

    let socket_name = config
        .socket_path
        .file_name()
        .map(Path::new)
        .ok_or_else(|| {
            DaemonError::SocketDirValidation(format!(
                "Invalid socket path without file name: '{}'",
                config.socket_path.display()
            ))
        })?;

    // Step 3: Check stale socket node descriptor-relative without following symlinks
    match nix::sys::stat::fstatat(
        Some(dir_lock.as_raw_fd()),
        socket_name,
        nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW,
    ) {
        Ok(stat) => {
            let file_type = stat.st_mode & libc::S_IFMT;
            if file_type == libc::S_IFLNK {
                return Err(DaemonError::SocketDirValidation(format!(
                    "Stale socket path is a symlink: '{}'",
                    config.socket_path.display()
                )));
            }
            if file_type != libc::S_IFSOCK {
                return Err(DaemonError::SocketDirValidation(format!(
                    "Existing file is not a socket: '{}'",
                    config.socket_path.display()
                )));
            }
            nix::unistd::unlinkat(
                Some(dir_lock.as_raw_fd()),
                socket_name,
                nix::unistd::UnlinkatFlags::NoRemoveDir,
            )
            .map_err(|e| {
                DaemonError::SocketDirValidation(format!(
                    "Failed to unlink stale socket '{}': {e}",
                    config.socket_path.display()
                ))
            })?;
            info!(path = %config.socket_path.display(), "Removed stale socket node");
        }
        Err(nix::errno::Errno::ENOENT) => {
            // Socket does not exist yet; nominal path
        }
        Err(err) => {
            return Err(DaemonError::SocketDirValidation(format!(
                "Failed to stat socket path '{}': {err}",
                config.socket_path.display()
            )));
        }
    }

    // Step 4: Bind the Unix domain socket listener
    let listener = UnixListener::bind(&config.socket_path)?;

    // Step 5: Post-bind verification that the newly bound path is a genuine socket
    let post_stat = nix::sys::stat::fstatat(
        Some(dir_lock.as_raw_fd()),
        socket_name,
        nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW,
    )
    .map_err(|e| {
        DaemonError::SocketDirValidation(format!(
            "Failed to stat bound socket '{}': {e}",
            config.socket_path.display()
        ))
    })?;

    if (post_stat.st_mode & libc::S_IFMT) != libc::S_IFSOCK {
        return Err(DaemonError::SocketDirValidation(format!(
            "Bound file is not a socket: '{}'",
            config.socket_path.display()
        )));
    }

    // Step 6: Enforce configured socket permissions (0660) without following symlinks
    let mode = nix::sys::stat::Mode::from_bits_truncate(config.socket_mode);
    let chmod_res = nix::sys::stat::fchmodat(
        Some(dir_lock.as_raw_fd()),
        socket_name,
        mode,
        nix::sys::stat::FchmodatFlags::NoFollowSymlink,
    );
    if let Err(err) = chmod_res {
        if err == nix::errno::Errno::EOPNOTSUPP {
            fs::set_permissions(
                &config.socket_path,
                fs::Permissions::from_mode(config.socket_mode),
            )?;
        } else {
            return Err(DaemonError::SocketDirValidation(format!(
                "Failed to set socket permissions on '{}': {err}",
                config.socket_path.display()
            )));
        }
    }

    // Step 7: Enforce socket group ownership (root:soos or configured group)
    if let Some(ref group_name) = config.socket_group {
        let is_root = nix::unistd::geteuid().is_root();
        match resolve_socket_group(group_name) {
            Ok(gid) => {
                let target_uid = if is_root || config.enforce_root_owner {
                    Some(nix::unistd::Uid::from_raw(0))
                } else {
                    None
                };
                let chown_res = nix::unistd::fchownat(
                    Some(dir_lock.as_raw_fd()),
                    socket_name,
                    target_uid,
                    Some(nix::unistd::Gid::from_raw(gid)),
                    nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW,
                );
                if let Err(err) = chown_res {
                    if is_root || config.enforce_root_owner {
                        return Err(DaemonError::SocketDirValidation(format!(
                            "Failed to set socket ownership to root:{group_name} on '{}': {err}",
                            config.socket_path.display()
                        )));
                    } else {
                        debug!(
                            error = %err,
                            group = %group_name,
                            "Unprivileged mode: skipped group chown"
                        );
                    }
                } else {
                    info!(
                        path = %config.socket_path.display(),
                        group = %group_name,
                        gid = gid,
                        "Configured socket group ownership"
                    );
                }
            }
            Err(err) => {
                if is_root || config.enforce_root_owner {
                    return Err(err);
                } else {
                    debug!(
                        error = %err,
                        group = %group_name,
                        "Unprivileged mode: socket group resolution skipped"
                    );
                }
            }
        }
    }

    info!(
        path = %config.socket_path.display(),
        mode = format!("{:o}", config.socket_mode),
        "Bound daemon Unix domain socket securely"
    );

    let guard = SocketGuard::new(config.socket_path.clone());
    Ok((listener, guard))
}
