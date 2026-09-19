//! Secure biometric persistence store providing atomic operations and permission management.

use crate::crypto::{decrypt_payload, encrypt_payload, MasterKey};
use crate::error::BiometricStoreError;
use crate::template::BiometricTemplate;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Default system storage path for biometric template files.
pub const DEFAULT_BIOMETRICS_DIR: &str = "/var/lib/soos/biometrics";

/// File extension for encrypted CBOR biometric templates.
pub const TEMPLATE_EXTENSION: &str = ".cbor.enc";

/// Biometric store responsible for managing encrypted biometric templates on disk.
#[derive(Debug)]
pub struct BiometricStore {
    base_dir: PathBuf,
    key: MasterKey,
}

impl BiometricStore {
    /// Creates a new `BiometricStore` at the specified base directory with the given master key.
    ///
    /// If `base_dir` does not exist, it is created with mode `0700` (`drwx------`).
    pub fn new<P: AsRef<Path>>(base_dir: P, key: MasterKey) -> Result<Self, BiometricStoreError> {
        let base_dir = base_dir.as_ref().to_path_buf();
        if let Ok(meta) = std::fs::symlink_metadata(&base_dir) {
            if meta.file_type().is_symlink() {
                return Err(BiometricStoreError::InvalidPath(format!(
                    "Biometrics directory '{}' is a symlink; symlinks are rejected",
                    base_dir.display()
                )));
            }
        }

        if !base_dir.exists() {
            std::fs::create_dir_all(&base_dir)?;
            std::fs::set_permissions(&base_dir, std::fs::Permissions::from_mode(0o700))?;
        } else {
            // Best-effort ensure mode 0700 if running with appropriate permissions
            let _ = std::fs::set_permissions(&base_dir, std::fs::Permissions::from_mode(0o700));
        }

        Ok(Self { base_dir, key })
    }

    /// Initializes a `BiometricStore` at the default path (`/var/lib/soos/biometrics`).
    pub fn with_default_path(key: MasterKey) -> Result<Self, BiometricStoreError> {
        Self::new(DEFAULT_BIOMETRICS_DIR, key)
    }

    /// Computes the absolute path for a user's biometric template file,
    /// verifying that the path is not a symlink.
    pub fn template_path(&self, uid: u32) -> Result<PathBuf, BiometricStoreError> {
        let path = self.base_dir.join(format!("{uid}{TEMPLATE_EXTENSION}"));
        match std::fs::symlink_metadata(&path) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return Err(BiometricStoreError::InvalidPath(format!(
                        "Template path '{}' is a symlink; symlinks are rejected for biometric storage",
                        path.display()
                    )));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Path does not exist, safe to return
            }
            Err(e) => return Err(BiometricStoreError::Io(e)),
        }
        Ok(path)
    }

    /// Checks whether a biometric template exists on disk for the given user ID.
    pub fn exists(&self, uid: u32) -> Result<bool, BiometricStoreError> {
        let path = self.template_path(uid)?;
        Ok(path.is_file())
    }

    /// Enrolls or updates a user's biometric template using atomic write semantics.
    ///
    /// The template is serialized to CBOR, encrypted with AES-256-GCM using a unique nonce,
    /// written to a temporary file created atomically with mode `0600`, synced to disk,
    /// and atomically renamed.
    pub fn enroll(&self, template: &BiometricTemplate) -> Result<(), BiometricStoreError> {
        let cbor_bytes = template.to_cbor()?;
        let encrypted_bytes = encrypt_payload(&self.key, &cbor_bytes)?;

        let dest_path = self.template_path(template.uid)?;
        let mut rand_suffix = [0u8; 8];
        getrandom::fill(&mut rand_suffix).map_err(|e| {
            BiometricStoreError::Crypto(format!("Failed to generate random suffix: {e}"))
        })?;
        let suffix_num = u64::from_ne_bytes(rand_suffix);

        let tmp_path = self.base_dir.join(format!(
            "{}.tmp.{}.{}",
            template.uid,
            std::process::id(),
            suffix_num
        ));

        {
            let mut tmp_file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&tmp_path)?;
            tmp_file.write_all(&encrypted_bytes)?;
            tmp_file.sync_all()?;
        }

        std::fs::rename(&tmp_path, &dest_path)?;
        Ok(())
    }

    /// Reads, decrypts, and deserializes a user's biometric template from disk.
    ///
    /// Returns `Ok(None)` if no template exists for the given user ID.
    pub fn get(&self, uid: u32) -> Result<Option<BiometricTemplate>, BiometricStoreError> {
        let path = self.template_path(uid)?;
        if !path.exists() {
            return Ok(None);
        }

        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)?;
        let mut encrypted_bytes = Vec::new();
        file.read_to_end(&mut encrypted_bytes)?;

        let decrypted_bytes = decrypt_payload(&self.key, &encrypted_bytes)?;
        let template = BiometricTemplate::from_cbor(&decrypted_bytes)?;

        if template.uid != uid {
            return Err(BiometricStoreError::CorruptFile(format!(
                "Mismatched UID in template: expected {uid}, found {}",
                template.uid
            )));
        }

        Ok(Some(template))
    }

    /// Deletes a user's enrolled biometric template file if it exists with secure 3-pass erasure.
    ///
    /// Overwrites file contents on disk with CSPRNG random bytes across 3 passes and flushes
    /// to physical media before unlinking, ensuring the deleted biometric template cannot
    /// be recovered from disk sectors.
    ///
    /// Returns `Ok(true)` if a template was securely erased and deleted, or `Ok(false)` if it did not exist.
    pub fn delete(&self, uid: u32) -> Result<bool, BiometricStoreError> {
        let path = self.template_path(uid)?;
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(BiometricStoreError::Io(e)),
        };

        if meta.file_type().is_symlink() {
            return Err(BiometricStoreError::InvalidPath(format!(
                "Template path '{}' is a symlink; refusing to delete",
                path.display()
            )));
        }

        let file_len = meta.len();
        if file_len > 0 {
            let mut file = OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&path)?;

            const BUFFER_SIZE: usize = 4096;
            const SHRED_PASSES: usize = 3;

            for _ in 0..SHRED_PASSES {
                file.seek(SeekFrom::Start(0))?;
                let mut buffer = [0u8; BUFFER_SIZE];
                let mut written: u64 = 0;

                while written < file_len {
                    let remaining = file_len.saturating_sub(written);
                    let to_write_u64 = remaining.min(BUFFER_SIZE as u64);
                    let to_write = usize::try_from(to_write_u64).unwrap_or(BUFFER_SIZE);

                    let slice = buffer.get_mut(..to_write).ok_or_else(|| {
                        BiometricStoreError::Crypto("Buffer slice out of bounds".to_string())
                    })?;
                    getrandom::fill(slice).map_err(|e| {
                        BiometricStoreError::Crypto(format!(
                            "CSPRNG failure during secure erasure: {e}"
                        ))
                    })?;
                    file.write_all(slice)?;
                    written = written.saturating_add(to_write_u64);
                }
                file.sync_all()?;
            }
        }

        std::fs::remove_file(&path)?;
        Ok(true)
    }

    /// Lists all enrolled user IDs present in the biometric store, returned in ascending order.
    pub fn list_enrolled(&self) -> Result<Vec<u32>, BiometricStoreError> {
        let mut uids = Vec::new();
        for entry in std::fs::read_dir(&self.base_dir)? {
            let entry = entry?;
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();

            if let Some(uid_str) = name.strip_suffix(TEMPLATE_EXTENSION) {
                if let Ok(uid) = uid_str.parse::<u32>() {
                    uids.push(uid);
                }
            }
        }
        uids.sort_unstable();
        Ok(uids)
    }
}
