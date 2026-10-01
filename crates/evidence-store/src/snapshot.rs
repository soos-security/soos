//! Data structures for evidence snapshot records, serialization, and date arithmetic.

use crate::error::EvidenceStoreError;
use crate::frame::{
    to_rgb24, FrameBytes, FrameMetadata, EVIDENCE_RECORD_VERSION, LEGACY_EVIDENCE_RECORD_VERSION,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use zeroize::Zeroizing;

/// Complete evidence record with metadata and image payload.
///
/// The whole record is CBOR-encoded and sealed in the AES-256-GCM envelope, so the version,
/// the capture metadata and the pixels are all authenticated. `Debug` never prints the
/// payload ([`FrameBytes`] is redacted) and the payload is zeroized on drop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRecord {
    /// Record format version ([`EVIDENCE_RECORD_VERSION`]); records written before
    /// GitHub #181 carry no field and decode as [`LEGACY_EVIDENCE_RECORD_VERSION`].
    #[serde(default = "legacy_version")]
    pub format_version: u16,
    /// Unique identifier for the snapshot (UUID v4 format).
    pub snapshot_id: String,
    /// Target Unix user ID.
    pub uid: u32,
    /// Unix epoch timestamp in seconds.
    pub timestamp: u64,
    /// Event reason (e.g. "auth_failure", "liveness_rejected").
    pub reason: String,
    /// Capture metadata describing `image_data`; `None` for legacy records and for opaque
    /// payloads stored through [`crate::EvidenceStore::store_snapshot`].
    #[serde(default)]
    pub frame: Option<FrameMetadata>,
    /// Frame payload exactly as captured (raw pixels or MJPEG, see `frame`).
    pub image_data: FrameBytes,
}

/// `std::io::Write` sink that only counts the bytes written (nothing is stored).
struct ByteCounter(usize);

impl std::io::Write for ByteCounter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 = self
            .0
            .checked_add(buf.len())
            .ok_or_else(|| std::io::Error::other("encoded size overflow"))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn legacy_version() -> u16 {
    LEGACY_EVIDENCE_RECORD_VERSION
}

impl EvidenceRecord {
    /// Serializes the record to CBOR binary format (zeroized on drop).
    ///
    /// The exact encoded size is measured first by a pass through a byte counter that
    /// stores nothing; the buffer is then reserved once at that size, so it never
    /// reallocates while holding pixels (a reallocation would free an unzeroized copy).
    pub fn to_cbor(&self) -> Result<Zeroizing<Vec<u8>>, EvidenceStoreError> {
        let mut counter = ByteCounter(0);
        ciborium::into_writer(self, &mut counter)
            .map_err(|e| EvidenceStoreError::Serialization(format!("CBOR encode failed: {e}")))?;
        let mut buf = Zeroizing::new(Vec::new());
        buf.try_reserve_exact(counter.0).map_err(|e| {
            EvidenceStoreError::Serialization(format!("CBOR buffer allocation failed: {e}"))
        })?;
        ciborium::into_writer(self, &mut *buf)
            .map_err(|e| EvidenceStoreError::Serialization(format!("CBOR encode failed: {e}")))?;
        Ok(buf)
    }

    /// Deserializes and validates an `EvidenceRecord` from CBOR binary format.
    ///
    /// # Errors
    ///
    /// [`EvidenceStoreError::Serialization`] on malformed CBOR;
    /// [`EvidenceStoreError::InvalidFrame`] when [`Self::validate`] fails.
    pub fn from_cbor(slice: &[u8]) -> Result<Self, EvidenceStoreError> {
        let record: Self = ciborium::from_reader(slice)
            .map_err(|e| EvidenceStoreError::Serialization(format!("CBOR decode failed: {e}")))?;
        record.validate()?;
        Ok(record)
    }

    /// Checks the record version and, when present, the frame metadata against the payload.
    ///
    /// # Errors
    ///
    /// [`EvidenceStoreError::InvalidFrame`] for an unknown version, a legacy record carrying
    /// metadata, or metadata inconsistent with the payload.
    pub fn validate(&self) -> Result<(), EvidenceStoreError> {
        match (self.format_version, &self.frame) {
            (LEGACY_EVIDENCE_RECORD_VERSION, None) => Ok(()),
            (EVIDENCE_RECORD_VERSION, None) => Ok(()),
            (EVIDENCE_RECORD_VERSION, Some(meta)) => meta.validate(self.image_data.len()),
            (version, _) => Err(EvidenceStoreError::InvalidFrame(format!(
                "unsupported evidence record version {version}"
            ))),
        }
    }

    /// Reconstructs the frame as packed RGB 8:8:8 (`width * height * 3` bytes, zeroized on
    /// drop) using the stored dimensions and pixel format.
    ///
    /// # Errors
    ///
    /// [`EvidenceStoreError::InvalidFrame`] when the record has no frame metadata (legacy or
    /// opaque payload), holds a compressed MJPEG frame, or is inconsistent.
    pub fn to_rgb24(&self) -> Result<Zeroizing<Vec<u8>>, EvidenceStoreError> {
        let meta = self.frame.as_ref().ok_or_else(|| {
            EvidenceStoreError::InvalidFrame(
                "record has no frame metadata (legacy or opaque payload)".to_string(),
            )
        })?;
        to_rgb24(meta, &self.image_data)
    }
}

/// Metadata summary returned upon successful snapshot storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotResult {
    /// Generated snapshot identifier (UUID v4).
    pub snapshot_id: String,
    /// Target path on disk where snapshot is stored.
    pub path: PathBuf,
    /// Date partition directory (`YYYY-MM-DD`).
    pub date: String,
    /// Target user ID.
    pub uid: u32,
}

/// Summary report of retention rotation operations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RetentionReport {
    /// Number of partitioned date directories pruned.
    pub directories_pruned: usize,
    /// Names of directories pruned (e.g. `["2026-09-01", ...]`).
    pub pruned_dates: Vec<String>,
}

/// A snapshot the legacy migration could not process; its file was left untouched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotMigrationFailure {
    /// Snapshot file (or date partition) that could not be processed.
    pub path: PathBuf,
    /// Error message (paths and ids only, never frame bytes or key material).
    pub error: String,
}

/// Result of [`crate::EvidenceStore::migrate_legacy_snapshots`] (GitHub #287), paths sorted
/// by date partition then file name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SnapshotMigrationReport {
    /// `true` when nothing was written.
    pub dry_run: bool,
    /// Legacy snapshots re-encrypted (in a dry run: that would be re-encrypted).
    pub migrated: Vec<PathBuf>,
    /// Snapshots already in the AAD-bound envelope, left untouched.
    pub already_current: Vec<PathBuf>,
    /// Snapshots or partitions that could not be processed, left untouched.
    pub failed: Vec<SnapshotMigrationFailure>,
}

/// Generates a cryptographically secure UUID v4 string.
pub fn generate_uuid_v4() -> Result<String, EvidenceStoreError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|e| EvidenceStoreError::KeyError(format!("CSPRNG error: {e}")))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5],
        bytes[6], bytes[7],
        bytes[8], bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    ))
}

/// Parses an ISO date string in `YYYY-MM-DD` format into `(year, month, day)`.
pub fn parse_date(date_str: &str) -> Result<(i32, u32, u32), EvidenceStoreError> {
    let parts: Vec<&str> = date_str.split('-').collect();
    if parts.len() != 3 {
        return Err(EvidenceStoreError::InvalidDate(format!(
            "Expected YYYY-MM-DD, got: {date_str}"
        )));
    }

    let year: i32 = parts
        .first()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| EvidenceStoreError::InvalidDate(format!("Invalid year in: {date_str}")))?;
    let month: u32 = parts
        .get(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| EvidenceStoreError::InvalidDate(format!("Invalid month in: {date_str}")))?;
    let day: u32 = parts
        .get(2)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| EvidenceStoreError::InvalidDate(format!("Invalid day in: {date_str}")))?;

    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return Err(EvidenceStoreError::InvalidDate(format!(
            "Out of bounds month or day in: {date_str}"
        )));
    }

    Ok((year, month, day))
}

/// Calculates days since Unix epoch (1970-01-01) for a given Gregorian date using affine calendar formula.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "Standard Euclidean affine calendar arithmetic with bounded calendar parameters"
)]
pub fn days_since_epoch(year: i32, month: u32, day: u32) -> i64 {
    let m = i64::from(month);
    let y = i64::from(year) - if m <= 2 { 1 } else { 0 };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Converts Unix epoch seconds into a formatted `YYYY-MM-DD` date string.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "Standard Euclidean affine calendar decomposition with bounded calendar parameters"
)]
pub fn format_date_from_timestamp(secs: u64) -> String {
    let days = i64::try_from(secs / 86400).unwrap_or(0) + 719468;
    let era = (if days >= 0 { days } else { days - 146096 }) / 146097;
    let doe = days - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + if m <= 2 { 1 } else { 0 };
    format!("{:04}-{:02}-{:02}", y, m, d)
}
