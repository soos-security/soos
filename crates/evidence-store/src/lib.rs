//! Local encrypted anti-intrusion evidence snapshot storage for the soos daemon.
//!
//! Provides strictly opt-in encrypted snapshot storage with 7-day retention
//! rotation and per-UID daily capture limits under `/var/lib/soos/evidence/`.

#![forbid(unsafe_code)]

pub mod config;
pub mod crypto;
pub mod error;
pub mod frame;
pub mod snapshot;
pub mod store;

pub use config::{
    EvidenceConfig, DEFAULT_DAILY_CAP_PER_UID, DEFAULT_DAILY_CAP_TOTAL, DEFAULT_EVIDENCE_DIR,
    DEFAULT_KEY_PATH, DEFAULT_RETENTION_DAYS,
};
pub use crypto::MasterKey;
pub use error::EvidenceStoreError;
pub use frame::{
    EvidenceFrame, EvidencePixelFormat, FrameBytes, FrameMetadata, EVIDENCE_RECORD_VERSION,
    LEGACY_EVIDENCE_RECORD_VERSION, MAX_EVIDENCE_DIMENSION, MAX_EVIDENCE_FILE_BYTES,
    MAX_EVIDENCE_IMAGE_BYTES,
};
pub use snapshot::{EvidenceRecord, RetentionReport, SnapshotResult};
pub use store::{
    EvidenceStore, DAILY_COUNT_FILE_PREFIX, FRAME_SNAPSHOT_EXTENSION, MAX_VALID_UID,
    OPAQUE_SNAPSHOT_EXTENSION,
};
