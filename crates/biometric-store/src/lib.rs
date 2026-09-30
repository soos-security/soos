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
    decrypt_payload, encrypt_payload, MasterKey, MAGIC_HEADER, MASTER_KEY_LEN, NONCE_LEN, TAG_LEN,
};
pub use error::BiometricStoreError;
pub use store::{
    BiometricStore, DEFAULT_BIOMETRICS_DIR, MAX_TEMPLATE_FILE_BYTES, TEMPLATE_EXTENSION,
};
pub use template::{BiometricTemplate, TemplateMetadata};
