//! Main storage engine for encrypted evidence snapshots.

use crate::config::{EvidenceConfig, DEFAULT_DAILY_CAP_TOTAL};
use crate::crypto::{decrypt_snapshot_payload, encrypt_snapshot_payload, MasterKey};
use crate::error::EvidenceStoreError;
use crate::frame::{
    check_payload_len, EvidenceFrame, FrameBytes, FrameMetadata, EVIDENCE_RECORD_VERSION,
    MAX_EVIDENCE_FILE_BYTES,
};
use crate::snapshot::{
    days_since_epoch, format_date_from_timestamp, generate_uuid_v4, parse_date, EvidenceRecord,
    RetentionReport, SnapshotResult,
};
use nix::fcntl::{Flock, FlockArg};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
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
        let file = File::open(path)?;
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

        let (date, snapshot_id) = snapshot_binding(path);
        let (decrypted, _format) = decrypt_snapshot_payload(&self.key, date, snapshot_id, &data)?;
        EvidenceRecord::from_cbor(&decrypted)
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
