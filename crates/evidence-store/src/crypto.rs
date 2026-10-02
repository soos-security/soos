//! Cryptographic primitives for AES-256-GCM evidence snapshot encryption.

use crate::error::EvidenceStoreError;
use aes_gcm::aead::{Aead, KeyInit, Payload};
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

/// Length of standard 96-bit AES-GCM nonce.
pub const NONCE_LEN: usize = 12;

/// Length of 128-bit authentication tag.
pub const TAG_LEN: usize = 16;

/// Magic header bytes identifying an encrypted evidence payload (`SOOSEVD1`).
///
/// Shared by the legacy unbound envelope and the AAD-bound envelope (GitHub #266); the format
/// is told apart by [`BOUND_FORMAT_MARKER`].
pub const MAGIC_HEADER: &[u8; 8] = b"SOOSEVD1";

/// Minimum ciphertext length: 8 bytes magic + 12 bytes nonce + 16 bytes tag = 36 bytes.
pub const MIN_PAYLOAD_LEN: usize = 8 + NONCE_LEN + TAG_LEN;

/// Cryptographic master key for evidence snapshot encryption with automatic zeroization.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct MasterKey([u8; MASTER_KEY_LEN]);

impl MasterKey {
    /// Generates a cryptographically secure 256-bit key using the kernel CSPRNG.
    pub fn generate() -> Result<Self, EvidenceStoreError> {
        let mut key_bytes = [0u8; MASTER_KEY_LEN];
        getrandom::fill(&mut key_bytes)
            .map_err(|e| EvidenceStoreError::KeyError(format!("CSPRNG error: {e}")))?;
        Ok(Self(key_bytes))
    }

    /// Constructs a `MasterKey` from a 32-byte array.
    pub fn from_bytes(bytes: [u8; MASTER_KEY_LEN]) -> Self {
        Self(bytes)
    }

    /// Constructs a `MasterKey` from a slice, verifying exact 32-byte length.
    pub fn from_slice(slice: &[u8]) -> Result<Self, EvidenceStoreError> {
        if slice.len() != MASTER_KEY_LEN {
            return Err(EvidenceStoreError::KeyError(format!(
                "Invalid key length: expected {MASTER_KEY_LEN} bytes, got {}",
                slice.len()
            )));
        }
        let mut key_bytes = [0u8; MASTER_KEY_LEN];
        key_bytes.copy_from_slice(slice);
        Ok(Self(key_bytes))
    }

    /// Returns a reference to the internal 32-byte key array.
    pub fn as_bytes(&self) -> &[u8; MASTER_KEY_LEN] {
        &self.0
    }

    /// Loads an existing master key or creates a new one on disk with mode `0600`.
    pub fn load_or_create<P: AsRef<Path>>(path: P) -> Result<Self, EvidenceStoreError> {
        let path = path.as_ref();

        // Reject symlinks targeting master key
        if let Ok(meta) = std::fs::symlink_metadata(path) {
            if meta.file_type().is_symlink() {
                return Err(EvidenceStoreError::InvalidPath(format!(
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
            getrandom::fill(&mut rand_bytes).map_err(|e| {
                EvidenceStoreError::Crypto(format!("Failed to generate random salt: {e}"))
            })?;
            let tmp_path = format!(
                "{}.tmp.{}.{:016x}",
                path.display(),
                std::process::id(),
                u64::from_ne_bytes(rand_bytes)
            );
            publish_new_key(path, Path::new(&tmp_path), key)
        }
    }
}

impl MasterKey {
    /// Loads an existing master key and never creates one (GitHub #287).
    ///
    /// Used by operator tools such as `soos-enroll migrate`, which must not mint an evidence
    /// key on a host where evidence was never enabled. A symlinked path is refused and the
    /// file is validated exactly like [`MasterKey::load_or_create`] validates an existing key;
    /// a missing file is an [`EvidenceStoreError::Io`] error of kind `NotFound`.
    pub fn load_existing<P: AsRef<Path>>(path: P) -> Result<Self, EvidenceStoreError> {
        let path = path.as_ref();
        if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
            return Err(EvidenceStoreError::InvalidPath(format!(
                "Master key path '{}' is a symlink; symlinks are forbidden for key files",
                path.display()
            )));
        }
        read_existing_key(path)
    }
}

/// Publishes a freshly generated key without ever replacing a key another caller has
/// already published (GitHub #303, STO-NEW-1).
///
/// The key is written and `fsync`ed into `tmp_path` (`O_CREAT | O_EXCL`, mode `0600`), then
/// published with `link(2)`, which fails with `EEXIST` instead of replacing an existing file
/// (unlike `rename(2)`). The loser of a concurrent first-boot race therefore reads and returns
/// the winner's key, so every caller ends up with the key that is on disk. The temporary file
/// is unlinked on every path, success or error.
fn publish_new_key(
    path: &Path,
    tmp_path: &Path,
    key: MasterKey,
) -> Result<MasterKey, EvidenceStoreError> {
    // `O_EXCL` creation failure (including a pre-existing name): nothing of ours to unlink.
    let mut tmp_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(tmp_path)?;
    let published =
        write_and_sync(&mut tmp_file, &key).and_then(|()| std::fs::hard_link(tmp_path, path));
    drop(tmp_file);
    // Best-effort cleanup on every path: the temporary name was created by this call and is
    // never read again (a failed unlink leaves only an inert `0600` copy of a key).
    let _ = std::fs::remove_file(tmp_path);
    match published.and_then(|()| sync_parent_dir(path)) {
        Ok(()) => Ok(key),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // Another caller published first: its key is authoritative.
            drop(key);
            read_existing_key(path)
        }
        Err(e) => Err(EvidenceStoreError::Io(e)),
    }
}

/// Flushes the directory entry of a just-published key, so a crash right after first-boot
/// key creation cannot lose the key while templates encrypted with it survive.
fn sync_parent_dir(path: &Path) -> std::io::Result<()> {
    match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => std::fs::File::open(dir)?.sync_all(),
        _ => Ok(()),
    }
}

/// Writes `key` into the freshly created temporary file and flushes it to stable storage.
fn write_and_sync(tmp_file: &mut std::fs::File, key: &MasterKey) -> std::io::Result<()> {
    tmp_file.write_all(key.as_bytes())?;
    tmp_file.sync_all()
}

/// Opens and validates an existing master key file (GitHub #230).
///
/// The file is opened with `O_NOFOLLOW | O_NONBLOCK` (a symlink swapped in after the
/// `symlink_metadata` pre-check fails with `ELOOP`, a FIFO never blocks) and validated on the
/// open descriptor: regular file, owned by root or by the effective UID, no group or world
/// permission bit, exactly [`MASTER_KEY_LEN`] bytes. At most `MASTER_KEY_LEN + 1` bytes are read.
fn read_existing_key(path: &Path) -> Result<MasterKey, EvidenceStoreError> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(path)?;
    let meta = file.metadata()?;
    if !meta.file_type().is_file() {
        return Err(EvidenceStoreError::KeyError(format!(
            "Evidence key '{}' is not a regular file",
            path.display()
        )));
    }
    let euid = nix::unistd::geteuid().as_raw();
    if meta.uid() != 0 && meta.uid() != euid {
        return Err(EvidenceStoreError::KeyError(format!(
            "Evidence key '{}' is owned by UID {}; only root or UID {euid} is trusted",
            path.display(),
            meta.uid()
        )));
    }
    if meta.mode() & 0o077 != 0 {
        return Err(EvidenceStoreError::KeyError(format!(
            "Evidence key '{}' has mode {:o}; group or world permission bits are forbidden (expected 0600)",
            path.display(),
            meta.mode() & 0o7777
        )));
    }
    if meta.len() != MASTER_KEY_LEN as u64 {
        return Err(EvidenceStoreError::KeyError(format!(
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
fn create_missing_key_parents(parent: &Path) -> Result<(), EvidenceStoreError> {
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
                    .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_DIRECTORY)
                    .open(dir)?;
                handle.set_permissions(std::fs::Permissions::from_mode(KEY_PARENT_DIR_MODE))?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(EvidenceStoreError::Io(e)),
        }
    }
    Ok(())
}

impl fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MasterKey([REDACTED])")
    }
}

/// Payload format version of AAD-bound evidence snapshots (GitHub #266, STO-22).
///
/// Version 1 is the legacy unbound envelope (no associated data); version 2 binds the
/// ciphertext to its file role, format version, date partition and snapshot id through AES-GCM
/// associated data. This is the envelope version, distinct from the CBOR record version
/// `EVIDENCE_RECORD_VERSION`.
pub const PAYLOAD_FORMAT_VERSION: u8 = 2;

/// Marker written right after [`MAGIC_HEADER`] by an AAD-bound (version 2) payload.
///
/// The 8-byte magic is unchanged; a legacy payload has random nonce bytes at this offset. A
/// legacy nonce that begins with the marker by chance (probability 2^-32) is still read
/// correctly because decoding falls back to the legacy layout when the bound layout does not
/// authenticate.
pub const BOUND_FORMAT_MARKER: [u8; 4] = [b'A', b'A', b'D', PAYLOAD_FORMAT_VERSION];

/// Domain label of the evidence associated data (file role: evidence snapshot).
pub const SNAPSHOT_AAD_DOMAIN: &[u8] = b"soos/evidence-snapshot";

/// Clear header of a bound snapshot: magic (8) + format marker (4).
pub const BOUND_HEADER_LEN: usize = 12;

/// Minimum length of a bound snapshot payload: clear header + nonce + tag.
pub const MIN_BOUND_PAYLOAD_LEN: usize = BOUND_HEADER_LEN + NONCE_LEN + TAG_LEN;

/// Envelope format of a decrypted payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadFormat {
    /// Legacy unbound envelope (`MAGIC_HEADER || nonce || ciphertext+tag`, no associated
    /// data), written by releases before GitHub #266. Still readable; expires with retention.
    LegacyV1,
    /// AAD-bound envelope (`MAGIC_HEADER || BOUND_FORMAT_MARKER || nonce || ciphertext+tag`)
    /// authenticated with [`snapshot_aad`].
    BoundV2,
}

/// Associated data binding a snapshot ciphertext to its date partition and snapshot id.
///
/// Layout: [`SNAPSHOT_AAD_DOMAIN`] `|| 0x00 ||` [`MAGIC_HEADER`] `||` [`BOUND_FORMAT_MARKER`]
/// `||` len(date) (big-endian u64) `||` date `||` len(id) (big-endian u64) `||` id. The length
/// prefixes keep the field boundaries unambiguous. The file suffix is deliberately not bound:
/// renaming `<id>.opaque.enc` to the historical `<id>.webp.enc` keeps the snapshot readable.
pub fn snapshot_aad(date: &str, snapshot_id: &str) -> Vec<u8> {
    let mut aad = Vec::with_capacity(
        SNAPSHOT_AAD_DOMAIN
            .len()
            .saturating_add(1 + BOUND_HEADER_LEN + 16)
            .saturating_add(date.len())
            .saturating_add(snapshot_id.len()),
    );
    aad.extend_from_slice(SNAPSHOT_AAD_DOMAIN);
    aad.push(0);
    aad.extend_from_slice(MAGIC_HEADER);
    aad.extend_from_slice(&BOUND_FORMAT_MARKER);
    for field in [date.as_bytes(), snapshot_id.as_bytes()] {
        aad.extend_from_slice(&u64::try_from(field.len()).unwrap_or(u64::MAX).to_be_bytes());
        aad.extend_from_slice(field);
    }
    aad
}

/// Generates a fresh CSPRNG nonce.
fn fresh_nonce() -> Result<[u8; NONCE_LEN], EvidenceStoreError> {
    let mut nonce_bytes = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce_bytes)
        .map_err(|e| EvidenceStoreError::Crypto(format!("Failed to generate nonce: {e}")))?;
    Ok(nonce_bytes)
}

/// AES-256-GCM encryption of `plaintext` with `aad` (empty for the legacy envelope).
fn aes_seal(
    key: &MasterKey,
    nonce_bytes: &[u8; NONCE_LEN],
    plaintext: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>, EvidenceStoreError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key.as_bytes()));
    cipher
        .encrypt(
            Nonce::from_slice(nonce_bytes),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|e| EvidenceStoreError::Crypto(format!("AES-GCM encryption failed: {e}")))
}

/// AES-256-GCM authenticated decryption with `aad`; `nonce` must be [`NONCE_LEN`] bytes.
fn aes_open(
    key: &MasterKey,
    nonce: &[u8],
    ciphertext: &[u8],
    aad: &[u8],
) -> Result<Zeroizing<Vec<u8>>, EvidenceStoreError> {
    if nonce.len() != NONCE_LEN {
        return Err(EvidenceStoreError::CorruptPayload(
            "Invalid nonce length".to_string(),
        ));
    }
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key.as_bytes()));
    cipher
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map(Zeroizing::new)
        .map_err(|e| EvidenceStoreError::Crypto(format!("Decryption or MAC failure: {e}")))
}

/// Concatenates envelope parts with an overflow-checked capacity.
fn assemble(parts: &[&[u8]]) -> Result<Vec<u8>, EvidenceStoreError> {
    let total_len = parts
        .iter()
        .try_fold(0usize, |acc, part| acc.checked_add(part.len()))
        .ok_or_else(|| EvidenceStoreError::Crypto("Payload length overflow".to_string()))?;
    let mut output = Vec::with_capacity(total_len);
    for part in parts {
        output.extend_from_slice(part);
    }
    Ok(output)
}

/// Encrypts plaintext bytes with the **legacy unbound** envelope (format version 1).
///
/// Output layout: `MAGIC_HEADER` (8 bytes) || `nonce` (12 bytes) || `ciphertext + tag`, with
/// no associated data. [`crate::EvidenceStore`] never writes this format since GitHub #266 (it
/// uses [`encrypt_snapshot_payload`]); the function is kept for context-free payloads and to
/// reproduce snapshots written by earlier releases.
pub fn encrypt_payload(key: &MasterKey, plaintext: &[u8]) -> Result<Vec<u8>, EvidenceStoreError> {
    let nonce_bytes = fresh_nonce()?;
    let ciphertext = aes_seal(key, &nonce_bytes, plaintext, &[])?;
    assemble(&[MAGIC_HEADER, &nonce_bytes, &ciphertext])
}

/// Decrypts a **legacy unbound** AES-256-GCM payload and returns a zeroizing buffer.
///
/// An AAD-bound payload written by [`encrypt_snapshot_payload`] never authenticates here.
pub fn decrypt_payload(
    key: &MasterKey,
    data: &[u8],
) -> Result<Zeroizing<Vec<u8>>, EvidenceStoreError> {
    check_envelope(data)?;
    let nonce_end = MAGIC_HEADER.len().saturating_add(NONCE_LEN);
    let nonce_slice = data
        .get(MAGIC_HEADER.len()..nonce_end)
        .ok_or_else(|| EvidenceStoreError::CorruptPayload("Missing nonce".to_string()))?;
    let ciphertext_slice = data
        .get(nonce_end..)
        .ok_or_else(|| EvidenceStoreError::CorruptPayload("Missing ciphertext".to_string()))?;
    aes_open(key, nonce_slice, ciphertext_slice, &[])
}

/// Checks the minimum envelope length and the magic header shared by both formats.
fn check_envelope(data: &[u8]) -> Result<(), EvidenceStoreError> {
    if data.len() < MIN_PAYLOAD_LEN {
        return Err(EvidenceStoreError::CorruptPayload(format!(
            "Payload length {} is smaller than minimum required length {MIN_PAYLOAD_LEN}",
            data.len()
        )));
    }
    let magic_slice = data
        .get(0..MAGIC_HEADER.len())
        .ok_or_else(|| EvidenceStoreError::CorruptPayload("Missing magic header".to_string()))?;
    if magic_slice != MAGIC_HEADER {
        return Err(EvidenceStoreError::CorruptPayload(
            "Invalid magic header in evidence file".to_string(),
        ));
    }
    Ok(())
}

/// Encrypts a snapshot record with the AAD-bound envelope (format version 2, GitHub #266).
///
/// Output layout: `MAGIC_HEADER || BOUND_FORMAT_MARKER || nonce || ciphertext + tag`,
/// authenticated with [`snapshot_aad`]`(date, snapshot_id)`.
pub fn encrypt_snapshot_payload(
    key: &MasterKey,
    date: &str,
    snapshot_id: &str,
    plaintext: &[u8],
) -> Result<Vec<u8>, EvidenceStoreError> {
    let nonce_bytes = fresh_nonce()?;
    let ciphertext = aes_seal(
        key,
        &nonce_bytes,
        plaintext,
        &snapshot_aad(date, snapshot_id),
    )?;
    assemble(&[
        MAGIC_HEADER,
        &BOUND_FORMAT_MARKER,
        &nonce_bytes,
        &ciphertext,
    ])
}

/// Decrypts a snapshot payload expected at `date` / `snapshot_id`, in either envelope format.
///
/// A bound payload must authenticate with [`snapshot_aad`]`(date, snapshot_id)`: a snapshot
/// moved to another date partition, renamed to another snapshot id or tampered with fails with
/// [`EvidenceStoreError::Crypto`]. A legacy payload (no marker) is decrypted without associated
/// data and reported as [`PayloadFormat::LegacyV1`]. When a payload carries the marker but does
/// not authenticate as bound, the legacy layout is tried as well (a legacy nonce can begin with
/// the marker by chance); if both fail, the bound authentication error is returned.
pub fn decrypt_snapshot_payload(
    key: &MasterKey,
    date: &str,
    snapshot_id: &str,
    data: &[u8],
) -> Result<(Zeroizing<Vec<u8>>, PayloadFormat), EvidenceStoreError> {
    check_envelope(data)?;
    let has_marker =
        data.get(MAGIC_HEADER.len()..BOUND_HEADER_LEN) == Some(&BOUND_FORMAT_MARKER[..]);
    if !has_marker || data.len() < MIN_BOUND_PAYLOAD_LEN {
        return decrypt_payload(key, data).map(|plain| (plain, PayloadFormat::LegacyV1));
    }
    let nonce_end = BOUND_HEADER_LEN.saturating_add(NONCE_LEN);
    let nonce = data
        .get(BOUND_HEADER_LEN..nonce_end)
        .ok_or_else(|| EvidenceStoreError::CorruptPayload("Missing nonce".to_string()))?;
    let ciphertext = data
        .get(nonce_end..)
        .ok_or_else(|| EvidenceStoreError::CorruptPayload("Missing ciphertext".to_string()))?;
    match aes_open(key, nonce, ciphertext, &snapshot_aad(date, snapshot_id)) {
        Ok(plain) => Ok((plain, PayloadFormat::BoundV2)),
        Err(bound_err) => match decrypt_payload(key, data) {
            Ok(plain) => Ok((plain, PayloadFormat::LegacyV1)),
            Err(_) => Err(bound_err),
        },
    }
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
