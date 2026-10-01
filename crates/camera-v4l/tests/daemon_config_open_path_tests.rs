//! The shared `daemon.toml` reader never opens a device node for reading (GitHub #291, matrix
//! row CDF1).
//!
//! Opening a character device runs its driver's `open` handler, which can have side effects (a
//! terminal becomes the controlling terminal, a camera powers up, a tape rewinds). The reader
//! first opens the path with `O_PATH` (no driver `open`), checks that handle, and only reopens a
//! regular file for reading. This test watches a private pseudo-terminal slave (a character
//! device nobody else opens) with inotify: a symbolic link to it must be refused as
//! `NotARegularFile` without any `IN_OPEN` event, while a real open of the same node is seen by
//! the watch (control). Every test uses a temporary directory, never `/etc`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stderr,
    reason = "Contractual test suite uses assertions, unwrap and a skip notice"
)]

use std::ffi::CString;
use std::fs::OpenOptions;
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::{FileTypeExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use soos_camera_v4l::daemon_config::{
    read_daemon_camera_config, DaemonCameraConfig, DaemonConfigError,
};

/// Runs the reader on another thread and fails the test if it does not return within 2 s.
fn read_with_deadline(path: &Path) -> Result<DaemonCameraConfig, DaemonConfigError> {
    let (tx, rx) = mpsc::channel();
    let owned = path.to_path_buf();
    std::thread::spawn(move || {
        let _ = tx.send(read_daemon_camera_config(&owned));
    });
    rx.recv_timeout(Duration::from_secs(2))
        .expect("the daemon.toml reader must return promptly, never block")
}

/// Opens a new pseudo-terminal master and returns it with the path of its slave node.
fn open_private_pty() -> Option<(OwnedFd, PathBuf)> {
    // SAFETY: posix_openpt takes plain flags and returns a new descriptor or -1.
    let raw = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC) };
    if raw < 0 {
        return None;
    }
    // SAFETY: `raw` is a freshly opened descriptor owned by nobody else.
    let master = unsafe { OwnedFd::from_raw_fd(raw) };
    // SAFETY: `master` is a valid pseudo-terminal master descriptor.
    if unsafe { libc::grantpt(master.as_raw_fd()) } != 0 {
        return None;
    }
    // SAFETY: as above.
    if unsafe { libc::unlockpt(master.as_raw_fd()) } != 0 {
        return None;
    }
    let mut name = [0 as libc::c_char; 128];
    // SAFETY: `name` is a writable buffer of the length passed; ptsname_r NUL-terminates it.
    if unsafe { libc::ptsname_r(master.as_raw_fd(), name.as_mut_ptr(), name.len()) } != 0 {
        return None;
    }
    // SAFETY: ptsname_r succeeded, so `name` holds a NUL-terminated string.
    let slave = unsafe { std::ffi::CStr::from_ptr(name.as_ptr()) };
    let slave = PathBuf::from(std::ffi::OsStr::from_bytes(slave.to_bytes()));
    Some((master, slave))
}

/// An inotify instance watching `IN_OPEN` on `path` (non-blocking reads).
fn watch_opens(path: &Path) -> OwnedFd {
    // SAFETY: inotify_init1 takes plain flags and returns a new descriptor or -1.
    let raw = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
    assert!(raw >= 0, "inotify_init1 failed");
    // SAFETY: `raw` is a freshly opened descriptor owned by nobody else.
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: `fd` is a valid inotify descriptor and `c_path` a NUL-terminated path.
    let wd = unsafe { libc::inotify_add_watch(fd.as_raw_fd(), c_path.as_ptr(), libc::IN_OPEN) };
    assert!(wd >= 0, "inotify_add_watch failed on {}", path.display());
    fd
}

/// Whether at least one inotify event is queued.
fn has_open_event(watch: &OwnedFd) -> bool {
    let mut buf = [0u8; 4096];
    // SAFETY: `buf` is a writable buffer of the length passed; the descriptor is non-blocking.
    let n = unsafe { libc::read(watch.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
    n > 0
}

/// CDF1: a symbolic link to a character device is refused without the device being opened.
#[test]
fn test_cdf_symlink_to_a_character_device_is_refused_without_opening_it() {
    let Some((_master, slave)) = open_private_pty() else {
        eprintln!("skipped: no pseudo-terminal available");
        return;
    };
    let file_type = std::fs::metadata(&slave).unwrap().file_type();
    assert!(
        file_type.is_char_device(),
        "{} is a character device",
        slave.display()
    );

    let watch = watch_opens(&slave);
    let dir = tempfile::tempdir().unwrap();
    let link = dir.path().join("daemon.toml");
    std::os::unix::fs::symlink(&slave, &link).unwrap();

    assert_eq!(
        read_with_deadline(&link).unwrap_err(),
        DaemonConfigError::NotARegularFile
    );
    assert!(
        !has_open_event(&watch),
        "the reader opened the character device behind the link (driver open side effects)"
    );
    // The path itself (no link) is refused the same way.
    assert_eq!(
        read_with_deadline(&slave).unwrap_err(),
        DaemonConfigError::NotARegularFile
    );
    assert!(!has_open_event(&watch), "the reader opened the device node");

    // Control: a real open of the node is seen by the watch.
    let opened = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOCTTY | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(&slave)
        .unwrap();
    assert!(
        has_open_event(&watch),
        "the inotify watch must see a real open"
    );
    drop(opened);
}

/// CDF1: a regular file behind a symbolic link is still read through the reopened handle.
#[test]
fn test_cdf_regular_file_is_read_through_the_reopened_handle() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("real.toml");
    std::fs::write(&target, "[pipeline]\ncamera_device = \"/dev/video6\"\n").unwrap();
    let watch = watch_opens(&target);
    let link = dir.path().join("daemon.toml");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let config = read_with_deadline(&link).unwrap();
    assert_eq!(
        config.settings.camera_device,
        Some(PathBuf::from("/dev/video6"))
    );
    assert!(
        has_open_event(&watch),
        "a regular file is opened for reading (once checked)"
    );
}
