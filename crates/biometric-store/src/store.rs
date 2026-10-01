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

use crate::crypto::{decrypt_template_payload, encrypt_template_payload, MasterKey, PayloadFormat};
use crate::error::BiometricStoreError;
use crate::template::{BiometricTemplate, TemplateMetadata};
use nix::errno::Errno;
use nix::fcntl::{Flock, FlockArg};
use std::fs::{DirBuilder, File, Metadata, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

/// Default system storage path for biometric template files.
pub const DEFAULT_BIOMETRICS_DIR: &str = "/var/lib/soos/biometrics";

/// File extension for encrypted CBOR biometric templates.
pub const TEMPLATE_EXTENSION: &str = ".cbor.enc";

/// Maximum size in bytes of an encrypted template file (GitHub #235, STO-19).
///
/// A 512-dimension template is about 4.7 KiB of CBOR plus the 36-byte envelope; 64 KiB
/// leaves room for larger embeddings (up to about 7000 dimensions) while bounding the memory a
/// corrupted or planted file can make a root process allocate. Larger files are refused with
/// [`BiometricStoreError::CorruptFile`] before any read, and [`BiometricStore::enroll`] refuses
/// to write a template that could never be read back.
pub const MAX_TEMPLATE_FILE_BYTES: u64 = 64 * 1024;

/// Mode of a store directory created by [`BiometricStore::new`] (`drwx------`).
pub const STORE_DIR_MODE: u32 = 0o700;

/// Permission bits that must be clear on an existing store directory: group-writable and
/// world-writable (`0o022`). The directory is never chmod-ed to clear them.
pub const FORBIDDEN_STORE_DIR_BITS: u32 = 0o022;

/// Default bound on the wait for the advisory store lock (GitHub #289).
pub const STORE_LOCK_TIMEOUT: Duration = Duration::from_secs(5);

/// Minimum age of an orphaned temporary file before the startup sweep removes it (GitHub #291).
pub const TEMP_SWEEP_MIN_AGE: Duration = Duration::from_secs(60);

/// Maximum number of orphaned temporary files one sweep removes (GitHub #291).
pub const MAX_TEMP_SWEEP_REMOVALS: usize = 256;

/// Maximum number of directory entries one sweep examines (GitHub #291).
const MAX_TEMP_SWEEP_SCANNED_ENTRIES: usize = 65_536;

/// Outcome of [`BiometricStore::sweep_orphaned_temp_files`] (GitHub #291).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TempSweepReport {
    /// Orphaned temporary files removed.
    pub removed: usize,
    /// Matching temporary files kept because they are younger than [`TEMP_SWEEP_MIN_AGE`].
    pub kept_recent: usize,
    /// The removal or scan bound was reached; a later sweep continues.
    pub limit_reached: bool,
    /// Another operation held the store lock: nothing was examined or removed.
    pub lock_busy: bool,
}

/// Number of CSPRNG overwrite passes applied before a template inode is released.
const SHRED_PASSES: usize = 3;

/// Overwrite buffer size in bytes.
const SHRED_BUFFER_SIZE: usize = 4096;

/// Decrypted template plaintext (zeroized on drop) and the envelope format it was read from.
type DecryptedTemplate = (Zeroizing<Vec<u8>>, PayloadFormat);

/// A [`DecryptedTemplate`] with the identity of the file it was read from.
type IdentifiedTemplate = (Zeroizing<Vec<u8>>, PayloadFormat, FileIdentity);

/// Poll interval of a contended store lock (GitHub #289).
const STORE_LOCK_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Outcome of [`BiometricStore::migrate_template`] for one UID (GitHub #287).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateMigration {
    /// No template file exists at the canonical path of the UID.
    Missing,
    /// The template already uses the AAD-bound envelope; it was not rewritten.
    AlreadyCurrent,
    /// The legacy template was re-encrypted with the AAD-bound envelope.
    Migrated,
    /// Dry run: the legacy template is valid and would be re-encrypted.
    WouldMigrate,
}

/// A template the bulk migration could not process; its file was left untouched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateMigrationFailure {
    /// UID of the template file.
    pub uid: u32,
    /// Error message (UIDs and paths only, never embedding values or key material).
    pub error: String,
}

/// Result of [`BiometricStore::migrate_legacy_templates`] (UIDs in ascending order).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TemplateMigrationReport {
    /// `true` when nothing was written.
    pub dry_run: bool,
    /// Legacy templates re-encrypted (in a dry run: that would be re-encrypted).
    pub migrated: Vec<u32>,
    /// Templates already in the AAD-bound envelope, left untouched.
    pub already_current: Vec<u32>,
    /// Templates that could not be processed, left untouched.
    pub failed: Vec<TemplateMigrationFailure>,
}

/// Biometric store responsible for managing encrypted biometric templates on disk.
#[derive(Debug)]
pub struct BiometricStore {
    base_dir: PathBuf,
    key: MasterKey,
    lock_timeout: Duration,
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
        Self::from_validated(base_dir, &meta, key)
    }

    /// Opens an existing store directory and never creates anything (GitHub #289).
    ///
    /// The directory is validated exactly like an existing directory in
    /// [`BiometricStore::new`] (real directory, owned by root or the effective UID, neither
    /// group- nor world-writable, permissions never modified). A missing directory is reported
    /// as [`BiometricStoreError::Io`] with [`std::io::ErrorKind::NotFound`]: neither the
    /// directory nor any parent is created. `soos-enroll migrate` uses it, so a directory that
    /// vanishes after its existence check is never recreated.
    pub fn open_existing<P: AsRef<Path>>(
        base_dir: P,
        key: MasterKey,
    ) -> Result<Self, BiometricStoreError> {
        let base_dir = base_dir.as_ref().to_path_buf();
        let meta = std::fs::symlink_metadata(&base_dir)?;
        Self::from_validated(base_dir, &meta, key)
    }

    /// Validates the metadata of an existing store directory and builds the store.
    fn from_validated(
        base_dir: PathBuf,
        meta: &Metadata,
        key: MasterKey,
    ) -> Result<Self, BiometricStoreError> {
        validate_store_dir(
            &base_dir,
            &StoreDirFacts::from_metadata(meta),
            nix::unistd::geteuid().as_raw(),
        )?;
        Ok(Self {
            base_dir,
            key,
            lock_timeout: STORE_LOCK_TIMEOUT,
        })
    }

    /// Sets the bound on the wait for the advisory store lock (GitHub #289).
    ///
    /// `enroll`, `delete` and a real (not dry-run) `migrate_template` take an exclusive
    /// `flock` on the store directory; a call that cannot take it within `timeout` fails with
    /// [`BiometricStoreError::LockTimeout`] and changes nothing. `Duration::ZERO` means a
    /// single attempt without waiting. The default is [`STORE_LOCK_TIMEOUT`].
    #[must_use]
    pub fn with_lock_timeout(mut self, timeout: Duration) -> Self {
        self.lock_timeout = timeout;
        self
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
    /// using a unique nonce and the AAD-bound envelope (UID, file role and format version as
    /// associated data, GitHub #266; a legacy unbound template is thereby upgraded), written
    /// to a temporary file created atomically with mode `0600`, synced to disk, atomically renamed over the destination, and the directory is synced.
    ///
    /// When a previous template is replaced, a handle on its inode is kept across the rename
    /// and its content is overwritten (best effort, see the module-level erasure model) once the
    /// new template is committed. If that overwrite fails, an error is returned although the
    /// new template is already in place.
    ///
    /// The whole write runs under the exclusive store lock (GitHub #289, see
    /// [`BiometricStore::with_lock_timeout`]), so it never interleaves with another `enroll`,
    /// `delete` or migration of this store; [`BiometricStoreError::LockTimeout`] is returned,
    /// and nothing is written, when the lock cannot be taken in time.
    pub fn enroll(&self, template: &BiometricTemplate) -> Result<(), BiometricStoreError> {
        let _lock = self.lock_store()?;
        self.write_template(template, None)
    }

    /// Enrolls `template` only if its UID has no template yet (GitHub #291).
    ///
    /// The existence check and the write run under the same exclusive store lock, so a
    /// template enrolled by another `enroll`, `import` or GUI save after an earlier check of the
    /// caller is never replaced: any entry at the template path makes the call fail with
    /// [`BiometricStoreError::AlreadyEnrolled`] and nothing is written. A symlinked template
    /// path is refused with [`BiometricStoreError::InvalidPath`] as in
    /// [`BiometricStore::template_path`]; [`BiometricStoreError::LockTimeout`] is returned,
    /// and nothing is written, when the lock cannot be taken in time. The write itself is the
    /// atomic write of [`BiometricStore::enroll`].
    pub fn enroll_if_absent(
        &self,
        template: &BiometricTemplate,
    ) -> Result<(), BiometricStoreError> {
        let _lock = self.lock_store()?;
        let path = self.template_path(template.uid)?;
        match std::fs::symlink_metadata(&path) {
            Ok(_) => return Err(BiometricStoreError::AlreadyEnrolled(template.uid)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(BiometricStoreError::Io(e)),
        }
        self.write_template(template, None)
    }

    /// Removes the temporary files an interrupted template write left in the store directory
    /// (GitHub #291); `soos-daemon` calls it once at startup.
    ///
    /// Only names of the exact form [`BiometricStore::enroll`] creates are candidates:
    /// `<uid>.tmp.<pid>.<suffix>` with canonical decimal numbers (`u32`, `u32`, `u64`). A
    /// candidate is removed only if it is a regular file (never a symlink, directory or FIFO;
    /// nothing is followed) with a single link, owned by root or the effective UID, and last
    /// modified at least [`TEMP_SWEEP_MIN_AGE`] ago (a future modification time is never
    /// old). Every check and the unlink are relative to a descriptor of the store directory
    /// opened `O_DIRECTORY | O_NOFOLLOW`. At most [`MAX_TEMP_SWEEP_REMOVALS`] files are removed
    /// and at most `MAX_TEMP_SWEEP_SCANNED_ENTRIES` entries examined per call
    /// ([`TempSweepReport::limit_reached`]). The store lock is tried once, never waited for:
    /// while another operation holds it nothing is examined ([`TempSweepReport::lock_busy`]).
    ///
    /// # Errors
    ///
    /// [`BiometricStoreError::Io`] when the store directory cannot be opened, listed or
    /// unlinked from, [`BiometricStoreError::InvalidPath`] when its path no longer names the
    /// opened directory.
    pub fn sweep_orphaned_temp_files(&self) -> Result<TempSweepReport, BiometricStoreError> {
        let dir = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(&self.base_dir)?;
        let lock = match Flock::lock(dir, FlockArg::LockExclusiveNonblock) {
            Ok(lock) => lock,
            Err((_, Errno::EAGAIN | Errno::EINTR)) => {
                return Ok(TempSweepReport {
                    lock_busy: true,
                    ..TempSweepReport::default()
                })
            }
            Err((_, errno)) => return Err(BiometricStoreError::Io(std::io::Error::from(errno))),
        };
        let opened = lock.metadata()?;
        let listed = std::fs::symlink_metadata(&self.base_dir)?;
        if listed.dev() != opened.dev() || listed.ino() != opened.ino() {
            return Err(BiometricStoreError::InvalidPath(format!(
                "Biometrics directory '{}' changed while it was being swept",
                self.base_dir.display()
            )));
        }

        let euid = nix::unistd::geteuid().as_raw();
        let now = std::time::SystemTime::now();
        let mut report = TempSweepReport::default();
        let mut scanned: usize = 0;
        for entry in std::fs::read_dir(&self.base_dir)? {
            if report.removed >= MAX_TEMP_SWEEP_REMOVALS
                || scanned >= MAX_TEMP_SWEEP_SCANNED_ENTRIES
            {
                report.limit_reached = true;
                break;
            }
            scanned = scanned.saturating_add(1);
            let name = entry?.file_name();
            if !name.to_str().is_some_and(is_template_temp_name) {
                continue;
            }
            match sweep_candidate(lock.as_raw_fd(), name.as_os_str(), euid, now)? {
                SweepVerdict::Removed => report.removed = report.removed.saturating_add(1),
                SweepVerdict::Recent => report.kept_recent = report.kept_recent.saturating_add(1),
                SweepVerdict::Kept => {}
            }
        }
        if report.removed > 0 {
            lock.sync_all()?;
        }
        Ok(report)
    }

    /// Writes `template` (the body of [`BiometricStore::enroll`]); the caller holds the store
    /// lock. With `expected`, the destination must still be the file a migration read: a
    /// missing, replaced or rewritten file is refused with
    /// [`BiometricStoreError::ChangedConcurrently`] before the rename and left as found.
    fn write_template(
        &self,
        template: &BiometricTemplate,
        expected: Option<&FileIdentity>,
    ) -> Result<(), BiometricStoreError> {
        let cbor_bytes = template.to_cbor()?;
        let encrypted_bytes = encrypt_template_payload(&self.key, template.uid, &cbor_bytes)?;
        let encrypted_len = u64::try_from(encrypted_bytes.len()).unwrap_or(u64::MAX);
        if encrypted_len > MAX_TEMPLATE_FILE_BYTES {
            return Err(BiometricStoreError::InvalidMetadata(format!(
                "encrypted template of {encrypted_len} bytes exceeds the \
                 {MAX_TEMPLATE_FILE_BYTES}-byte template file limit"
            )));
        }

        let dest_path = self.template_path(template.uid)?;
        let previous = open_existing_template_for_overwrite(&dest_path)?;
        if let Some(expected) = expected {
            let unchanged = match previous.as_ref() {
                Some(file) => expected.matches(&file.metadata()?),
                None => false,
            };
            if !unchanged {
                return Err(changed_concurrently(template.uid));
            }
        }

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
        let written = tmp_file
            .write_all(&encrypted_bytes)
            .and_then(|()| tmp_file.sync_all());
        drop(tmp_file);
        let committed = written.map_err(BiometricStoreError::Io).and_then(|()| {
            // Rename only over the file that was read: a path swapped meanwhile by a writer
            // that bypasses the store lock is left as it is.
            if let Some(expected) = expected {
                if !expected.still_at(&dest_path) {
                    return Err(changed_concurrently(template.uid));
                }
            }
            std::fs::rename(&tmp_path, &dest_path).map_err(BiometricStoreError::Io)
        });
        if let Err(e) = committed {
            // Best-effort cleanup of the uncommitted ciphertext; the original error is reported.
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e);
        }
        sync_dir(&self.base_dir)?;

        if let Some(previous) = previous {
            overwrite_file_contents(&previous)?;
        }
        Ok(())
    }

    /// Reads, decrypts, and deserializes a user's biometric template from disk.
    ///
    /// Returns `Ok(None)` if no template exists for the given user ID. A file larger than
    /// [`MAX_TEMPLATE_FILE_BYTES`] is refused with [`BiometricStoreError::CorruptFile`] before
    /// it is read.
    pub fn get(&self, uid: u32) -> Result<Option<BiometricTemplate>, BiometricStoreError> {
        let Some((decrypted_bytes, _format)) = self.read_decrypted(uid)? else {
            return Ok(None);
        };
        let template = BiometricTemplate::from_cbor(&decrypted_bytes)?;
        check_uid(uid, template.uid)?;
        Ok(Some(template))
    }

    /// Reads a user's template metadata without materialising the embedding vector
    /// (GitHub #235, STO-19).
    ///
    /// The file is size-bounded and authenticated exactly like [`BiometricStore::get`]; the
    /// decrypted CBOR (held in a zeroizing buffer) is validated by
    /// [`TemplateMetadata::from_cbor`], which checks the embedding array shape and values
    /// without allocating it. Returns `Ok(None)` if no template exists.
    pub fn get_metadata(&self, uid: u32) -> Result<Option<TemplateMetadata>, BiometricStoreError> {
        let Some((decrypted_bytes, _format)) = self.read_decrypted(uid)? else {
            return Ok(None);
        };
        let metadata = TemplateMetadata::from_cbor(&decrypted_bytes)?;
        check_uid(uid, metadata.uid)?;
        Ok(Some(metadata))
    }

    /// Reports the envelope format of a user's template (GitHub #266, STO-22).
    ///
    /// The file is size-bounded, authenticated and its metadata validated (including the UID
    /// check) exactly like [`BiometricStore::get_metadata`]. Returns `Ok(None)` if no template
    /// exists, [`PayloadFormat::LegacyV1`] for a template written before AAD binding and
    /// [`PayloadFormat::BoundV2`] otherwise.
    pub fn template_format(&self, uid: u32) -> Result<Option<PayloadFormat>, BiometricStoreError> {
        let Some((decrypted_bytes, format)) = self.read_decrypted(uid)? else {
            return Ok(None);
        };
        let metadata = TemplateMetadata::from_cbor(&decrypted_bytes)?;
        check_uid(uid, metadata.uid)?;
        Ok(Some(format))
    }

    /// Re-encrypts a legacy unbound template with the AAD-bound envelope (GitHub #266, STO-22).
    ///
    /// Returns `Ok(false)` when no template exists or it is already bound, `Ok(true)` once a
    /// legacy template has been rewritten through [`BiometricStore::enroll`] (atomic replace,
    /// best-effort overwrite of the legacy inode). A legacy template whose embedded UID does not
    /// match its file name is refused with [`BiometricStoreError::CorruptFile`] and left
    /// untouched. The migration is serialized with `enroll` and `delete` by the store lock and
    /// never rewrites a file that changed after it was read (see
    /// [`BiometricStore::migrate_template`]).
    pub fn migrate_legacy_template(&self, uid: u32) -> Result<bool, BiometricStoreError> {
        Ok(self.migrate_template(uid, false)? == TemplateMigration::Migrated)
    }

    /// Migrates one template to the AAD-bound envelope, or only reports what would happen
    /// when `dry_run` is set (GitHub #287).
    ///
    /// The file is size-bounded, authenticated and fully validated (CBOR template and UID
    /// check) in both modes, so a dry run reports exactly the files a real run would refuse.
    /// A legacy template is rewritten through [`BiometricStore::enroll`] (temporary file
    /// created `0600` with `O_NOFOLLOW`, `fsync`, atomic rename, directory `fsync`,
    /// best-effort overwrite of the legacy inode); a bound template is never rewritten. A
    /// refused file is left untouched.
    ///
    /// Concurrency (GitHub #289): a real run holds the exclusive store lock from the read to
    /// the rewrite, so a concurrent `enroll`, `delete` or `import` of the same UID either
    /// completes first (and is read) or waits (and wins afterwards); a deleted template is
    /// never resurrected and a newer one never overwritten. As a second layer, the device,
    /// inode, size and modification time of the file read are recorded and the rename is
    /// refused with [`BiometricStoreError::ChangedConcurrently`] when the path no longer holds
    /// that file (a writer that bypasses the lock); the file is then left as found. A dry run
    /// only reads and never takes the lock.
    pub fn migrate_template(
        &self,
        uid: u32,
        dry_run: bool,
    ) -> Result<TemplateMigration, BiometricStoreError> {
        self.migrate_template_with_hook(uid, dry_run, &mut || {})
    }

    /// [`Self::migrate_template`] with a hook run between the read and the rewrite.
    fn migrate_template_with_hook(
        &self,
        uid: u32,
        dry_run: bool,
        before_rewrite: &mut dyn FnMut(),
    ) -> Result<TemplateMigration, BiometricStoreError> {
        let _lock = if dry_run {
            None
        } else {
            Some(self.lock_store()?)
        };
        let Some((decrypted_bytes, format, identity)) = self.read_decrypted_with_identity(uid)?
        else {
            return Ok(TemplateMigration::Missing);
        };
        if format == PayloadFormat::BoundV2 {
            let metadata = TemplateMetadata::from_cbor(&decrypted_bytes)?;
            check_uid(uid, metadata.uid)?;
            return Ok(TemplateMigration::AlreadyCurrent);
        }
        let template = BiometricTemplate::from_cbor(&decrypted_bytes)?;
        drop(decrypted_bytes);
        check_uid(uid, template.uid)?;
        if dry_run {
            return Ok(TemplateMigration::WouldMigrate);
        }
        before_rewrite();
        self.write_template(&template, Some(&identity))?;
        Ok(TemplateMigration::Migrated)
    }

    /// Migrates every legacy unbound template of the store to the AAD-bound envelope, or
    /// only reports what would happen when `dry_run` is set (GitHub #287, owner decision
    /// 2026-10-01: an operator-run migration; legacy templates stay readable without it).
    ///
    /// Each UID of [`BiometricStore::list_enrolled`] goes through
    /// [`BiometricStore::migrate_template`]. A per-file error (unreadable, tampered, foreign
    /// UID, symlink, I/O failure) is recorded in [`TemplateMigrationReport::failed`] and the
    /// remaining templates are still processed; a refused file is never rewritten. Running the
    /// migration again reports every template as already current. Only listing the store
    /// directory can fail the whole call. The report carries UIDs and error messages only,
    /// never embedding values or key material.
    pub fn migrate_legacy_templates(
        &self,
        dry_run: bool,
    ) -> Result<TemplateMigrationReport, BiometricStoreError> {
        let mut uids = self.list_enrolled()?;
        uids.dedup();
        let mut report = TemplateMigrationReport {
            dry_run,
            ..TemplateMigrationReport::default()
        };
        for uid in uids {
            match self.migrate_template(uid, dry_run) {
                Ok(TemplateMigration::Migrated | TemplateMigration::WouldMigrate) => {
                    report.migrated.push(uid);
                }
                Ok(TemplateMigration::AlreadyCurrent) => report.already_current.push(uid),
                // Listed under a non-canonical name (for example a leading zero) or removed
                // concurrently: no template is read at the canonical path of this UID.
                Ok(TemplateMigration::Missing) => {}
                Err(e) => report.failed.push(TemplateMigrationFailure {
                    uid,
                    error: e.to_string(),
                }),
            }
        }
        Ok(report)
    }

    /// Reads (at most [`MAX_TEMPLATE_FILE_BYTES`]) and decrypts a user's template file,
    /// reporting its envelope format. A bound template must be bound to `uid`.
    fn read_decrypted(&self, uid: u32) -> Result<Option<DecryptedTemplate>, BiometricStoreError> {
        Ok(self
            .read_decrypted_with_identity(uid)?
            .map(|(bytes, format, _identity)| (bytes, format)))
    }

    /// [`Self::read_decrypted`], also reporting the identity of the file that was read.
    fn read_decrypted_with_identity(
        &self,
        uid: u32,
    ) -> Result<Option<IdentifiedTemplate>, BiometricStoreError> {
        let path = self.template_path(uid)?;
        if !path.exists() {
            return Ok(None);
        }

        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)?;
        let opened = file.metadata()?;
        let identity = FileIdentity::of(&opened);
        let len = opened.len();
        if len > MAX_TEMPLATE_FILE_BYTES {
            return Err(BiometricStoreError::CorruptFile(format!(
                "template file of {len} bytes exceeds {MAX_TEMPLATE_FILE_BYTES} bytes"
            )));
        }
        // The file may grow after the length check: read at most one byte past the bound.
        let capacity = usize::try_from(len).unwrap_or(0);
        let mut encrypted_bytes = Vec::with_capacity(capacity);
        file.take(MAX_TEMPLATE_FILE_BYTES.saturating_add(1))
            .read_to_end(&mut encrypted_bytes)?;
        if u64::try_from(encrypted_bytes.len()).unwrap_or(u64::MAX) > MAX_TEMPLATE_FILE_BYTES {
            return Err(BiometricStoreError::CorruptFile(format!(
                "template file exceeds {MAX_TEMPLATE_FILE_BYTES} bytes"
            )));
        }

        let (plaintext, format) = decrypt_template_payload(&self.key, uid, &encrypted_bytes)?;
        Ok(Some((plaintext, format, identity)))
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
    /// not exist. Runs under the exclusive store lock like [`BiometricStore::enroll`]
    /// (GitHub #289); [`BiometricStoreError::LockTimeout`] leaves the template in place.
    pub fn delete(&self, uid: u32) -> Result<bool, BiometricStoreError> {
        let _lock = self.lock_store()?;
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

impl BiometricStore {
    /// Takes the exclusive advisory store lock (GitHub #289): a `flock` on the store
    /// directory itself, opened `O_RDONLY | O_DIRECTORY | O_NOFOLLOW`, so no lock file is
    /// ever created. Released when the returned guard is dropped (also on error paths).
    ///
    /// A contended lock is retried every [`STORE_LOCK_POLL_INTERVAL`] until the lock timeout;
    /// past it, [`BiometricStoreError::LockTimeout`] is returned.
    fn lock_store(&self) -> Result<Flock<File>, BiometricStoreError> {
        let mut dir = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(&self.base_dir)?;
        let started = Instant::now();
        loop {
            match Flock::lock(dir, FlockArg::LockExclusiveNonblock) {
                Ok(lock) => return Ok(lock),
                Err((returned, Errno::EAGAIN | Errno::EINTR)) => {
                    let waited = started.elapsed();
                    if waited >= self.lock_timeout {
                        return Err(BiometricStoreError::LockTimeout(format!(
                            "biometric store '{}' is locked by another enroll, delete, import or \
                             migrate operation; gave up after {} ms",
                            self.base_dir.display(),
                            u64::try_from(waited.as_millis()).unwrap_or(u64::MAX)
                        )));
                    }
                    std::thread::sleep(
                        STORE_LOCK_POLL_INTERVAL.min(self.lock_timeout.saturating_sub(waited)),
                    );
                    dir = returned;
                }
                Err((_, errno)) => {
                    return Err(BiometricStoreError::Io(std::io::Error::from(errno)))
                }
            }
        }
    }
}

/// Identity of a template file at read time: device, inode, size and modification time
/// (GitHub #289). A writer that replaces the file (rename) changes the device or inode; one
/// that rewrites it in place changes the size or the modification time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    dev: u64,
    ino: u64,
    len: u64,
    mtime: i64,
    mtime_nsec: i64,
}

impl FileIdentity {
    fn of(meta: &Metadata) -> Self {
        Self {
            dev: meta.dev(),
            ino: meta.ino(),
            len: meta.len(),
            mtime: meta.mtime(),
            mtime_nsec: meta.mtime_nsec(),
        }
    }

    /// Whether `meta` describes the same, unmodified regular file.
    fn matches(&self, meta: &Metadata) -> bool {
        meta.file_type().is_file() && Self::of(meta) == *self
    }

    /// Whether `path` (not followed if it is a symlink) still holds this file.
    fn still_at(&self, path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_ok_and(|meta| self.matches(&meta))
    }
}

/// Whether `name` is exactly a temporary file name of [`BiometricStore::enroll`]:
/// `<uid>.tmp.<pid>.<suffix>` with canonical decimal `u32`, `u32` and `u64`.
fn is_template_temp_name(name: &str) -> bool {
    let mut parts = name.split('.');
    match (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) {
        (Some(uid), Some("tmp"), Some(pid), Some(suffix), None) => {
            canonical_decimal::<u32>(uid)
                && canonical_decimal::<u32>(pid)
                && canonical_decimal::<u64>(suffix)
        }
        _ => false,
    }
}

/// Whether `text` is the canonical decimal form of a `T` (no sign, no leading zero).
fn canonical_decimal<T: std::str::FromStr + ToString>(text: &str) -> bool {
    text.parse::<T>()
        .is_ok_and(|value| value.to_string() == text)
}

/// What the sweep did with one candidate name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SweepVerdict {
    /// The orphaned file was unlinked.
    Removed,
    /// A matching regular file younger than [`TEMP_SWEEP_MIN_AGE`]: possibly a write in
    /// progress, kept.
    Recent,
    /// Not a removable file (gone, not regular, linked, foreign owner): kept.
    Kept,
}

/// Examines `name` inside the directory `dir_fd` without following it and unlinks it when it
/// is an orphaned temporary file (regular, one link, owned by root or `euid`, old enough).
fn sweep_candidate(
    dir_fd: std::os::fd::RawFd,
    name: &std::ffi::OsStr,
    euid: u32,
    now: std::time::SystemTime,
) -> Result<SweepVerdict, BiometricStoreError> {
    let Ok(stat) =
        nix::sys::stat::fstatat(Some(dir_fd), name, nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW)
    else {
        return Ok(SweepVerdict::Kept);
    };
    let regular = stat.st_mode & libc::S_IFMT == libc::S_IFREG;
    if !regular || stat.st_nlink != 1 || (stat.st_uid != 0 && stat.st_uid != euid) {
        return Ok(SweepVerdict::Kept);
    }
    if !old_enough(stat.st_mtime, now) {
        return Ok(SweepVerdict::Recent);
    }
    match nix::unistd::unlinkat(Some(dir_fd), name, nix::unistd::UnlinkatFlags::NoRemoveDir) {
        Ok(()) => Ok(SweepVerdict::Removed),
        Err(Errno::ENOENT) => Ok(SweepVerdict::Kept),
        Err(errno) => Err(BiometricStoreError::Io(std::io::Error::from(errno))),
    }
}

/// Whether a modification time (seconds since the epoch) is at least [`TEMP_SWEEP_MIN_AGE`]
/// before `now`. A time in the future is never old; one before the epoch always is.
fn old_enough(mtime_secs: libc::time_t, now: std::time::SystemTime) -> bool {
    let modified = match u64::try_from(mtime_secs) {
        Ok(secs) => std::time::UNIX_EPOCH.checked_add(Duration::from_secs(secs)),
        Err(_) => Some(std::time::UNIX_EPOCH),
    };
    modified
        .and_then(|modified| now.duration_since(modified).ok())
        .is_some_and(|age| age >= TEMP_SWEEP_MIN_AGE)
}

/// The per-file migration error for a template that changed after it was read.
fn changed_concurrently(uid: u32) -> BiometricStoreError {
    BiometricStoreError::ChangedConcurrently(format!(
        "template of UID {uid} changed concurrently during migration; left as found"
    ))
}

/// Refuses a template whose embedded UID differs from the UID of its file name.
fn check_uid(expected: u32, found: u32) -> Result<(), BiometricStoreError> {
    if found != expected {
        return Err(BiometricStoreError::CorruptFile(format!(
            "Mismatched UID in template: expected {expected}, found {found}"
        )));
    }
    Ok(())
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

/// GitHub #289: a migration racing a `delete` or a re-enrollment of the same UID (matrix
/// MLS1, MLS2). The private hook runs between the migration read and its rewrite, which is
/// exactly the window of the race.
#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Unit tests utilize direct assertions"
)]
mod concurrency_tests {
    use super::{BiometricStore, TemplateMigration};
    use crate::crypto::{encrypt_payload, MasterKey, PayloadFormat};
    use crate::error::BiometricStoreError;
    use crate::template::BiometricTemplate;
    use std::path::Path;
    use std::thread::JoinHandle;
    use std::time::Duration;
    use tempfile::TempDir;
    use zeroize::Zeroizing;

    /// Long enough for the racing thread to reach the store in the window.
    const RACE_WINDOW: Duration = Duration::from_millis(300);

    const UID: u32 = 1000;

    fn template(value: f32, timestamp: u64) -> BiometricTemplate {
        BiometricTemplate::new(
            UID,
            "arcface_w600k_mbf".to_string(),
            "2.0.0".to_string(),
            timestamp,
            Zeroizing::new(vec![value; 512]),
        )
        .unwrap()
    }

    fn store_at(dir: &Path, key: &MasterKey) -> BiometricStore {
        BiometricStore::new(dir, key.clone()).unwrap()
    }

    /// A store holding one legacy (v1) template of `UID` with value 0.25.
    fn legacy_store(temp: &TempDir) -> (BiometricStore, MasterKey) {
        let key = MasterKey::generate().unwrap();
        let store = store_at(&temp.path().join("bio"), &key);
        let cbor = template(0.25, 1).to_cbor().unwrap();
        std::fs::write(
            store.template_path(UID).unwrap(),
            encrypt_payload(&key, &cbor).unwrap(),
        )
        .unwrap();
        (store, key)
    }

    fn dir_entries(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn test_mls_concurrent_delete_during_migrate_does_not_resurrect() {
        let temp = TempDir::new().unwrap();
        let (store, key) = legacy_store(&temp);
        let dir = temp.path().join("bio");
        let mut racer: Option<JoinHandle<Result<bool, BiometricStoreError>>> = None;

        let outcome = store
            .migrate_template_with_hook(UID, false, &mut || {
                let other = store_at(&dir, &key);
                racer = Some(std::thread::spawn(move || other.delete(UID)));
                std::thread::sleep(RACE_WINDOW);
            })
            .unwrap();
        let deleted = racer.unwrap().join().unwrap().unwrap();

        assert_eq!(outcome, TemplateMigration::Migrated);
        assert!(
            deleted,
            "the delete runs once the migration released the lock"
        );
        assert!(
            !store.exists(UID).unwrap(),
            "a deleted template must never be resurrected by a migration"
        );
        assert!(dir_entries(&dir).is_empty());
    }

    #[test]
    fn test_mls_concurrent_enroll_during_migrate_is_not_overwritten() {
        let temp = TempDir::new().unwrap();
        let (store, key) = legacy_store(&temp);
        let dir = temp.path().join("bio");
        let mut racer: Option<JoinHandle<Result<(), BiometricStoreError>>> = None;

        let outcome = store
            .migrate_template_with_hook(UID, false, &mut || {
                let other = store_at(&dir, &key);
                racer = Some(std::thread::spawn(move || other.enroll(&template(0.75, 2))));
                std::thread::sleep(RACE_WINDOW);
            })
            .unwrap();
        racer.unwrap().join().unwrap().unwrap();

        assert_eq!(outcome, TemplateMigration::Migrated);
        let kept = store.get(UID).unwrap().unwrap();
        assert_eq!(
            kept.enrollment_timestamp, 2,
            "the newer enrollment must never be overwritten by the migrated legacy template"
        );
        assert!(kept
            .embedding
            .iter()
            .all(|v| (*v - 0.75).abs() < f32::EPSILON));
        assert_eq!(
            store.template_format(UID).unwrap(),
            Some(PayloadFormat::BoundV2)
        );
        assert_eq!(dir_entries(&dir), vec!["1000.cbor.enc".to_string()]);
    }

    #[test]
    fn test_mls_unlocked_removal_during_migrate_is_refused_not_resurrected() {
        let temp = TempDir::new().unwrap();
        let (store, _key) = legacy_store(&temp);
        let dir = temp.path().join("bio");
        let path = store.template_path(UID).unwrap();

        // A writer that bypasses the store lock removes the file inside the window.
        let result = store.migrate_template_with_hook(UID, false, &mut || {
            std::fs::remove_file(&path).unwrap();
        });

        match result {
            Err(BiometricStoreError::ChangedConcurrently(msg)) => {
                assert!(msg.contains("changed concurrently"), "{msg}");
                assert!(msg.contains(&UID.to_string()), "{msg}");
            }
            other => panic!("expected ChangedConcurrently, got {other:?}"),
        }
        assert!(!path.exists(), "the removed template is not resurrected");
        assert!(dir_entries(&dir).is_empty(), "no temporary file left");
    }

    #[test]
    fn test_mls_unlocked_replace_during_migrate_is_refused_and_newer_file_kept() {
        let temp = TempDir::new().unwrap();
        let (store, key) = legacy_store(&temp);
        let dir = temp.path().join("bio");
        let path = store.template_path(UID).unwrap();

        // The newer template is produced by a second store with the same key.
        let staging = store_at(&temp.path().join("staging"), &key);
        staging.enroll(&template(0.75, 2)).unwrap();
        let newer = std::fs::read(staging.template_path(UID).unwrap()).unwrap();

        // A writer that bypasses the store lock replaces the file inside the window.
        let result = store.migrate_template_with_hook(UID, false, &mut || {
            let tmp = dir.join("racer.tmp");
            std::fs::write(&tmp, &newer).unwrap();
            std::fs::rename(&tmp, &path).unwrap();
        });

        assert!(
            matches!(result, Err(BiometricStoreError::ChangedConcurrently(_))),
            "got {result:?}"
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            newer,
            "the newer template is kept byte for byte"
        );
        assert_eq!(dir_entries(&dir), vec!["1000.cbor.enc".to_string()]);
    }

    #[test]
    fn test_mls_unlocked_in_place_rewrite_during_migrate_is_refused() {
        let temp = TempDir::new().unwrap();
        let (store, key) = legacy_store(&temp);
        let path = store.template_path(UID).unwrap();
        let mut rewritten = encrypt_payload(&key, &template(0.5, 3).to_cbor().unwrap()).unwrap();
        // One more byte than the legacy file: the size differs even on a coarse clock.
        rewritten.push(0);

        // Same inode, new content: refused on the size and modification time.
        let result = store.migrate_template_with_hook(UID, false, &mut || {
            std::fs::write(&path, &rewritten).unwrap();
        });

        assert!(
            matches!(result, Err(BiometricStoreError::ChangedConcurrently(_))),
            "got {result:?}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), rewritten);
    }
}
