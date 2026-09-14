//! Main storage engine for encrypted evidence snapshots.

use crate::config::EvidenceConfig;
use crate::crypto::{decrypt_payload, encrypt_payload, MasterKey};
use crate::error::EvidenceStoreError;
use crate::snapshot::{
    days_since_epoch, format_date_from_timestamp, generate_uuid_v4, parse_date, EvidenceRecord,
    RetentionReport, SnapshotResult,
};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Primary evidence store engine.
pub struct EvidenceStore {
    config: EvidenceConfig,
    key: MasterKey,
    daily_counts: Mutex<HashMap<(u32, String), u32>>,
}

impl EvidenceStore {
    /// Constructs a new `EvidenceStore` with provided config and key.
    pub fn new(config: EvidenceConfig, key: MasterKey) -> Self {
        Self {
            config,
            key,
            daily_counts: Mutex::new(HashMap::new()),
        }
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

    /// Stores an anti-intrusion evidence snapshot to disk with AES-256-GCM encryption.
    ///
    /// The snapshot is written atomically to `/var/lib/soos/evidence/YYYY-MM-DD/<uuid>.webp.enc`
    /// with mode `0600` and its parent directory with mode `0700`.
    pub fn store_snapshot(
        &self,
        uid: u32,
        reason: &str,
        image_data: &[u8],
        date_override: Option<&str>,
        timestamp_override: Option<u64>,
    ) -> Result<SnapshotResult, EvidenceStoreError> {
        if !self.config.enabled {
            return Err(EvidenceStoreError::Disabled);
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

        // Enforce per-UID daily cap
        {
            let mut counts = self.daily_counts.lock().unwrap_or_else(|e| e.into_inner());
            let entry = counts.entry((uid, date_str.clone())).or_insert(0);
            if *entry >= self.config.daily_cap_per_uid {
                return Err(EvidenceStoreError::DailyCapExceeded {
                    uid,
                    cap: self.config.daily_cap_per_uid,
                    date: date_str,
                });
            }
            *entry = entry.saturating_add(1);
        }

        let snapshot_id = generate_uuid_v4()?;
        let target_dir = self.config.base_dir.join(&date_str);

        if !target_dir.exists() {
            fs::create_dir_all(&target_dir)?;
            fs::set_permissions(&target_dir, fs::Permissions::from_mode(0o700))?;
        }

        let record = EvidenceRecord {
            snapshot_id: snapshot_id.clone(),
            uid,
            timestamp: ts,
            reason: reason.to_string(),
            image_data: image_data.to_vec(),
        };

        let serialized = record.to_cbor()?;
        let ciphertext = encrypt_payload(&self.key, &serialized)?;

        let filename = format!("{snapshot_id}.webp.enc");
        let final_path = target_dir.join(filename);
        let tmp_path = target_dir.join(format!(".tmp.{snapshot_id}.{}", std::process::id()));

        {
            let mut file = File::create(&tmp_path)?;
            fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600))?;
            file.write_all(&ciphertext)?;
            file.sync_all()?;
        }

        fs::rename(&tmp_path, &final_path)?;

        Ok(SnapshotResult {
            snapshot_id,
            path: final_path,
            date: date_str,
            uid,
        })
    }

    /// Loads and decrypts an evidence snapshot from disk.
    pub fn load_snapshot<P: AsRef<Path>>(
        &self,
        path: P,
    ) -> Result<EvidenceRecord, EvidenceStoreError> {
        let path = path.as_ref();
        let mut file = File::open(path)?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)?;

        let decrypted = decrypt_payload(&self.key, &data)?;
        EvidenceRecord::from_cbor(&decrypted)
    }

    /// Gets current daily snapshot count for a UID and date.
    pub fn daily_count(&self, uid: u32, date: &str) -> u32 {
        let lock = self.daily_counts.lock().unwrap_or_else(|e| e.into_inner());
        lock.get(&(uid, date.to_string())).copied().unwrap_or(0)
    }

    /// Lists snapshot file paths for a specific date partition.
    pub fn list_snapshots_for_date(&self, date: &str) -> Result<Vec<PathBuf>, EvidenceStoreError> {
        let date_dir = self.config.base_dir.join(date);
        if !date_dir.exists() {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();
        for entry in fs::read_dir(&date_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("enc") {
                results.push(path);
            }
        }
        results.sort();
        Ok(results)
    }

    /// Executes retention rotation: deletes date directories strictly older than `retention_days`.
    pub fn rotate_retention(
        &self,
        current_date: &str,
    ) -> Result<RetentionReport, EvidenceStoreError> {
        let (cur_y, cur_m, cur_d) = parse_date(current_date)?;
        let current_days = days_since_epoch(cur_y, cur_m, cur_d);

        if !self.config.base_dir.exists() {
            return Ok(RetentionReport::default());
        }

        let mut pruned_dates = Vec::new();

        for entry in fs::read_dir(&self.config.base_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                if let Some(folder_name) = path.file_name().and_then(|s| s.to_str()) {
                    if let Ok((y, m, d)) = parse_date(folder_name) {
                        let dir_days = days_since_epoch(y, m, d);
                        let age_days = current_days.saturating_sub(dir_days);
                        if age_days > i64::from(self.config.retention_days) {
                            fs::remove_dir_all(&path)?;
                            pruned_dates.push(folder_name.to_string());
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
