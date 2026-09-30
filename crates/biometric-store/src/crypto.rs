//! Cryptographic primitives for AES-256-GCM encrypted biometric storage.

use crate::error::BiometricStoreError;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use std::fmt;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

/// Length of the AES-256 master key in bytes (32 bytes = 256 bits).
pub const MASTER_KEY_LEN: usize = 32;

/// Mode applied to every key parent directory created by [`MasterKey::load_or_create`].
///
/// Matches the `/var/lib/soos` contract (`0755`, root-owned): the models directory below it
/// must stay traversable by non-root users. The key file itself is always `0600` (GitHub #230).
pub const KEY_PARENT_DIR_MODE: u32 = 0o755;

/// Length of the AES-GCM standard initialization vector / nonce in bytes (12 bytes = 96 bits).
pub const NONCE_LEN: usize = 12;

/// Length of the AES-GCM authentication tag in bytes (16 bytes = 128 bits).
pub const TAG_LEN: usize = 16;

/// Magic header bytes identifying an encrypted biometric payload (`SOOSBIO1`).
pub const MAGIC_HEADER: &[u8; 8] = b"SOOSBIO1";

/// Minimum ciphertext length: 8 bytes magic + 12 bytes nonce + 16 bytes tag = 36 bytes.
pub const MIN_PAYLOAD_LEN: usize = 8 + NONCE_LEN + TAG_LEN;

/// Strongly-typed 256-bit cryptographic master key with automatic zeroization on drop.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct MasterKey([u8; MASTER_KEY_LEN]);

impl MasterKey {
    /// Generates a new cryptographically secure 256-bit master key using the kernel CSPRNG.
    pub fn generate() -> Result<Self, BiometricStoreError> {
        let mut key_bytes = [0u8; MASTER_KEY_LEN];
        getrandom::fill(&mut key_bytes)
            .map_err(|e| BiometricStoreError::KeyError(format!("CSPRNG error: {e}")))?;
        Ok(Self(key_bytes))
    }

    /// Constructs a `MasterKey` from a 32-byte array.
    pub fn from_bytes(bytes: [u8; MASTER_KEY_LEN]) -> Self {
        Self(bytes)
    }

    /// Constructs a `MasterKey` from a byte slice, asserting exact 32-byte length.
    pub fn from_slice(slice: &[u8]) -> Result<Self, BiometricStoreError> {
        if slice.len() != MASTER_KEY_LEN {
            return Err(BiometricStoreError::KeyError(format!(
                "Invalid key length: expected {MASTER_KEY_LEN} bytes, got {}",
                slice.len()
            )));
        }
        let mut key_bytes = [0u8; MASTER_KEY_LEN];
        key_bytes.copy_from_slice(slice);
        Ok(Self(key_bytes))
    }

    /// Returns a reference to the inner 32-byte key buffer.
    pub fn as_bytes(&self) -> &[u8; MASTER_KEY_LEN] {
        &self.0
    }

    /// Loads an existing master key from disk or generates and saves a new one with mode `0600`.
    pub fn load_or_create<P: AsRef<Path>>(path: P) -> Result<Self, BiometricStoreError> {
        let path = path.as_ref();

        // Reject symlinks targeting master key
        if let Ok(meta) = std::fs::symlink_metadata(path) {
            if meta.file_type().is_symlink() {
                return Err(BiometricStoreError::InvalidPath(format!(
                    "Master key path '{}' is a symlink; symlinks are forbidden for key files",
                    path.display()
                )));
            }
        }

        if path.exists() {
            read_existing_key(path)
        } else {
            if let Some(parent) = path.parent() {
                create_missing_key_parents(parent)?;
            }

            let key = Self::generate()?;
            let mut rand_bytes = [0u8; 8];
            getrandom::fill(&mut rand_bytes)
                .map_err(|e| BiometricStoreError::KeyError(format!("CSPRNG error: {e}")))?;
            let rand_num = u64::from_ne_bytes(rand_bytes);
            let tmp_path = format!("{}.tmp.{}.{}", path.display(), std::process::id(), rand_num);
            {
                // Atomically create file with O_CREAT | O_EXCL and explicit mode 0600
                let mut tmp_file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&tmp_path)?;
                tmp_file.write_all(key.as_bytes())?;
                tmp_file.sync_all()?;
            }
            std::fs::rename(&tmp_path, path)?;
            Ok(key)
        }
    }
}

/// Opens and validates an existing master key file (GitHub #230).
///
/// The file is opened with `O_NOFOLLOW | O_NONBLOCK` (a symlink swapped in after the
/// `symlink_metadata` pre-check fails with `ELOOP`, a FIFO never blocks) and validated on the
/// open descriptor: regular file, owned by root or by the effective UID, no group or world
/// permission bit, exactly [`MASTER_KEY_LEN`] bytes. At most `MASTER_KEY_LEN + 1` bytes are read.
fn read_existing_key(path: &Path) -> Result<MasterKey, BiometricStoreError> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let meta = file.metadata()?;
    if !meta.file_type().is_file() {
        return Err(BiometricStoreError::KeyError(format!(
            "Master key '{}' is not a regular file",
            path.display()
        )));
    }
    let euid = nix::unistd::geteuid().as_raw();
    if meta.uid() != 0 && meta.uid() != euid {
        return Err(BiometricStoreError::KeyError(format!(
            "Master key '{}' is owned by UID {}; only root or UID {euid} is trusted",
            path.display(),
            meta.uid()
        )));
    }
    if meta.mode() & 0o077 != 0 {
        return Err(BiometricStoreError::KeyError(format!(
            "Master key '{}' has mode {:o}; group or world permission bits are forbidden (expected 0600)",
            path.display(),
            meta.mode() & 0o7777
        )));
    }
    if meta.len() != MASTER_KEY_LEN as u64 {
        return Err(BiometricStoreError::KeyError(format!(
            "Invalid key length: expected {MASTER_KEY_LEN} bytes, got {}",
            meta.len()
        )));
    }
    let mut key_bytes = Zeroizing::new(Vec::with_capacity(MASTER_KEY_LEN.saturating_add(1)));
    file.take(MASTER_KEY_LEN.saturating_add(1) as u64)
        .read_to_end(&mut key_bytes)?;
    MasterKey::from_slice(&key_bytes)
}

/// Creates the missing ancestors of a key parent directory with [`KEY_PARENT_DIR_MODE`].
///
/// Directories that already exist are never modified (GitHub #230): forcing `0700` on a
/// pre-existing or shared parent such as `/var/lib/soos` broke non-root access to models.
fn create_missing_key_parents(parent: &Path) -> Result<(), BiometricStoreError> {
    if parent.as_os_str().is_empty() || std::fs::symlink_metadata(parent).is_ok() {
        return Ok(());
    }
    let mut missing = Vec::new();
    let mut cursor = Some(parent);
    while let Some(dir) = cursor {
        if dir.as_os_str().is_empty() || std::fs::symlink_metadata(dir).is_ok() {
            break;
        }
        missing.push(dir.to_path_buf());
        cursor = dir.parent();
    }
    for dir in missing.iter().rev() {
        match std::fs::DirBuilder::new()
            .mode(KEY_PARENT_DIR_MODE)
            .create(dir)
        {
            Ok(()) => {
                // Deterministic mode independent of the process umask, applied through a
                // descriptor opened without following symlinks, and only to the directory
                // this call has just created.
                let handle = OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
                    .open(dir)?;
                handle.set_permissions(std::fs::Permissions::from_mode(KEY_PARENT_DIR_MODE))?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(BiometricStoreError::Io(e)),
        }
    }
    Ok(())
}

impl fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MasterKey([REDACTED])")
    }
}

/// Encrypts plaintext bytes using AES-256-GCM with a fresh CSPRNG nonce.
///
/// Output layout: `MAGIC_HEADER` (8 bytes) || `nonce` (12 bytes) || `ciphertext + tag`.
pub fn encrypt_payload(key: &MasterKey, plaintext: &[u8]) -> Result<Vec<u8>, BiometricStoreError> {
    let mut nonce_bytes = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce_bytes)
        .map_err(|e| BiometricStoreError::Crypto(format!("Failed to generate nonce: {e}")))?;

    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key.as_bytes()));
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| BiometricStoreError::Crypto(format!("AES-GCM encryption failed: {e}")))?;

    let total_len = MAGIC_HEADER
        .len()
        .checked_add(NONCE_LEN)
        .and_then(|l| l.checked_add(ciphertext.len()))
        .ok_or_else(|| BiometricStoreError::Crypto("Payload length overflow".to_string()))?;

    let mut output = Vec::with_capacity(total_len);
    output.extend_from_slice(MAGIC_HEADER);
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);

    Ok(output)
}

/// Decrypts an authenticated AES-256-GCM payload and returns a zeroizing memory buffer.
///
/// Validates magic header, nonce, and authentication tag.
pub fn decrypt_payload(
    key: &MasterKey,
    data: &[u8],
) -> Result<Zeroizing<Vec<u8>>, BiometricStoreError> {
    if data.len() < MIN_PAYLOAD_LEN {
        return Err(BiometricStoreError::CorruptFile(format!(
            "Payload length {} is smaller than minimum required length {MIN_PAYLOAD_LEN}",
            data.len()
        )));
    }

    let magic_len = MAGIC_HEADER.len();
    let magic_slice = data
        .get(0..magic_len)
        .ok_or_else(|| BiometricStoreError::CorruptFile("Missing magic header".to_string()))?;

    if magic_slice != MAGIC_HEADER {
        return Err(BiometricStoreError::CorruptFile(
            "Invalid magic header in biometric file".to_string(),
        ));
    }

    let nonce_end = magic_len
        .checked_add(NONCE_LEN)
        .ok_or_else(|| BiometricStoreError::CorruptFile("Nonce offset overflow".to_string()))?;

    let nonce_slice = data
        .get(magic_len..nonce_end)
        .ok_or_else(|| BiometricStoreError::CorruptFile("Missing nonce".to_string()))?;

    let ciphertext_slice = data
        .get(nonce_end..)
        .ok_or_else(|| BiometricStoreError::CorruptFile("Missing ciphertext".to_string()))?;

    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key.as_bytes()));
    let nonce = Nonce::from_slice(nonce_slice);

    let plaintext = cipher
        .decrypt(nonce, ciphertext_slice)
        .map_err(|e| BiometricStoreError::Crypto(format!("Decryption or MAC failure: {e}")))?;

    Ok(Zeroizing::new(plaintext))
}

#[cfg(test)]
mod key_open_tests {
    #![allow(clippy::unwrap_used, reason = "unit tests")]

    use super::{read_existing_key, MASTER_KEY_LEN};
    use std::os::unix::fs::PermissionsExt;

    /// GitHub #230: the open itself refuses a symlink, independently of the
    /// `symlink_metadata` pre-check (a swap between the check and the open is a TOCTOU).
    #[test]
    fn test_230_key_open_does_not_follow_symlinks() {
        let tmp = tempfile::TempDir::new().unwrap();
        let target = tmp.path().join("target.key");
        std::fs::write(&target, [5u8; MASTER_KEY_LEN]).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = tmp.path().join("link.key");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        assert!(read_existing_key(&target).is_ok());
        assert!(
            read_existing_key(&link).is_err(),
            "the key open must use O_NOFOLLOW"
        );
    }
}
