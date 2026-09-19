//! Cryptographic primitives for AES-256-GCM evidence snapshot encryption.

use crate::error::EvidenceStoreError;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use std::fmt;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

/// Length of the AES-256 master key in bytes (32 bytes = 256 bits).
pub const MASTER_KEY_LEN: usize = 32;

/// Length of standard 96-bit AES-GCM nonce.
pub const NONCE_LEN: usize = 12;

/// Length of 128-bit authentication tag.
pub const TAG_LEN: usize = 16;

/// Magic header bytes identifying an encrypted evidence payload (`SOOSEVD1`).
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
            let mut file = File::open(path)?;
            let mut key_bytes = Vec::new();
            file.read_to_end(&mut key_bytes)?;
            let key = Self::from_slice(&key_bytes)?;
            key_bytes.zeroize();
            Ok(key)
        } else {
            if let Some(parent) = path.parent() {
                if !parent.exists() {
                    std::fs::create_dir_all(parent)?;
                    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
                }
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
            {
                use std::os::unix::fs::OpenOptionsExt;
                let mut tmp_file = std::fs::OpenOptions::new()
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

impl fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MasterKey([REDACTED])")
    }
}

/// Encrypts plaintext bytes using AES-256-GCM with a fresh CSPRNG nonce.
///
/// Output layout: `MAGIC_HEADER` (8 bytes) || `nonce` (12 bytes) || `ciphertext + tag`.
pub fn encrypt_payload(key: &MasterKey, plaintext: &[u8]) -> Result<Vec<u8>, EvidenceStoreError> {
    let mut nonce_bytes = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce_bytes)
        .map_err(|e| EvidenceStoreError::Crypto(format!("Failed to generate nonce: {e}")))?;

    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key.as_bytes()));
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| EvidenceStoreError::Crypto(format!("AES-GCM encryption failed: {e}")))?;

    let total_len = MAGIC_HEADER
        .len()
        .checked_add(NONCE_LEN)
        .and_then(|l| l.checked_add(ciphertext.len()))
        .ok_or_else(|| EvidenceStoreError::Crypto("Payload length overflow".to_string()))?;

    let mut output = Vec::with_capacity(total_len);
    output.extend_from_slice(MAGIC_HEADER);
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);

    Ok(output)
}

/// Decrypts an authenticated AES-256-GCM payload and returns a zeroizing buffer.
pub fn decrypt_payload(
    key: &MasterKey,
    data: &[u8],
) -> Result<Zeroizing<Vec<u8>>, EvidenceStoreError> {
    if data.len() < MIN_PAYLOAD_LEN {
        return Err(EvidenceStoreError::CorruptPayload(format!(
            "Payload length {} is smaller than minimum required length {MIN_PAYLOAD_LEN}",
            data.len()
        )));
    }

    let magic_len = MAGIC_HEADER.len();
    let magic_slice = data
        .get(0..magic_len)
        .ok_or_else(|| EvidenceStoreError::CorruptPayload("Missing magic header".to_string()))?;

    if magic_slice != MAGIC_HEADER {
        return Err(EvidenceStoreError::CorruptPayload(
            "Invalid magic header in evidence file".to_string(),
        ));
    }

    let nonce_end = magic_len
        .checked_add(NONCE_LEN)
        .ok_or_else(|| EvidenceStoreError::CorruptPayload("Nonce offset overflow".to_string()))?;

    let nonce_slice = data
        .get(magic_len..nonce_end)
        .ok_or_else(|| EvidenceStoreError::CorruptPayload("Missing nonce".to_string()))?;

    let ciphertext_slice = data
        .get(nonce_end..)
        .ok_or_else(|| EvidenceStoreError::CorruptPayload("Missing ciphertext".to_string()))?;

    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key.as_bytes()));
    let nonce = Nonce::from_slice(nonce_slice);

    let plaintext = cipher
        .decrypt(nonce, ciphertext_slice)
        .map_err(|e| EvidenceStoreError::Crypto(format!("Decryption or MAC failure: {e}")))?;

    Ok(Zeroizing::new(plaintext))
}
