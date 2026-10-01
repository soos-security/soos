//! `soos-biometric-store`: Encrypted biometric embedding storage with CRUD operations.
//!
//! Provides AES-256-GCM authenticated encryption at rest for biometric templates,
//! atomic file writes with strict POSIX permissions (`0600`), metadata tracking,
//! and automatic memory zeroization on drop.

#![forbid(unsafe_code)]

pub mod crypto;
pub mod error;
pub mod store;
pub mod template;

pub use crypto::{
    decrypt_payload, decrypt_template_payload, encrypt_payload, encrypt_template_payload,
    template_aad, MasterKey, PayloadFormat, BOUND_FORMAT_MARKER, MAGIC_HEADER, MASTER_KEY_LEN,
    NONCE_LEN, PAYLOAD_FORMAT_VERSION, TAG_LEN, TEMPLATE_AAD_DOMAIN,
};
pub use error::BiometricStoreError;
pub use store::{
    BiometricStore, TempSweepReport, TemplateMigration, TemplateMigrationFailure,
    TemplateMigrationReport, DEFAULT_BIOMETRICS_DIR, MAX_TEMPLATE_FILE_BYTES,
    MAX_TEMP_SWEEP_REMOVALS, STORE_LOCK_TIMEOUT, TEMPLATE_EXTENSION, TEMP_SWEEP_MIN_AGE,
};
pub use template::{BiometricTemplate, TemplateMetadata};
