//! Best-effort in-place overwrite of sensitive biometric files before removal.
//!
//! Overwriting cannot guarantee physical erasure on copy-on-write or journaling
//! filesystems, snapshots or flash storage; the real guarantee is encryption at rest
//! plus key destruction (ADR 2026-09-30, GitHub #179).

use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

use crate::error::EnrollmentCliError;

const BUFFER_SIZE: usize = 4096;

/// Overwrites file contents with random bytes and zeros before unlinking (best effort;
/// see the module documentation for what this cannot guarantee).
pub fn secure_shred_file<P: AsRef<Path>>(path: P) -> Result<(), EnrollmentCliError> {
    let path = path.as_ref();
    if !path.exists() {
        return Err(EnrollmentCliError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("File '{}' does not exist", path.display()),
        )));
    }

    let metadata = std::fs::metadata(path)?;
    let file_len = metadata.len();

    if file_len > 0 {
        let mut file = OpenOptions::new().write(true).open(path)?;

        // Pass 1: Cryptographically secure random bytes
        let mut random_buf = [0u8; BUFFER_SIZE];
        let mut written: u64 = 0;

        while written < file_len {
            let remaining = file_len.saturating_sub(written);
            let to_write_u64 = remaining.min(BUFFER_SIZE as u64);
            let to_write = usize::try_from(to_write_u64).unwrap_or(BUFFER_SIZE);

            if let Some(slice) = random_buf.get_mut(..to_write) {
                getrandom::fill(slice)
                    .map_err(|e| EnrollmentCliError::Internal(format!("CSPRNG failure: {e}")))?;
                file.write_all(slice)?;
            }
            written = written.saturating_add(to_write_u64);
        }
        file.sync_all()?;

        // Pass 2: Cryptographic zeroization pass
        file.seek(SeekFrom::Start(0))?;
        let zero_buf = [0u8; BUFFER_SIZE];
        let mut written_zeros: u64 = 0;

        while written_zeros < file_len {
            let remaining = file_len.saturating_sub(written_zeros);
            let to_write_u64 = remaining.min(BUFFER_SIZE as u64);
            let to_write = usize::try_from(to_write_u64).unwrap_or(BUFFER_SIZE);

            if let Some(slice) = zero_buf.get(..to_write) {
                file.write_all(slice)?;
            }
            written_zeros = written_zeros.saturating_add(to_write_u64);
        }
        file.sync_all()?;
    }

    std::fs::remove_file(path)?;
    Ok(())
}
