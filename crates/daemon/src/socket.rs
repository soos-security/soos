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
///
/// The group check ([`validate_directory_group`]) is done by [`bind_socket`], which knows the
/// configured socket group.
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

/// Checks that the already-opened socket directory belongs to the socket group
/// (GitHub #315, review finding DMN-NEW-5).
///
/// `/run/soos` is `0750 root:<socket_group>`: the group bit is what lets members of the
/// socket group reach `daemon.sock`, so a directory of any other group would silently grant
/// that access to the wrong group. The check runs on the descriptor returned by
/// [`open_and_validate_directory`] (no path is resolved again). Under systemd the directory
/// comes from `RuntimeDirectory=soos` with `Group=soos` in `packaging/soos-daemon.service`.
///
/// # Errors
///
/// Returns [`DaemonError::SocketDirValidation`] when the directory group differs from
/// `expected_gid` or the descriptor cannot be inspected.
pub fn validate_directory_group(
    dir_file: &std::fs::File,
    dir: &Path,
    expected_gid: u32,
) -> Result<(), DaemonError> {
    let stat = nix::sys::stat::fstat(dir_file.as_raw_fd()).map_err(|e| {
        DaemonError::SocketDirValidation(format!(
            "Failed to stat directory fd for '{}': {e}",
            dir.display()
        ))
    })?;
    if stat.st_gid != expected_gid {
        return Err(DaemonError::SocketDirValidation(format!(
            "Directory group is gid {} instead of the socket group gid {expected_gid}: '{}'",
            stat.st_gid,
            dir.display()
        )));
    }
    Ok(())
}

/// Resolves the GID for the given group name.
///
/// The daemon never creates the group itself (GitHub #202, DMN-13): every packaging path
/// (`packaging/debian/postinst`, `packaging/rpm/soos.spec`, `packaging/arch/soos.install`,
/// `scripts/install.sh`) provisions it, and under `ProtectSystem=strict` a runtime
/// `groupadd` could only fail. A missing group is reported as an error.
pub fn resolve_socket_group(group_name: &str) -> Result<u32, DaemonError> {
    match nix::unistd::Group::from_name(group_name) {
        Ok(Some(group)) => Ok(group.gid.as_raw()),
        _ => Err(DaemonError::SocketDirValidation(format!(
            "Group '{}' not found",
            group_name
        ))),
    }
}

/// Changes the mode of the socket node `name` inside `dir` without ever following a symlink.
///
/// Fallback for C libraries whose `fchmodat(..., AT_SYMLINK_NOFOLLOW)` reports
/// `EOPNOTSUPP` (glibc < 2.32, musl) (GitHub #202, DMN-13). The node is pinned with
/// `O_PATH | O_NOFOLLOW` relative to the already-validated directory descriptor, its type
/// is checked on the pinned descriptor (a symlink or any non-socket is refused), and the
/// mode is changed through `/proc/self/fd/<n>`, which resolves to the pinned inode itself.
/// Fails closed when `/proc` is unavailable.
pub fn chmod_socket_node_nofollow(
    dir: &fs::File,
    name: &Path,
    mode: u32,
) -> Result<(), DaemonError> {
    let mut components = name.components();
    let single_normal = matches!(
        (components.next(), components.next()),
        (Some(std::path::Component::Normal(_)), None)
    );
    if !single_normal {
        return Err(DaemonError::SocketDirValidation(format!(
            "Invalid socket node name: '{}'",
            name.display()
        )));
    }

    let dir_proc = PathBuf::from(format!("/proc/self/fd/{}", dir.as_raw_fd()));
    let node = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(dir_proc.join(name))
        .map_err(|e| {
            DaemonError::SocketDirValidation(format!(
                "Failed to pin socket node '{}': {e}",
                name.display()
            ))
        })?;

    let stat = nix::sys::stat::fstat(node.as_raw_fd()).map_err(|e| {
        DaemonError::SocketDirValidation(format!(
            "Failed to stat pinned socket node '{}': {e}",
            name.display()
        ))
    })?;
    if (stat.st_mode & libc::S_IFMT) != libc::S_IFSOCK {
        return Err(DaemonError::SocketDirValidation(format!(
            "Refusing to chmod non-socket node '{}'",
            name.display()
        )));
    }

    let pinned = PathBuf::from(format!("/proc/self/fd/{}", node.as_raw_fd()));
    fs::set_permissions(&pinned, fs::Permissions::from_mode(mode)).map_err(|e| {
        DaemonError::SocketDirValidation(format!(
            "Failed to set permissions on pinned socket node '{}': {e}",
            name.display()
        ))
    })
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
/// 0. Validates `socket_mode` against [`crate::config::ALLOWED_SOCKET_MODES`].
/// 1. Opens and validates parent directory invariants (`O_DIRECTORY | O_NOFOLLOW`).
/// 2. Acquires exclusive lock (`flock`) on the parent directory descriptor to serialize binding.
///    When `enforce_root_owner` is set and a `socket_group` is configured, the directory must
///    also belong to that group ([`validate_directory_group`], GitHub #315).
/// 3. Checks stale socket node via `fstatat` with `AT_SYMLINK_NOFOLLOW`:
///    - If symlink: rejects immediately (prevents symlink race attacks).
///    - If non-socket: rejects immediately.
///    - If socket: unlinks descriptor-relative via `unlinkat`.
/// 4. Binds the `tokio::net::UnixListener`.
/// 5. Validates bound node is genuine socket via `fstatat`.
/// 6. Sets permissions to `socket_mode` (0660) via `fchmodat` with `NoFollowSymlink`
///    (or [`chmod_socket_node_nofollow`] where the C library lacks that flag).
/// 7. Sets group ownership (`root:soos` or configured group) via `fchownat`.
/// 8. Releases directory lock and returns listener with `SocketGuard`.
pub async fn bind_socket(
    config: &SocketConfig,
) -> Result<(UnixListener, SocketGuard), DaemonError> {
    // Step 0: refuse a world-accessible or otherwise unexpected mode before touching the
    // filesystem (GitHub #199, DMN-08).
    config.validate()?;

    let dir_file = open_and_validate_directory(&config.socket_dir, config.enforce_root_owner)?;
    let dir_lock = nix::fcntl::Flock::lock(dir_file, nix::fcntl::FlockArg::LockExclusiveNonblock)
        .map_err(|(_, e)| {
        DaemonError::SocketDirValidation(format!(
            "Failed to lock socket directory (another daemon may be binding): {e}"
        ))
    })?;

    // Step 2b: `/run/soos` must be `root:<socket_group>` (GitHub #315). Checked on the locked
    // descriptor, so the directory that was validated is the one the socket is bound in.
    if config.enforce_root_owner {
        if let Some(ref group_name) = config.socket_group {
            let gid = resolve_socket_group(group_name)?;
            validate_directory_group(&dir_lock, &config.socket_dir, gid)?;
        }
    }

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
            chmod_socket_node_nofollow(&dir_lock, socket_name, config.socket_mode)?;
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
