//! Best-effort in-place overwrite of sensitive biometric files before removal.
//!
//! Overwriting cannot guarantee physical erasure on copy-on-write or journaling
//! filesystems, snapshots or flash storage; the real guarantee is encryption at rest
//! plus key destruction (ADR 2026-09-30, GitHub #179).
//!
//! Production template deletion goes through `BiometricStore::delete`. This helper applies
//! the same guarantees to an arbitrary path (GitHub #233, STO-17): it never follows a
//! symbolic link (`symlink_metadata` + `O_NOFOLLOW`), refuses anything but a regular file,
//! checks that the opened file is the inspected one, and overwrites it with the same number
//! of CSPRNG passes as the store.

use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use crate::error::EnrollmentCliError;

const BUFFER_SIZE: usize = 4096;

/// Number of CSPRNG overwrite passes, identical to `BiometricStore::delete`.
pub const SHRED_PASSES: usize = 3;

/// Overwrites a regular file with random bytes before unlinking it (best effort; see the
/// module documentation for what this cannot guarantee).
///
/// # Errors
///
/// - [`EnrollmentCliError::Io`] (`NotFound`) when `path` does not exist.
/// - [`EnrollmentCliError::InvalidPath`] when `path` is a symbolic link (dangling or not) or
///   is not a regular file, or when the file changed between inspection and opening. The
///   link and its target are left untouched.
/// - I/O and CSPRNG errors otherwise.
pub fn secure_shred_file<P: AsRef<Path>>(path: P) -> Result<(), EnrollmentCliError> {
    let path = path.as_ref();
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(EnrollmentCliError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("File '{}' does not exist", path.display()),
            )));
        }
        Err(e) => return Err(e.into()),
    };
    if metadata.file_type().is_symlink() {
        return Err(EnrollmentCliError::InvalidPath(format!(
            "refusing to shred symbolic link '{}'",
            path.display()
        )));
    }
    if !metadata.is_file() {
        return Err(EnrollmentCliError::InvalidPath(format!(
            "refusing to shred non-regular file '{}'",
            path.display()
        )));
    }

    // O_NOFOLLOW closes the race where the path is swapped for a link after inspection;
    // O_NONBLOCK keeps a swapped-in FIFO from blocking the open.
    let mut file = OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let opened = file.metadata()?;
    if !opened.is_file() || opened.dev() != metadata.dev() || opened.ino() != metadata.ino() {
        return Err(EnrollmentCliError::InvalidPath(format!(
            "'{}' changed while it was being shredded",
            path.display()
        )));
    }

    let file_len = opened.len();
    let mut buf = [0u8; BUFFER_SIZE];
    for _ in 0..SHRED_PASSES {
        file.seek(SeekFrom::Start(0))?;
        let mut written: u64 = 0;
        while written < file_len {
            let remaining = file_len.saturating_sub(written);
            let to_write_u64 = remaining.min(BUFFER_SIZE as u64);
            let to_write = usize::try_from(to_write_u64).unwrap_or(BUFFER_SIZE);
            if let Some(slice) = buf.get_mut(..to_write) {
                getrandom::fill(slice)
                    .map_err(|e| EnrollmentCliError::Internal(format!("CSPRNG failure: {e}")))?;
                file.write_all(slice)?;
            }
            written = written.saturating_add(to_write_u64);
        }
        file.sync_all()?;
    }
    drop(file);

    std::fs::remove_file(path)?;
    Ok(())
}
