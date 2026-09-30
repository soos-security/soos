//! Secure biometric persistence store providing atomic operations and permission management.
//!
//! # Erasure model (ADR 2026-09-30 "Biometric Template Erasure Model")
//!
//! Templates are AES-256-GCM ciphertext under the host master key; the guarantee that a
//! discarded template cannot be recovered rests on that encryption and on the destruction of
//! the master key, not on overwriting file blocks. Overwrite-before-release (on `delete` and on
//! re-`enroll`) is a best-effort defense-in-depth measure: it reaches the physical blocks only
//! on filesystems that rewrite data in place, and is ineffective on copy-on-write filesystems
//! (btrfs, ZFS), with data journaling, snapshots or backups, and on flash media with wear
//! levelling (SSD, eMMC).

use crate::crypto::{decrypt_payload, encrypt_payload, MasterKey};
use crate::error::BiometricStoreError;
use crate::template::BiometricTemplate;
use std::fs::{DirBuilder, File, Metadata, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// Default system storage path for biometric template files.
pub const DEFAULT_BIOMETRICS_DIR: &str = "/var/lib/soos/biometrics";

/// File extension for encrypted CBOR biometric templates.
pub const TEMPLATE_EXTENSION: &str = ".cbor.enc";

/// Mode of a store directory created by [`BiometricStore::new`] (`drwx------`).
pub const STORE_DIR_MODE: u32 = 0o700;

/// Permission bits that must be clear on an existing store directory: group-writable and
/// world-writable (`0o022`). The directory is never chmod-ed to clear them.
pub const FORBIDDEN_STORE_DIR_BITS: u32 = 0o022;

/// Number of CSPRNG overwrite passes applied before a template inode is released.
const SHRED_PASSES: usize = 3;

/// Overwrite buffer size in bytes.
const SHRED_BUFFER_SIZE: usize = 4096;

/// Biometric store responsible for managing encrypted biometric templates on disk.
#[derive(Debug)]
pub struct BiometricStore {
    base_dir: PathBuf,
    key: MasterKey,
}

impl BiometricStore {
    /// Creates a new `BiometricStore` at the specified base directory with the given master key.
    ///
    /// - If `base_dir` does not exist, it is created (missing parents with default permissions)
    ///   and the final component receives mode `0700` (`drwx------`).
    /// - If `base_dir` already exists, its permissions are **never modified**. It is validated
    ///   instead and refused with [`BiometricStoreError::InvalidPath`] unless it is a real
    ///   directory (not a symlink), owned by root or by the effective UID of this process, and
    ///   neither group- nor world-writable. A typo such as `/tmp` therefore fails loudly instead
    ///   of silently losing its sticky bit.
    pub fn new<P: AsRef<Path>>(base_dir: P, key: MasterKey) -> Result<Self, BiometricStoreError> {
        let base_dir = base_dir.as_ref().to_path_buf();
        let meta = match std::fs::symlink_metadata(&base_dir) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                create_store_dir(&base_dir)?;
                std::fs::symlink_metadata(&base_dir)?
            }
            Err(e) => return Err(BiometricStoreError::Io(e)),
        };

        validate_store_dir(
            &base_dir,
            &StoreDirFacts::from_metadata(&meta),
            nix::unistd::geteuid().as_raw(),
        )?;

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
    /// The template is serialized to CBOR (in a zeroizing buffer), encrypted with AES-256-GCM
    /// using a unique nonce, written to a temporary file created atomically with mode `0600`,
    /// synced to disk, atomically renamed over the destination, and the directory is synced.
    ///
    /// When a previous template is replaced, a handle on its inode is kept across the rename
    /// and its content is overwritten (best effort, see the module-level erasure model) once the
    /// new template is committed. If that overwrite fails, an error is returned although the
    /// new template is already in place.
    pub fn enroll(&self, template: &BiometricTemplate) -> Result<(), BiometricStoreError> {
        let cbor_bytes = template.to_cbor()?;
        let encrypted_bytes = encrypt_payload(&self.key, &cbor_bytes)?;

        let dest_path = self.template_path(template.uid)?;
        let previous = open_existing_template_for_overwrite(&dest_path)?;

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

        let mut tmp_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&tmp_path)?;
        let committed = tmp_file
            .write_all(&encrypted_bytes)
            .and_then(|()| tmp_file.sync_all())
            .and_then(|()| std::fs::rename(&tmp_path, &dest_path));
        drop(tmp_file);
        if let Err(e) = committed {
            // Best-effort cleanup of the uncommitted ciphertext; the original error is reported.
            let _ = std::fs::remove_file(&tmp_path);
            return Err(BiometricStoreError::Io(e));
        }
        sync_dir(&self.base_dir)?;

        if let Some(previous) = previous {
            overwrite_file_contents(&previous)?;
        }
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

    /// Deletes a user's enrolled biometric template file if it exists.
    ///
    /// Before unlinking, the file content is overwritten in place with CSPRNG bytes across
    /// 3 passes, each flushed with `fsync`, and the directory is synced after the unlink. This
    /// is best effort only (see the module-level erasure model): it does not reach the physical
    /// blocks on copy-on-write or data-journaling filesystems, snapshots, backups or flash media
    /// with wear levelling. The recoverability guarantee is the encryption at rest; destroying
    /// the master key renders every residual copy undecryptable.
    ///
    /// Returns `Ok(true)` if a template was overwritten and deleted, or `Ok(false)` if it did
    /// not exist.
    pub fn delete(&self, uid: u32) -> Result<bool, BiometricStoreError> {
        let path = self.template_path(uid)?;
        let Some(file) = open_existing_template_for_overwrite(&path)? else {
            return Ok(false);
        };

        overwrite_file_contents(&file)?;
        drop(file);
        std::fs::remove_file(&path)?;
        sync_dir(&self.base_dir)?;
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

/// Security-relevant facts about an existing store directory, decoupled from
/// `std::fs::Metadata` so that the validation rule is testable for owners the test process
/// cannot create.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StoreDirFacts {
    is_symlink: bool,
    is_dir: bool,
    owner_uid: u32,
    mode: u32,
}

impl StoreDirFacts {
    fn from_metadata(meta: &Metadata) -> Self {
        Self {
            is_symlink: meta.file_type().is_symlink(),
            is_dir: meta.is_dir(),
            owner_uid: meta.uid(),
            mode: meta.mode(),
        }
    }
}

/// Validates an existing store directory without modifying it.
///
/// Accepts only a real directory (not a symlink) owned by root or by `euid` whose mode has
/// neither the group-writable nor the world-writable bit set. Every other bit (setgid, sticky,
/// read and search permissions) is left as the administrator chose.
fn validate_store_dir(
    path: &Path,
    facts: &StoreDirFacts,
    euid: u32,
) -> Result<(), BiometricStoreError> {
    if facts.is_symlink {
        return Err(BiometricStoreError::InvalidPath(format!(
            "Biometrics directory '{}' is a symlink; symlinks are rejected",
            path.display()
        )));
    }
    if !facts.is_dir {
        return Err(BiometricStoreError::InvalidPath(format!(
            "Biometrics directory '{}' exists but is not a directory",
            path.display()
        )));
    }
    if facts.owner_uid != 0 && facts.owner_uid != euid {
        return Err(BiometricStoreError::InvalidPath(format!(
            "Biometrics directory '{}' is owned by UID {}; it must be owned by root or UID {euid}",
            path.display(),
            facts.owner_uid
        )));
    }
    if facts.mode & FORBIDDEN_STORE_DIR_BITS != 0 {
        return Err(BiometricStoreError::InvalidPath(format!(
            "Biometrics directory '{}' has mode {:04o}, which is group- or world-writable; \
             refusing to use it (its permissions are left unchanged)",
            path.display(),
            facts.mode & 0o7777
        )));
    }
    Ok(())
}

/// Creates a missing store directory: missing parents with default permissions, the final
/// component with mode `0700`. A directory that appears concurrently is left untouched and is
/// validated by the caller like any other existing directory.
fn create_store_dir(path: &Path) -> Result<(), BiometricStoreError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    match DirBuilder::new().mode(STORE_DIR_MODE).create(path) {
        Ok(()) => {
            // The process umask can only remove bits; set the exact mode on the directory this
            // call created (never on a pre-existing one).
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(STORE_DIR_MODE))?;
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(BiometricStoreError::Io(e)),
    }
}

/// Opens an existing template file for in-place overwrite.
///
/// Returns `Ok(None)` if the file does not exist. Refuses symlinks, non-regular files and a
/// file swapped between `lstat` and `open` (inode mismatch). `O_NONBLOCK` prevents a FIFO
/// planted at the path from blocking the caller; it has no effect on regular files.
fn open_existing_template_for_overwrite(path: &Path) -> Result<Option<File>, BiometricStoreError> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(BiometricStoreError::Io(e)),
    };
    if meta.file_type().is_symlink() {
        return Err(BiometricStoreError::InvalidPath(format!(
            "Template path '{}' is a symlink; refusing to overwrite",
            path.display()
        )));
    }
    if !meta.is_file() {
        return Err(BiometricStoreError::InvalidPath(format!(
            "Template path '{}' is not a regular file",
            path.display()
        )));
    }

    let file = match OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(BiometricStoreError::Io(e)),
    };
    let opened = file.metadata()?;
    if !opened.is_file() || opened.ino() != meta.ino() || opened.dev() != meta.dev() {
        return Err(BiometricStoreError::InvalidPath(format!(
            "Template path '{}' changed while it was being opened",
            path.display()
        )));
    }
    Ok(Some(file))
}

/// Overwrites the whole content of `file` in place with CSPRNG bytes, `SHRED_PASSES` times,
/// flushing each pass to the device with `fsync`. Best effort, see the module-level erasure
/// model.
fn overwrite_file_contents(file: &File) -> Result<(), BiometricStoreError> {
    let file_len = file.metadata()?.len();
    if file_len == 0 {
        return Ok(());
    }
    let mut writer = file;
    let mut buffer = [0u8; SHRED_BUFFER_SIZE];

    for _ in 0..SHRED_PASSES {
        writer.seek(SeekFrom::Start(0))?;
        let mut written: u64 = 0;

        while written < file_len {
            let remaining = file_len.saturating_sub(written);
            let to_write_u64 = remaining.min(SHRED_BUFFER_SIZE as u64);
            let to_write = usize::try_from(to_write_u64).unwrap_or(SHRED_BUFFER_SIZE);

            let slice = buffer.get_mut(..to_write).ok_or_else(|| {
                BiometricStoreError::Crypto("Buffer slice out of bounds".to_string())
            })?;
            getrandom::fill(slice).map_err(|e| {
                BiometricStoreError::Crypto(format!("CSPRNG failure during overwrite: {e}"))
            })?;
            writer.write_all(slice)?;
            written = written.saturating_add(to_write_u64);
        }
        writer.sync_all()?;
    }
    Ok(())
}

/// Flushes a directory's entries (a completed rename or unlink) to the device.
fn sync_dir(dir: &Path) -> Result<(), BiometricStoreError> {
    File::open(dir)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Unit tests utilize direct assertions"
)]
mod tests {
    use super::{validate_store_dir, StoreDirFacts};
    use crate::error::BiometricStoreError;
    use std::path::Path;

    const EUID: u32 = 1000;

    fn facts(owner_uid: u32, mode: u32) -> StoreDirFacts {
        StoreDirFacts {
            is_symlink: false,
            is_dir: true,
            owner_uid,
            mode: 0o040_000 | mode,
        }
    }

    fn is_invalid_path(res: Result<(), BiometricStoreError>) -> bool {
        matches!(res, Err(BiometricStoreError::InvalidPath(_)))
    }

    #[test]
    fn test_validate_store_dir_accepts_root_and_euid_owned_safe_dirs() {
        let p = Path::new("/var/lib/soos/biometrics");
        assert!(validate_store_dir(p, &facts(0, 0o700), EUID).is_ok());
        assert!(validate_store_dir(p, &facts(EUID, 0o700), EUID).is_ok());
        assert!(validate_store_dir(p, &facts(0, 0o755), EUID).is_ok());
        assert!(validate_store_dir(p, &facts(0, 0o2750), EUID).is_ok());
    }

    #[test]
    fn test_validate_store_dir_refuses_foreign_owner() {
        let p = Path::new("/srv/foreign");
        assert!(is_invalid_path(validate_store_dir(
            p,
            &facts(65534, 0o700),
            EUID
        )));
        assert!(is_invalid_path(validate_store_dir(
            p,
            &facts(EUID, 0o700),
            0
        )));
    }

    #[test]
    fn test_validate_store_dir_refuses_group_or_world_writable() {
        let p = Path::new("/tmp");
        for mode in [0o1777, 0o777, 0o770, 0o720, 0o702, 0o3770] {
            assert!(
                is_invalid_path(validate_store_dir(p, &facts(0, mode), EUID)),
                "mode {mode:o} must be refused"
            );
        }
    }

    #[test]
    fn test_validate_store_dir_refuses_symlink_and_non_directory() {
        let p = Path::new("/var/lib/soos/biometrics");
        let mut link = facts(0, 0o700);
        link.is_symlink = true;
        link.is_dir = false;
        assert!(is_invalid_path(validate_store_dir(p, &link, EUID)));

        let mut file = facts(0, 0o600);
        file.is_dir = false;
        assert!(is_invalid_path(validate_store_dir(p, &file, EUID)));
    }
}
