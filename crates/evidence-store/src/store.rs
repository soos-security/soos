//! Main storage engine for encrypted evidence snapshots.

use crate::config::{EvidenceConfig, DEFAULT_DAILY_CAP_TOTAL};
use crate::crypto::{decrypt_payload, encrypt_payload, MasterKey};
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
pub const OPAQUE_SNAPSHOT_EXTENSION: &str = ".webp.enc";

/// In-memory capture counters of the current day (GitHub #276).
///
/// Only one day is tracked: when a snapshot is requested for another date the counters are
/// reset to that date, so the map never holds more than one entry per UID that stored a
/// snapshot that day, which the global cap bounds.
#[derive(Debug, Default)]
struct DailyCounters {
    date: String,
    per_uid: HashMap<u32, u32>,
    total: u32,
}

impl DailyCounters {
    /// Switches the counters to `date`, dropping every counter of any other date.
    fn roll_to(&mut self, date: &str) {
        if self.date != date {
            self.date = date.to_string();
            self.per_uid.clear();
            self.per_uid.shrink_to_fit();
            self.total = 0;
        }
    }

    fn count(&self, uid: u32, date: &str) -> u32 {
        if self.date == date {
            self.per_uid.get(&uid).copied().unwrap_or(0)
        } else {
            0
        }
    }

    fn total(&self, date: &str) -> u32 {
        if self.date == date {
            self.total
        } else {
            0
        }
    }
}

/// Primary evidence store engine.
pub struct EvidenceStore {
    config: EvidenceConfig,
    key: MasterKey,
    daily_cap_total: u32,
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

        // Enforce the per-UID and the global daily caps (GitHub #276). Counters of any
        // other day are dropped first, so the map stays bounded by the global cap.
        {
            let mut counts = self.daily_counts.lock().unwrap_or_else(|e| e.into_inner());
            counts.roll_to(&date_str);
            let uid_count = counts.count(uid, &date_str);
            if uid_count >= self.config.daily_cap_per_uid {
                return Err(EvidenceStoreError::DailyCapExceeded {
                    uid,
                    cap: self.config.daily_cap_per_uid,
                    date: date_str,
                });
            }
            if counts.total >= self.daily_cap_total {
                return Err(EvidenceStoreError::GlobalDailyCapExceeded {
                    cap: self.daily_cap_total,
                    date: date_str,
                });
            }
            counts.per_uid.insert(uid, uid_count.saturating_add(1));
            counts.total = counts.total.saturating_add(1);
        }

        let snapshot_id = generate_uuid_v4()?;
        let target_dir = self.config.base_dir.join(&date_str);

        // Pre-creation symlink check on date directory (Sub-issue #30.1)
        match fs::symlink_metadata(&target_dir) {
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
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
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
            Err(e) => return Err(EvidenceStoreError::Io(e)),
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
        let ciphertext = encrypt_payload(&self.key, &serialized)?;
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

        {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&tmp_path)?;
            file.write_all(&ciphertext)?;
            file.sync_all()?;
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

        fs::rename(&tmp_path, &final_path)?;

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

        let decrypted = decrypt_payload(&self.key, &data)?;
        EvidenceRecord::from_cbor(&decrypted)
    }

    /// Gets current daily snapshot count for a UID and date.
    ///
    /// Only the most recent day is tracked; any other date reports `0`.
    pub fn daily_count(&self, uid: u32, date: &str) -> u32 {
        let lock = self.daily_counts.lock().unwrap_or_else(|e| e.into_inner());
        lock.count(uid, date)
    }

    /// Gets the snapshot count of `date` across all UIDs (`0` for an untracked date).
    pub fn daily_total(&self, date: &str) -> u32 {
        let lock = self.daily_counts.lock().unwrap_or_else(|e| e.into_inner());
        lock.total(date)
    }

    /// Number of per-UID daily counters currently held in memory (bounded by the global cap).
    pub fn tracked_daily_counters(&self) -> usize {
        let lock = self.daily_counts.lock().unwrap_or_else(|e| e.into_inner());
        lock.per_uid.len()
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

        Ok(RetentionReport {
            directories_pruned,
            pruned_dates,
        })
    }
}
