//! Data structures for evidence snapshot records, serialization, and date arithmetic.

use crate::error::EvidenceStoreError;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Complete evidence record with metadata and raw image payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRecord {
    /// Unique identifier for the snapshot (UUID v4 format).
    pub snapshot_id: String,
    /// Target Unix user ID.
    pub uid: u32,
    /// Unix epoch timestamp in seconds.
    pub timestamp: u64,
    /// Event reason (e.g. "auth_failure", "liveness_rejected").
    pub reason: String,
    /// Binary frame payload (e.g. WebP / JPEG encoded data).
    pub image_data: Vec<u8>,
}

impl EvidenceRecord {
    /// Serializes the record to CBOR binary format.
    pub fn to_cbor(&self) -> Result<Vec<u8>, EvidenceStoreError> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf)
            .map_err(|e| EvidenceStoreError::Serialization(format!("CBOR encode failed: {e}")))?;
        Ok(buf)
    }

    /// Deserializes an `EvidenceRecord` from CBOR binary format.
    pub fn from_cbor(slice: &[u8]) -> Result<Self, EvidenceStoreError> {
        ciborium::from_reader(slice)
            .map_err(|e| EvidenceStoreError::Serialization(format!("CBOR decode failed: {e}")))
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
