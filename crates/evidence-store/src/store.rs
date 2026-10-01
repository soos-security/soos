//! Main storage engine for encrypted evidence snapshots.

use crate::config::{EvidenceConfig, DEFAULT_DAILY_CAP_TOTAL};
use crate::crypto::{decrypt_snapshot_payload, encrypt_snapshot_payload, MasterKey, PayloadFormat};
use crate::error::EvidenceStoreError;
use crate::frame::{
    check_payload_len, EvidenceFrame, FrameBytes, FrameMetadata, EVIDENCE_RECORD_VERSION,
    MAX_EVIDENCE_FILE_BYTES,
};
use crate::snapshot::{
    days_since_epoch, format_date_from_timestamp, generate_uuid_v4, parse_date, EvidenceRecord,
    RetentionReport, SnapshotMigrationFailure, SnapshotMigrationReport, SnapshotResult,
};
use nix::fcntl::{Flock, FlockArg};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Maximum valid POSIX UID accepted by the evidence store (2^31 - 1).
///
/// Values above this range typically represent negative signed integers cast to unsigned
/// or reserved POSIX sentinel values like `(uid_t)-1` (4,294,967,295).
pub const MAX_VALID_UID: u32 = 2_147_483_647;

/// File suffix of self-describing frame snapshots written by
/// [`EvidenceStore::store_frame_snapshot`] (record version 2 with frame metadata).
pub const FRAME_SNAPSHOT_EXTENSION: &str = ".frame.enc";

/// File suffix of opaque snapshots written by [`EvidenceStore::store_snapshot`].
///
/// Kept for backward compatibility of the opaque-bytes API and its contract tests: the store
/// never encodes WebP, the payload is whatever the caller passed, and the record carries no
/// frame metadata. New code must use [`EvidenceStore::store_frame_snapshot`].
pub const OPAQUE_SNAPSHOT_EXTENSION: &str = ".opaque.enc";

/// File name prefix of the persisted per-UID daily capture counter inside a date partition
/// (`YYYY-MM-DD/.daily_count.<uid>`, GitHub #234). It never ends in `.enc`, so it is never
/// listed as a snapshot, and it is removed with its partition by retention.
pub const DAILY_COUNT_FILE_PREFIX: &str = ".daily_count.";

/// Minimum age of an orphaned temporary file before the startup sweep removes it (GitHub #291).
pub const TEMP_SWEEP_MIN_AGE: std::time::Duration = std::time::Duration::from_secs(60);

/// Maximum number of orphaned temporary files one sweep removes (GitHub #291).
pub const MAX_TEMP_SWEEP_REMOVALS: usize = 256;

/// Maximum number of directory entries one sweep examines, partitions included (GitHub #291).
const MAX_TEMP_SWEEP_SCANNED_ENTRIES: usize = 65_536;

/// Outcome of [`EvidenceStore::sweep_orphaned_temp_files`] (GitHub #291).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TempSweepReport {
    /// Orphaned temporary files removed.
    pub removed: usize,
    /// Matching temporary files kept because they are younger than [`TEMP_SWEEP_MIN_AGE`].
    pub kept_recent: usize,
    /// The removal or scan bound was reached; a later sweep continues.
    pub limit_reached: bool,
    /// Retention or migration held the base-directory lock: nothing was examined or removed.
    pub lock_busy: bool,
}

/// Upper bound of a persisted daily counter file (a decimal `u32` plus an optional newline).
const MAX_DAILY_COUNT_FILE_BYTES: u64 = 16;

/// In-memory view of the day the store currently writes to (GitHub #276, #234).
///
/// The persisted per-UID counters (`YYYY-MM-DD/.daily_count.<uid>`) stay the source of truth
/// for the per-UID cap. This view only tracks one day: the global total of that day (derived
/// from the snapshot files of its partition when the store switches to it, then kept up to
/// date after every successful write) and the per-UID counts written by this process that
/// day. Switching to another date drops every entry of the previous one, so the map never
/// holds more entries than the global cap.
#[derive(Debug, Default)]
struct DailyCounters {
    /// Tracked date (`YYYY-MM-DD`), `None` before the first write of this process.
    date: Option<String>,
    /// Per-UID counts of the tracked date written by this process.
    per_uid: HashMap<u32, u32>,
    /// Snapshots stored on the tracked date across all UIDs (persisted files included).
    total: u32,
}

impl DailyCounters {
    fn tracks(&self, date: &str) -> bool {
        self.date.as_deref() == Some(date)
    }

    /// `true` when another date is tracked, so `date` must report `0`.
    fn tracks_other_than(&self, date: &str) -> bool {
        self.date.as_deref().is_some_and(|d| d != date)
    }

    /// Switches the view to `date` with the persisted `total`, dropping the previous day.
    fn roll_to(&mut self, date: &str, total: u32) {
        self.date = Some(date.to_string());
        self.per_uid.clear();
        self.per_uid.shrink_to_fit();
        self.total = total;
    }

    fn reset(&mut self) {
        self.date = None;
        self.per_uid.clear();
        self.per_uid.shrink_to_fit();
        self.total = 0;
    }
}

/// Primary evidence store engine.
pub struct EvidenceStore {
    config: EvidenceConfig,
    key: MasterKey,
    /// Global daily snapshot cap across all UIDs (GitHub #276).
    daily_cap_total: u32,
    /// Serializes the "check caps, write snapshot, persist counter" critical section so that
    /// concurrent writers of this process can never exceed either cap (GitHub #234, #276).
    /// The per-UID counts live on disk; the in-memory view is bounded by the global cap.
    daily_counts: Mutex<DailyCounters>,
}

impl EvidenceStore {
    /// Constructs a new `EvidenceStore` with provided config and key.
    pub fn new(config: EvidenceConfig, key: MasterKey) -> Self {
        Self {
            config,
            key,
            daily_cap_total: DEFAULT_DAILY_CAP_TOTAL,
            daily_counts: Mutex::new(DailyCounters::default()),
        }
    }

    /// Overrides the global daily snapshot cap across all UIDs
    /// (default [`DEFAULT_DAILY_CAP_TOTAL`]).
    #[must_use]
    pub fn with_daily_cap_total(mut self, cap: u32) -> Self {
        self.daily_cap_total = cap;
        self
    }

    /// Returns the global daily snapshot cap across all UIDs.
    pub fn daily_cap_total(&self) -> u32 {
        self.daily_cap_total
    }

    /// Initializes an evidence store by loading or creating the master key.
    pub fn open(config: EvidenceConfig) -> Result<Self, EvidenceStoreError> {
        if !config.enabled {
            let dummy_key = MasterKey::generate()?;
            return Ok(Self::new(config, dummy_key));
        }
        let key = MasterKey::load_or_create(&config.key_path)?;
        Ok(Self::new(config, key))
    }

    /// Removes the temporary files an interrupted write left in the date partitions
    /// (GitHub #291): a blocking evidence write abandoned at daemon shutdown is never awaited,
    /// so its `.tmp.*` file can survive the process. `soos-daemon` calls it once at startup.
    ///
    /// Only real `YYYY-MM-DD` partition directories of the base directory are examined
    /// (symlinked or misnamed entries are skipped, never followed), and inside them only the
    /// exact names this store creates: `.tmp.<uuid>.<pid>.<16 hex>` (snapshot),
    /// `.tmp.migrate.<uuid>.<pid>.<16 hex>` (migration) and
    /// `.tmp.daily_count.<uid>.<pid>.<16 hex>` (daily counter), with a lowercase hyphenated
    /// UUID, canonical decimal numbers and lowercase hex. A candidate is removed only if it is
    /// a regular file with a single link, owned by root or the effective UID and last
    /// modified at least [`TEMP_SWEEP_MIN_AGE`] ago (a future time is never old). The
    /// partition is opened `O_DIRECTORY | O_NOFOLLOW` relative to the locked base-directory
    /// descriptor, and its listing, the checks and the unlink all go through that one
    /// descriptor: no path is resolved again after the base directory's identity check
    /// (GitHub #293). At most [`MAX_TEMP_SWEEP_REMOVALS`] files are removed and a
    /// bounded number of entries examined per call ([`TempSweepReport::limit_reached`]).
    ///
    /// Writers of this process are excluded by the daily-counter mutex; retention and
    /// migration by the base-directory `flock`, which is tried once and never waited for
    /// ([`TempSweepReport::lock_busy`]). A disabled store or a missing base directory is not
    /// touched (nothing is created).
    ///
    /// # Errors
    ///
    /// [`EvidenceStoreError::InvalidPath`] for a symlinked or non-directory base directory;
    /// [`EvidenceStoreError::Io`] when it cannot be opened, listed or unlinked from.
    pub fn sweep_orphaned_temp_files(&self) -> Result<TempSweepReport, EvidenceStoreError> {
        self.sweep_orphaned_temp_files_with(&mut |_| {})
    }

    /// [`Self::sweep_orphaned_temp_files`] with a test seam: `after_partition_open` runs with
    /// the partition name right after its descriptor is opened, before it is listed
    /// (GitHub #293).
    fn sweep_orphaned_temp_files_with(
        &self,
        after_partition_open: &mut dyn FnMut(&str),
    ) -> Result<TempSweepReport, EvidenceStoreError> {
        let mut report = TempSweepReport::default();
        if !self.config.enabled {
            return Ok(report);
        }
        let base = &self.config.base_dir;
        match fs::symlink_metadata(base) {
            Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
                return Err(EvidenceStoreError::InvalidPath(format!(
                    "Evidence base directory '{}' is a symlink or not a directory",
                    base.display()
                )));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(report),
            Err(e) => return Err(EvidenceStoreError::Io(e)),
        }

        let _writers = self.daily_counts.lock().unwrap_or_else(|e| e.into_inner());
        let dir = open_dir_no_follow(base)?;
        let lock = match Flock::lock(dir, FlockArg::LockExclusiveNonblock) {
            Ok(lock) => lock,
            Err((_, nix::errno::Errno::EAGAIN | nix::errno::Errno::EINTR)) => {
                report.lock_busy = true;
                return Ok(report);
            }
            Err((_, errno)) => return Err(EvidenceStoreError::Io(std::io::Error::from(errno))),
        };
        let opened = lock.metadata()?;
        let listed = fs::symlink_metadata(base)?;
        if listed.dev() != opened.dev() || listed.ino() != opened.ino() {
            return Err(EvidenceStoreError::InvalidPath(format!(
                "Evidence base directory '{}' changed while it was being swept",
                base.display()
            )));
        }

        let euid = nix::unistd::geteuid().as_raw();
        let now = SystemTime::now();
        let mut scanned: usize = 0;
        let mut partitions = Vec::new();
        // The base directory is listed and its partitions are opened through the locked,
        // identity-checked descriptor: the path is never resolved again (GitHub #293).
        let base_fd = lock.as_raw_fd();
        let mut base_dir = open_dir_at_no_follow(base_fd, c".")?;
        for entry in base_dir.iter() {
            let entry = entry.map_err(errno_to_io)?;
            let name = entry.file_name();
            if is_dot_entry(name) {
                continue;
            }
            if scanned >= MAX_TEMP_SWEEP_SCANNED_ENTRIES {
                report.limit_reached = true;
                return Ok(report);
            }
            scanned = scanned.saturating_add(1);
            // The partition is opened `O_NOFOLLOW` below as well: a symlink is never followed.
            if !is_directory_entry(base_fd, &entry) {
                continue;
            }
            if name.to_str().is_ok_and(|date| parse_date(date).is_ok()) {
                partitions.push(name.to_owned());
            }
        }
        drop(base_dir);

        for date in partitions {
            let Ok(mut partition) = open_dir_at_no_follow(base_fd, &date) else {
                continue;
            };
            after_partition_open(date.to_str().unwrap_or_default());
            // Listed, examined and unlinked from through the same descriptor: a partition
            // swapped after the open is never consulted (GitHub #293).
            let partition_fd = partition.as_raw_fd();
            let mut sync_needed = false;
            for entry in partition.iter() {
                let entry = entry.map_err(errno_to_io)?;
                let name = entry.file_name();
                if is_dot_entry(name) {
                    continue;
                }
                if report.removed >= MAX_TEMP_SWEEP_REMOVALS
                    || scanned >= MAX_TEMP_SWEEP_SCANNED_ENTRIES
                {
                    report.limit_reached = true;
                    break;
                }
                scanned = scanned.saturating_add(1);
                if !name.to_str().is_ok_and(is_evidence_temp_name) {
                    continue;
                }
                match sweep_candidate(partition_fd, name, euid, now)? {
                    SweepVerdict::Removed => {
                        report.removed = report.removed.saturating_add(1);
                        sync_needed = true;
                    }
                    SweepVerdict::Recent => {
                        report.kept_recent = report.kept_recent.saturating_add(1);
                    }
                    SweepVerdict::Kept => {}
                }
            }
            if sync_needed {
                nix::unistd::fsync(partition_fd).map_err(errno_to_io)?;
            }
            if report.limit_reached {
                break;
            }
        }
        Ok(report)
    }

    /// Returns a reference to the active configuration.
    pub fn config(&self) -> &EvidenceConfig {
        &self.config
    }

    /// Stores an opaque payload (no frame metadata) with AES-256-GCM encryption.
    ///
    /// The snapshot is written atomically to
    /// `/var/lib/soos/evidence/YYYY-MM-DD/<uuid>`[`OPAQUE_SNAPSHOT_EXTENSION`] with mode
    /// `0600` and its parent directory with mode `0700`. The payload is bounded by
    /// [`crate::MAX_EVIDENCE_IMAGE_BYTES`] and cannot be rendered without out-of-band
    /// knowledge; camera frames must go through [`Self::store_frame_snapshot`].
    pub fn store_snapshot(
        &self,
        uid: u32,
        reason: &str,
        image_data: &[u8],
        date_override: Option<&str>,
        timestamp_override: Option<u64>,
    ) -> Result<SnapshotResult, EvidenceStoreError> {
        self.store_record(
            uid,
            reason,
            None,
            image_data,
            date_override,
            timestamp_override,
        )
    }

    /// Stores a self-describing camera frame snapshot (GitHub #181).
    ///
    /// Width, height, pixel format, capture time and sequence are sealed with the pixels in
    /// a version [`crate::EVIDENCE_RECORD_VERSION`] record, written atomically to
    /// `/var/lib/soos/evidence/YYYY-MM-DD/<uuid>`[`FRAME_SNAPSHOT_EXTENSION`] (mode `0600`).
    /// The frame is validated before the daily cap is consumed or anything is written.
    ///
    /// # Errors
    ///
    /// [`EvidenceStoreError::InvalidFrame`] when the metadata is out of bounds or does not
    /// match the payload length, plus every error of [`Self::store_snapshot`].
    pub fn store_frame_snapshot(
        &self,
        uid: u32,
        reason: &str,
        frame: &EvidenceFrame<'_>,
        date_override: Option<&str>,
        timestamp_override: Option<u64>,
    ) -> Result<SnapshotResult, EvidenceStoreError> {
        self.store_record(
            uid,
            reason,
            Some(&frame.metadata),
            frame.data,
            date_override,
            timestamp_override,
        )
    }

    fn store_record(
        &self,
        uid: u32,
        reason: &str,
        metadata: Option<&FrameMetadata>,
        image_data: &[u8],
        date_override: Option<&str>,
        timestamp_override: Option<u64>,
    ) -> Result<SnapshotResult, EvidenceStoreError> {
        if !self.config.enabled {
            return Err(EvidenceStoreError::Disabled);
        }

        // Validate UID parameter against POSIX bounds (Sub-issue #30.3)
        if uid > MAX_VALID_UID {
            return Err(EvidenceStoreError::InvalidUid(uid));
        }

        // Bounded, self-consistent payload before any side effect (GitHub #181).
        match metadata {
            Some(meta) => meta.validate(image_data.len())?,
            None => check_payload_len(image_data.len())?,
        }

        // Reject symlinks targeting base directory
        if let Ok(meta) = fs::symlink_metadata(&self.config.base_dir) {
            if meta.file_type().is_symlink() {
                return Err(EvidenceStoreError::InvalidPath(format!(
                    "Evidence base directory '{}' is a symlink; symlinks are forbidden",
                    self.config.base_dir.display()
                )));
            }
        }

        let ts = match timestamp_override {
            Some(t) => t,
            None => {
                let dur = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|e| EvidenceStoreError::Crypto(format!("Clock error: {e}")))?;
                dur.as_secs()
            }
        };

        let date_str = match date_override {
            Some(d) => {
                // Validate date format
                let _ = parse_date(d)?;
                d.to_string()
            }
            None => format_date_from_timestamp(ts),
        };

        // One writer at a time: the cap checks, the snapshot write and the counter updates are
        // a single critical section (GitHub #234, #276).
        let mut counts = self.daily_counts.lock().unwrap_or_else(|e| e.into_inner());

        let target_dir = self.config.base_dir.join(&date_str);

        // Pre-creation symlink check on date directory (Sub-issue #30.1)
        let dir_exists = match fs::symlink_metadata(&target_dir) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return Err(EvidenceStoreError::InvalidPath(format!(
                        "Date directory '{}' is a symlink; symlinks are forbidden",
                        target_dir.display()
                    )));
                }
                if !meta.is_dir() {
                    return Err(EvidenceStoreError::InvalidPath(format!(
                        "Date path '{}' exists but is not a directory",
                        target_dir.display()
                    )));
                }
                true
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(EvidenceStoreError::Io(e)),
        };

        // Switch the in-memory view to this date; its global total starts from the snapshot
        // files already in the partition, so it survives a restart too (GitHub #276).
        if !counts.tracks(&date_str) {
            let persisted_total = if dir_exists {
                count_snapshot_files(&target_dir)?
            } else {
                0
            };
            counts.roll_to(&date_str, persisted_total);
        }

        // Enforce the per-UID daily cap from the persisted counter (survives restarts).
        let used = if dir_exists {
            read_daily_count(&target_dir, uid)?
        } else {
            0
        };
        if used >= self.config.daily_cap_per_uid {
            return Err(EvidenceStoreError::DailyCapExceeded {
                uid,
                cap: self.config.daily_cap_per_uid,
                date: date_str,
            });
        }

        // Enforce the global daily cap across all UIDs before any write; a refusal consumes
        // neither the per-UID nor the global quota (GitHub #276).
        if counts.total >= self.daily_cap_total {
            return Err(EvidenceStoreError::GlobalDailyCapExceeded {
                cap: self.daily_cap_total,
                date: date_str,
            });
        }

        let snapshot_id = generate_uuid_v4()?;

        if !dir_exists {
            fs::create_dir_all(&target_dir)?;
            fs::set_permissions(&target_dir, fs::Permissions::from_mode(0o700))?;
            let meta = fs::symlink_metadata(&target_dir)?;
            if meta.file_type().is_symlink() {
                return Err(EvidenceStoreError::InvalidPath(format!(
                    "Date directory '{}' was created as a symlink; symlinks are forbidden",
                    target_dir.display()
                )));
            }
        }

        let record = EvidenceRecord {
            format_version: EVIDENCE_RECORD_VERSION,
            snapshot_id: snapshot_id.clone(),
            uid,
            timestamp: ts,
            reason: reason.to_string(),
            frame: metadata.cloned(),
            image_data: FrameBytes::new(image_data.to_vec()),
        };

        // Both the record (FrameBytes) and its CBOR plaintext are zeroized on drop.
        let serialized = record.to_cbor()?;
        drop(record);
        let ciphertext = encrypt_snapshot_payload(&self.key, &date_str, &snapshot_id, &serialized)?;
        drop(serialized);

        let extension = if metadata.is_some() {
            FRAME_SNAPSHOT_EXTENSION
        } else {
            OPAQUE_SNAPSHOT_EXTENSION
        };
        let filename = format!("{snapshot_id}{extension}");
        let final_path = target_dir.join(filename);
        let mut rand_bytes = [0u8; 8];
        getrandom::fill(&mut rand_bytes).map_err(|e| {
            EvidenceStoreError::Crypto(format!("Failed to generate random salt: {e}"))
        })?;
        let tmp_path = target_dir.join(format!(
            ".tmp.{snapshot_id}.{}.{:016x}",
            std::process::id(),
            u64::from_ne_bytes(rand_bytes)
        ));

        if let Err(e) = write_new_file(&tmp_path, &ciphertext) {
            let _ = fs::remove_file(&tmp_path);
            return Err(e);
        }

        // Verify final path is not a symlink before renaming
        if let Ok(meta) = fs::symlink_metadata(&final_path) {
            if meta.file_type().is_symlink() {
                let _ = fs::remove_file(&tmp_path);
                return Err(EvidenceStoreError::InvalidPath(format!(
                    "Final snapshot path '{}' is a symlink",
                    final_path.display()
                )));
            }
        }

        if let Err(e) = fs::rename(&tmp_path, &final_path) {
            let _ = fs::remove_file(&tmp_path);
            return Err(EvidenceStoreError::Io(e));
        }

        // The slot is consumed only once the snapshot is in place; if the counter cannot be
        // persisted the snapshot is rolled back so that disk usage never escapes the cap.
        let new_count = used.saturating_add(1);
        if let Err(e) = write_daily_count(&target_dir, uid, new_count) {
            let _ = fs::remove_file(&final_path);
            return Err(e);
        }
        counts.per_uid.insert(uid, new_count);
        counts.total = counts.total.saturating_add(1);
        drop(counts);

        Ok(SnapshotResult {
            snapshot_id,
            path: final_path,
            date: date_str,
            uid,
        })
    }

    /// Loads, decrypts and validates an evidence snapshot from disk.
    ///
    /// Files larger than [`MAX_EVIDENCE_FILE_BYTES`] are refused before being read. Legacy
    /// records (no version) decode as version 1 without frame metadata.
    ///
    /// The ciphertext is authenticated against the date partition (parent directory name) and
    /// the snapshot id (file name up to its first `.`) of `path` (GitHub #266): a snapshot moved
    /// to another partition or renamed to another id fails with [`EvidenceStoreError::Crypto`].
    /// Snapshots written before AAD binding (legacy unbound envelope) remain readable.
    pub fn load_snapshot<P: AsRef<Path>>(
        &self,
        path: P,
    ) -> Result<EvidenceRecord, EvidenceStoreError> {
        let path = path.as_ref();
        let data = read_bounded_evidence(File::open(path)?)?;

        let (date, snapshot_id) = snapshot_binding(path);
        let (decrypted, _format) = decrypt_snapshot_payload(&self.key, date, snapshot_id, &data)?;
        EvidenceRecord::from_cbor(&decrypted)
    }

    /// Re-encrypts every legacy unbound snapshot with the AAD-bound envelope, or only reports
    /// what would happen when `dry_run` is set (GitHub #287, owner decision 2026-10-01: an
    /// operator-run migration; legacy snapshots stay readable without it).
    ///
    /// Walks the date partitions (`YYYY-MM-DD`, real directories only) of the base directory
    /// under the same exclusive `flock` as [`Self::rotate_retention`], and every snapshot file
    /// of [`Self::list_snapshots_for_date`] (symlinks are skipped, never followed). Each file
    /// is opened with `O_NOFOLLOW`, size-bounded, authenticated against its path binding and
    /// its record decoded; a legacy record must carry the snapshot id of its file name. A
    /// legacy file is rewritten through a temporary file created exclusively with mode
    /// `0600`, synced, renamed over the original only if the path still holds the inode that
    /// was read, then the partition is synced. A bound file is never rewritten. A per-file
    /// error is recorded in [`SnapshotMigrationReport::failed`], the file is left untouched
    /// and the other files are still processed. Daily counters are not changed. A missing
    /// base directory yields an empty report (nothing is created); a symlinked base
    /// directory is refused with [`EvidenceStoreError::InvalidPath`].
    pub fn migrate_legacy_snapshots(
        &self,
        dry_run: bool,
    ) -> Result<SnapshotMigrationReport, EvidenceStoreError> {
        let base = &self.config.base_dir;
        let mut report = SnapshotMigrationReport {
            dry_run,
            ..SnapshotMigrationReport::default()
        };
        match fs::symlink_metadata(base) {
            Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
                return Err(EvidenceStoreError::InvalidPath(format!(
                    "Evidence base directory '{}' is a symlink or not a directory",
                    base.display()
                )));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(report),
            Err(e) => return Err(EvidenceStoreError::Io(e)),
        }

        let dir_file = File::open(base)?;
        let _lock = Flock::lock(dir_file, FlockArg::LockExclusive).map_err(|(_, e)| {
            EvidenceStoreError::Io(std::io::Error::from_raw_os_error(e as i32))
        })?;

        let mut dates = Vec::new();
        for entry in fs::read_dir(base)? {
            let entry = entry?;
            // `DirEntry::file_type` does not follow symlinks: a symlinked partition is skipped.
            if !entry.file_type()?.is_dir() {
                continue;
            }
            if let Some(name) = entry.file_name().to_str() {
                if parse_date(name).is_ok() {
                    dates.push(name.to_string());
                }
            }
        }
        dates.sort();

        for date in dates {
            let files = match self.list_snapshots_for_date(&date) {
                Ok(files) => files,
                Err(e) => {
                    report.failed.push(SnapshotMigrationFailure {
                        path: base.join(&date),
                        error: e.to_string(),
                    });
                    continue;
                }
            };
            for path in files {
                match self.migrate_snapshot_file(&path, dry_run) {
                    Ok(true) => report.migrated.push(path),
                    Ok(false) => report.already_current.push(path),
                    Err(e) => report.failed.push(SnapshotMigrationFailure {
                        path,
                        error: e.to_string(),
                    }),
                }
            }
        }
        Ok(report)
    }

    /// Migrates one snapshot file: `Ok(true)` when it is (or, in a dry run, would be)
    /// re-encrypted, `Ok(false)` when it is already bound. On error the file is untouched.
    fn migrate_snapshot_file(
        &self,
        path: &Path,
        dry_run: bool,
    ) -> Result<bool, EvidenceStoreError> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
            .open(path)?;
        let opened = file.metadata()?;
        if !opened.is_file() {
            return Err(EvidenceStoreError::InvalidPath(format!(
                "Snapshot path '{}' is not a regular file",
                path.display()
            )));
        }
        let data = read_bounded_evidence(file)?;

        let (date, snapshot_id) = snapshot_binding(path);
        let (plaintext, format) = decrypt_snapshot_payload(&self.key, date, snapshot_id, &data)?;
        drop(data);
        let record = EvidenceRecord::from_cbor(&plaintext)?;
        if format == PayloadFormat::BoundV2 {
            return Ok(false);
        }
        if record.snapshot_id != snapshot_id {
            return Err(EvidenceStoreError::CorruptPayload(format!(
                "legacy snapshot '{}' records id '{}'; refusing to bind it to another id",
                path.display(),
                record.snapshot_id
            )));
        }
        drop(record);
        if dry_run {
            return Ok(true);
        }

        // Keep a write handle on the legacy inode across the rename so that its ciphertext can
        // be overwritten once the bound file is committed (best effort, like templates).
        let previous = OpenOptions::new()
            .write(true)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
            .open(path)?;
        let previous_meta = previous.metadata()?;
        if !previous_meta.is_file()
            || previous_meta.ino() != opened.ino()
            || previous_meta.dev() != opened.dev()
        {
            return Err(EvidenceStoreError::InvalidPath(format!(
                "Snapshot path '{}' changed during migration",
                path.display()
            )));
        }

        // The decrypted CBOR is re-sealed byte for byte: the record content cannot change.
        let sealed = encrypt_snapshot_payload(&self.key, date, snapshot_id, &plaintext)?;
        drop(plaintext);
        if u64::try_from(sealed.len()).unwrap_or(u64::MAX) > MAX_EVIDENCE_FILE_BYTES {
            return Err(EvidenceStoreError::CorruptPayload(format!(
                "re-encrypted snapshot '{}' would exceed {MAX_EVIDENCE_FILE_BYTES} bytes",
                path.display()
            )));
        }

        let dir = path.parent().ok_or_else(|| {
            EvidenceStoreError::InvalidPath(format!(
                "Snapshot path '{}' has no parent directory",
                path.display()
            ))
        })?;
        let mut rand_bytes = [0u8; 8];
        getrandom::fill(&mut rand_bytes).map_err(|e| {
            EvidenceStoreError::Crypto(format!("Failed to generate random salt: {e}"))
        })?;
        let tmp_path = dir.join(format!(
            ".tmp.migrate.{snapshot_id}.{}.{:016x}",
            std::process::id(),
            u64::from_ne_bytes(rand_bytes)
        ));
        if let Err(e) = write_new_file(&tmp_path, &sealed) {
            let _ = fs::remove_file(&tmp_path);
            return Err(e);
        }

        // Replace only the file that was read: a path swapped meanwhile (another inode, a
        // symlink) is refused and left as it is.
        let unchanged = fs::symlink_metadata(path).is_ok_and(|now| {
            now.file_type().is_file() && now.ino() == opened.ino() && now.dev() == opened.dev()
        });
        if !unchanged {
            let _ = fs::remove_file(&tmp_path);
            return Err(EvidenceStoreError::InvalidPath(format!(
                "Snapshot path '{}' changed during migration",
                path.display()
            )));
        }
        if let Err(e) = fs::rename(&tmp_path, path) {
            let _ = fs::remove_file(&tmp_path);
            return Err(EvidenceStoreError::Io(e));
        }
        File::open(dir)?.sync_all()?;
        // The bound file is committed; an error here is reported although the snapshot is
        // already migrated (a later run reports it as already current).
        overwrite_file_contents(&previous).map_err(|e| {
            EvidenceStoreError::Io(std::io::Error::other(format!(
                "snapshot '{}' migrated, but the best-effort overwrite of its legacy ciphertext failed: {e}",
                path.display()
            )))
        })?;
        Ok(true)
    }

    /// Gets the persisted daily snapshot count for a UID and date.
    ///
    /// The count is read from the date partition, so it survives a restart and disappears
    /// with the partition when retention prunes it. Once this store has written a snapshot,
    /// only the day it currently tracks is reported and any other date reports `0` (GitHub
    /// #276); the cap enforcement of [`Self::store_snapshot`] always reads the persisted
    /// counter of the target date. An invalid date, a missing partition or an unreadable
    /// counter reports `0`; [`Self::store_snapshot`] fails closed instead.
    pub fn daily_count(&self, uid: u32, date: &str) -> u32 {
        if parse_date(date).is_err() {
            return 0;
        }
        let counts = self.daily_counts.lock().unwrap_or_else(|e| e.into_inner());
        if counts.tracks_other_than(date) {
            return 0;
        }
        match self.existing_date_dir(date) {
            Some(date_dir) => read_daily_count(&date_dir, uid).unwrap_or(0),
            None => 0,
        }
    }

    /// Gets the snapshot count of `date` across all UIDs.
    ///
    /// Reports the tracked day from memory; before the first write of this store it counts
    /// the snapshot files of the partition. Any other date, an invalid date or an unreadable
    /// partition reports `0`.
    pub fn daily_total(&self, date: &str) -> u32 {
        if parse_date(date).is_err() {
            return 0;
        }
        let counts = self.daily_counts.lock().unwrap_or_else(|e| e.into_inner());
        if counts.tracks(date) {
            return counts.total;
        }
        if counts.tracks_other_than(date) {
            return 0;
        }
        match self.existing_date_dir(date) {
            Some(date_dir) => count_snapshot_files(&date_dir).unwrap_or(0),
            None => 0,
        }
    }

    /// Number of per-UID daily counters currently held in memory (bounded by the global cap).
    pub fn tracked_daily_counters(&self) -> usize {
        let counts = self.daily_counts.lock().unwrap_or_else(|e| e.into_inner());
        counts.per_uid.len()
    }

    /// Returns the partition directory of `date` when it exists as a real directory.
    fn existing_date_dir(&self, date: &str) -> Option<PathBuf> {
        let date_dir = self.config.base_dir.join(date);
        match fs::symlink_metadata(&date_dir) {
            Ok(meta) if meta.is_dir() => Some(date_dir),
            _ => None,
        }
    }

    /// Lists snapshot file paths for a specific date partition.
    pub fn list_snapshots_for_date(&self, date: &str) -> Result<Vec<PathBuf>, EvidenceStoreError> {
        let date_dir = self.config.base_dir.join(date);
        if let Ok(meta) = fs::symlink_metadata(&date_dir) {
            if meta.file_type().is_symlink() {
                return Err(EvidenceStoreError::InvalidPath(format!(
                    "Date directory '{}' is a symlink; symlinks are forbidden",
                    date_dir.display()
                )));
            }
        }
        if !date_dir.exists() {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();
        for entry in fs::read_dir(&date_dir)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_file() && path.extension().and_then(|e| e.to_str()) == Some("enc") {
                results.push(path);
            }
        }
        results.sort();
        Ok(results)
    }

    /// Executes retention rotation: deletes date directories strictly older than `retention_days`.
    ///
    /// Synchronizes concurrent executions using an exclusive file lock (`flock`) on the evidence base directory.
    pub fn rotate_retention(
        &self,
        current_date: &str,
    ) -> Result<RetentionReport, EvidenceStoreError> {
        let (cur_y, cur_m, cur_d) = parse_date(current_date)?;
        let current_days = days_since_epoch(cur_y, cur_m, cur_d);

        if let Ok(meta) = fs::symlink_metadata(&self.config.base_dir) {
            if meta.file_type().is_symlink() {
                return Err(EvidenceStoreError::InvalidPath(format!(
                    "Evidence base directory '{}' is a symlink; symlinks are forbidden",
                    self.config.base_dir.display()
                )));
            }
        }

        if !self.config.base_dir.exists() {
            return Ok(RetentionReport::default());
        }

        // Acquire exclusive RAII file lock on the evidence root directory (Sub-issue #30.2)
        let dir_file = File::open(&self.config.base_dir)?;
        let _lock = Flock::lock(dir_file, FlockArg::LockExclusive).map_err(|(_, e)| {
            EvidenceStoreError::Io(std::io::Error::from_raw_os_error(e as i32))
        })?;

        let mut pruned_dates = Vec::new();

        for entry in fs::read_dir(&self.config.base_dir)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            // Never follow symlinks in evidence root
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                if let Some(folder_name) = path.file_name().and_then(|s| s.to_str()) {
                    if let Ok((y, m, d)) = parse_date(folder_name) {
                        let dir_days = days_since_epoch(y, m, d);
                        let age_days = current_days.saturating_sub(dir_days);
                        if age_days > i64::from(self.config.retention_days) {
                            match fs::remove_dir_all(&path) {
                                Ok(()) => pruned_dates.push(folder_name.to_string()),
                                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                                    // Directory already pruned concurrently by another thread/process
                                }
                                Err(e) => return Err(EvidenceStoreError::Io(e)),
                            }
                        }
                    }
                }
            }
        }

        pruned_dates.sort();
        let directories_pruned = pruned_dates.len();

        // A pruned tracked day must not keep counters in memory.
        {
            let mut counts = self.daily_counts.lock().unwrap_or_else(|e| e.into_inner());
            if counts
                .date
                .as_ref()
                .is_some_and(|d| pruned_dates.iter().any(|p| p == d))
            {
                counts.reset();
            }
        }

        Ok(RetentionReport {
            directories_pruned,
            pruned_dates,
        })
    }
}

/// Opens a directory for descriptor-relative work, refusing a symlink as its last component.
fn open_dir_no_follow(path: &Path) -> Result<File, EvidenceStoreError> {
    Ok(OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_DIRECTORY | nix::libc::O_NOFOLLOW)
        .open(path)?)
}

/// Opens `name` relative to `dir_fd` as a directory stream, refusing a symlink as its last
/// component (`O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC`).
fn open_dir_at_no_follow(
    dir_fd: std::os::fd::RawFd,
    name: &std::ffi::CStr,
) -> Result<nix::dir::Dir, EvidenceStoreError> {
    use nix::fcntl::OFlag;
    nix::dir::Dir::openat(
        Some(dir_fd),
        name,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        nix::sys::stat::Mode::empty(),
    )
    .map_err(errno_to_io)
}

/// Converts a `nix` error into the store's I/O error.
fn errno_to_io(errno: nix::errno::Errno) -> EvidenceStoreError {
    EvidenceStoreError::Io(std::io::Error::from(errno))
}

/// Whether a directory stream entry is `.` or `..` (never listed by `fs::read_dir`).
fn is_dot_entry(name: &std::ffi::CStr) -> bool {
    matches!(name.to_bytes(), b"." | b"..")
}

/// Whether a directory stream entry of `dir_fd` is a directory, without following a symlink
/// (`fstatat` with `AT_SYMLINK_NOFOLLOW` when the stream does not report the type).
fn is_directory_entry(dir_fd: std::os::fd::RawFd, entry: &nix::dir::Entry) -> bool {
    match entry.file_type() {
        Some(kind) => kind == nix::dir::Type::Directory,
        None => nix::sys::stat::fstatat(
            Some(dir_fd),
            entry.file_name(),
            nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW,
        )
        .is_ok_and(|stat| stat.st_mode & nix::libc::S_IFMT == nix::libc::S_IFDIR),
    }
}

/// Whether `name` is exactly a temporary file name this store creates inside a date
/// partition (GitHub #291): `.tmp.<uuid>.<pid>.<16 hex>`, `.tmp.migrate.<uuid>.<pid>.<16 hex>`
/// or `.tmp.daily_count.<uid>.<pid>.<16 hex>`.
fn is_evidence_temp_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix(".tmp.") else {
        return false;
    };
    let counter_prefix = DAILY_COUNT_FILE_PREFIX.trim_start_matches('.');
    let (id_ok, tail) = if let Some(tail) = rest.strip_prefix(counter_prefix) {
        match tail.split_once('.') {
            Some((uid, tail)) => (
                canonical_decimal::<u32>(uid)
                    && uid.parse::<u32>().is_ok_and(|u| u <= MAX_VALID_UID),
                tail,
            ),
            None => return false,
        }
    } else {
        let rest = rest.strip_prefix("migrate.").unwrap_or(rest);
        match rest.split_once('.') {
            Some((id, tail)) => (is_lowercase_uuid(id), tail),
            None => return false,
        }
    };
    match tail.split_once('.') {
        Some((pid, salt)) => id_ok && canonical_decimal::<u32>(pid) && is_lowercase_hex16(salt),
        None => false,
    }
}

/// Whether `text` is the canonical decimal form of a `T` (no sign, no leading zero).
fn canonical_decimal<T: std::str::FromStr + ToString>(text: &str) -> bool {
    text.parse::<T>()
        .is_ok_and(|value| value.to_string() == text)
}

/// Whether `text` is a lowercase hyphenated UUID (`8-4-4-4-12` hex digits).
fn is_lowercase_uuid(text: &str) -> bool {
    text.len() == 36
        && text.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_digit() || ('a'..='f').contains(&c),
        })
}

/// Whether `text` is exactly 16 lowercase hex digits.
fn is_lowercase_hex16(text: &str) -> bool {
    text.len() == 16
        && text
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

/// What the sweep did with one candidate name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SweepVerdict {
    /// The orphaned file was unlinked.
    Removed,
    /// A matching regular file younger than [`TEMP_SWEEP_MIN_AGE`]: kept.
    Recent,
    /// Not a removable file (gone, not regular, linked, foreign owner): kept.
    Kept,
}

/// Examines `name` inside the directory `dir_fd` without following it and unlinks it when it
/// is an orphaned temporary file (regular, one link, owned by root or `euid`, old enough).
fn sweep_candidate(
    dir_fd: std::os::fd::RawFd,
    name: &std::ffi::CStr,
    euid: u32,
    now: SystemTime,
) -> Result<SweepVerdict, EvidenceStoreError> {
    let Ok(stat) =
        nix::sys::stat::fstatat(Some(dir_fd), name, nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW)
    else {
        return Ok(SweepVerdict::Kept);
    };
    let regular = stat.st_mode & nix::libc::S_IFMT == nix::libc::S_IFREG;
    if !regular || stat.st_nlink != 1 || (stat.st_uid != 0 && stat.st_uid != euid) {
        return Ok(SweepVerdict::Kept);
    }
    if !old_enough(stat.st_mtime, now) {
        return Ok(SweepVerdict::Recent);
    }
    match nix::unistd::unlinkat(Some(dir_fd), name, nix::unistd::UnlinkatFlags::NoRemoveDir) {
        Ok(()) => Ok(SweepVerdict::Removed),
        Err(nix::errno::Errno::ENOENT) => Ok(SweepVerdict::Kept),
        Err(errno) => Err(EvidenceStoreError::Io(std::io::Error::from(errno))),
    }
}

/// Whether a modification time (seconds since the epoch) is at least [`TEMP_SWEEP_MIN_AGE`]
/// before `now`. A time in the future is never old; one before the epoch always is.
fn old_enough(mtime_secs: nix::libc::time_t, now: SystemTime) -> bool {
    let modified = match u64::try_from(mtime_secs) {
        Ok(secs) => UNIX_EPOCH.checked_add(std::time::Duration::from_secs(secs)),
        Err(_) => Some(UNIX_EPOCH),
    };
    modified
        .and_then(|modified| now.duration_since(modified).ok())
        .is_some_and(|age| age >= TEMP_SWEEP_MIN_AGE)
}

/// Counts the snapshot files (regular, non-symlink `*.enc` entries) of a date partition.
///
/// Temporary files and counter files never end in `.enc` and are not counted.
fn count_snapshot_files(date_dir: &Path) -> Result<u32, EvidenceStoreError> {
    let mut count: u32 = 0;
    for entry in fs::read_dir(date_dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_file() && entry.path().extension().and_then(|e| e.to_str()) == Some("enc") {
            count = count.saturating_add(1);
        }
    }
    Ok(count)
}

/// Path of the persisted per-UID daily counter inside a date partition.
fn daily_count_path(date_dir: &Path, uid: u32) -> PathBuf {
    date_dir.join(format!("{DAILY_COUNT_FILE_PREFIX}{uid}"))
}

/// Reads the persisted daily counter of `uid` in `date_dir` (missing file: `0`).
///
/// The file is opened with `O_NOFOLLOW | O_NONBLOCK`, must be a regular file of at most
/// [`MAX_DAILY_COUNT_FILE_BYTES`] bytes and hold a decimal `u32`; anything else fails closed.
fn read_daily_count(date_dir: &Path, uid: u32) -> Result<u32, EvidenceStoreError> {
    let path = daily_count_path(date_dir, uid);
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(&path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(EvidenceStoreError::Io(e)),
    };
    let meta = file.metadata()?;
    if !meta.is_file() || meta.len() > MAX_DAILY_COUNT_FILE_BYTES {
        return Err(EvidenceStoreError::CorruptPayload(format!(
            "daily counter '{}' is not a small regular file",
            path.display()
        )));
    }
    let mut text = String::new();
    file.take(MAX_DAILY_COUNT_FILE_BYTES)
        .read_to_string(&mut text)
        .map_err(|_| {
            EvidenceStoreError::CorruptPayload(format!(
                "daily counter '{}' is not UTF-8",
                path.display()
            ))
        })?;
    text.trim_end_matches('\n').parse::<u32>().map_err(|_| {
        EvidenceStoreError::CorruptPayload(format!(
            "daily counter '{}' does not hold a decimal count",
            path.display()
        ))
    })
}

/// Atomically replaces the persisted daily counter of `uid` in `date_dir` (mode `0600`).
fn write_daily_count(date_dir: &Path, uid: u32, count: u32) -> Result<(), EvidenceStoreError> {
    let mut rand_bytes = [0u8; 8];
    getrandom::fill(&mut rand_bytes)
        .map_err(|e| EvidenceStoreError::Crypto(format!("Failed to generate random salt: {e}")))?;
    let tmp_path = date_dir.join(format!(
        ".tmp{DAILY_COUNT_FILE_PREFIX}{uid}.{}.{:016x}",
        std::process::id(),
        u64::from_ne_bytes(rand_bytes)
    ));
    if let Err(e) = write_new_file(&tmp_path, format!("{count}\n").as_bytes()) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }
    // rename(2) replaces a planted symlink itself and never follows it.
    if let Err(e) = fs::rename(&tmp_path, daily_count_path(date_dir, uid)) {
        let _ = fs::remove_file(&tmp_path);
        return Err(EvidenceStoreError::Io(e));
    }
    Ok(())
}

/// Date partition and snapshot id a snapshot file is bound to (GitHub #266).
///
/// The date is the parent directory name and the id is the file name up to its first `.`.
/// A component that is missing or not UTF-8 yields an empty string, which never matches the
/// binding of a stored snapshot (only a legacy unbound payload can then be read).
fn snapshot_binding(path: &Path) -> (&str, &str) {
    let date = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let snapshot_id = path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.split('.').next())
        .unwrap_or("");
    (date, snapshot_id)
}

/// Number of CSPRNG overwrite passes applied to a superseded legacy snapshot inode.
const SHRED_PASSES: usize = 3;

/// Overwrite buffer size in bytes.
const SHRED_BUFFER_SIZE: usize = 4096;

/// Overwrites the whole content of `file` in place with CSPRNG bytes, [`SHRED_PASSES`] times,
/// flushing each pass with `fsync` (same scheme as the biometric store, GitHub #179).
///
/// Best effort only: it does not reach the physical blocks on copy-on-write or
/// data-journaling filesystems, snapshots, backups or flash media with wear levelling; the
/// guarantee is the encryption at rest under the evidence key.
fn overwrite_file_contents(file: &File) -> Result<(), EvidenceStoreError> {
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
                EvidenceStoreError::Crypto("Buffer slice out of bounds".to_string())
            })?;
            getrandom::fill(slice).map_err(|e| {
                EvidenceStoreError::Crypto(format!("CSPRNG failure during overwrite: {e}"))
            })?;
            writer.write_all(slice)?;
            written = written.saturating_add(to_write_u64);
        }
        writer.sync_all()?;
    }
    Ok(())
}

/// Reads at most [`MAX_EVIDENCE_FILE_BYTES`] bytes of an opened evidence file.
///
/// A file larger than the bound is refused before it is read, and a file that grows past the
/// bound while it is read is refused too.
fn read_bounded_evidence(file: File) -> Result<Vec<u8>, EvidenceStoreError> {
    let len = file.metadata()?.len();
    if len > MAX_EVIDENCE_FILE_BYTES {
        return Err(EvidenceStoreError::CorruptPayload(format!(
            "evidence file of {len} bytes exceeds {MAX_EVIDENCE_FILE_BYTES} bytes"
        )));
    }
    let mut data = Vec::new();
    file.take(MAX_EVIDENCE_FILE_BYTES.saturating_add(1))
        .read_to_end(&mut data)?;
    if u64::try_from(data.len()).unwrap_or(u64::MAX) > MAX_EVIDENCE_FILE_BYTES {
        return Err(EvidenceStoreError::CorruptPayload(
            "evidence file grew beyond the read bound".to_string(),
        ));
    }
    Ok(data)
}

/// Creates `path` exclusively with mode `0600`, writes `bytes` and syncs it to disk.
fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), EvidenceStoreError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod temp_sweep_fd_tests {
    #![allow(
        clippy::unwrap_used,
        clippy::arithmetic_side_effects,
        reason = "unit tests"
    )]

    //! GitHub #293 (ESL1): the sweep lists a date partition through the descriptor it opened,
    //! so the names it examines and the directory it unlinks from are always the same
    //! directory, even when the partition path is swapped between the open and the listing.

    use super::{EvidenceStore, TEMP_SWEEP_MIN_AGE};
    use crate::config::EvidenceConfig;
    use crate::crypto::MasterKey;
    use std::path::Path;
    use std::time::{Duration, SystemTime};

    const DATE: &str = "2026-09-30";
    const OPENED_TEMP: &str = ".tmp.0f8c2a4e-1b3d-4c5e-8f60-718293a4b5c6.4242.0123456789abcdef";
    const SWAPPED_TEMP: &str =
        ".tmp.migrate.1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d.4243.fedcba9876543210";

    fn plant_old(dir: &Path, name: &str) {
        let path = dir.join(name);
        std::fs::write(&path, b"orphaned ciphertext").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::now() - TEMP_SWEEP_MIN_AGE - Duration::from_secs(60))
            .unwrap();
    }

    fn store(base: &Path, key: &Path) -> EvidenceStore {
        EvidenceStore::new(
            EvidenceConfig::enabled_with_dir(base.to_path_buf(), key.to_path_buf()),
            MasterKey::generate().unwrap(),
        )
    }

    /// The partition is renamed away and replaced by another directory holding a matching
    /// temporary name after it was opened: the replacement is left intact and the orphan of
    /// the opened directory is the one removed.
    #[test]
    fn test_esl_swapped_partition_is_listed_through_the_opened_descriptor() {
        let temp = tempfile::TempDir::new().unwrap();
        let base = temp.path().join("evidence");
        let opened = base.join(DATE);
        std::fs::create_dir_all(&opened).unwrap();
        plant_old(&opened, OPENED_TEMP);
        let moved = base.join("moved-away");
        let store = store(&base, &temp.path().join("ev.key"));

        let mut swaps = 0;
        let report = store
            .sweep_orphaned_temp_files_with(&mut |date| {
                assert_eq!(date, DATE);
                std::fs::rename(base.join(date), &moved).unwrap();
                std::fs::create_dir(base.join(date)).unwrap();
                plant_old(&base.join(date), SWAPPED_TEMP);
                swaps += 1;
            })
            .unwrap();

        assert_eq!(swaps, 1);
        assert!(
            base.join(DATE).join(SWAPPED_TEMP).exists(),
            "a name of the replacement directory must never be examined or removed"
        );
        assert!(
            !moved.join(OPENED_TEMP).exists(),
            "the orphan of the opened partition must be removed through its descriptor"
        );
        assert_eq!(report.removed, 1);
        assert_eq!(report.kept_recent, 0);
        assert!(!report.limit_reached);
    }

    /// The seam is a no-op in production: the public sweep still removes an orphan.
    #[test]
    fn test_esl_unswapped_partition_is_still_swept() {
        let temp = tempfile::TempDir::new().unwrap();
        let base = temp.path().join("evidence");
        let partition = base.join(DATE);
        std::fs::create_dir_all(&partition).unwrap();
        plant_old(&partition, OPENED_TEMP);
        let report = store(&base, &temp.path().join("ev.key"))
            .sweep_orphaned_temp_files()
            .unwrap();
        assert_eq!(report.removed, 1);
        assert!(!partition.join(OPENED_TEMP).exists());
    }
}
