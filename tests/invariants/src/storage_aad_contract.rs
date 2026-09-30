//! AES-GCM associated-data binding invariants for the encrypted stores (GitHub #266, STO-22).
//!
//! The stores must write only the AAD-bound envelope: `soos-biometric-store` seals templates
//! with `encrypt_template_payload` (UID, file role, format version) and `soos-evidence-store`
//! seals snapshots with `encrypt_snapshot_payload` (date partition, snapshot id, file role,
//! format version). The legacy unbound `encrypt_payload` stays available for context-free
//! payloads and tests, but no store write path may call it, and the read paths must go through
//! the migrating decoders so that legacy files stay readable.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Architectural invariant test runner utilizes direct assertions and panics"
)]

use std::fs;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set");
    Path::new(&manifest_dir)
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

/// Source of `rel` without its `#[cfg(test)]` tail and without line comments.
fn production_source(rel: &str) -> String {
    let text = fs::read_to_string(workspace_root().join(rel))
        .unwrap_or_else(|e| panic!("cannot read {rel}: {e}"));
    let body = text.split("#[cfg(test)]").next().unwrap_or("");
    body.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `true` when `source` calls the free function `name` (not a longer identifier ending in it).
fn calls(source: &str, name: &str) -> bool {
    let needle = format!("{name}(");
    source.match_indices(&needle).any(|(i, _)| {
        source[..i]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
    })
}

#[test]
fn test_stores_write_only_aad_bound_payloads() {
    let bio = production_source("crates/biometric-store/src/store.rs");
    assert!(calls(&bio, "encrypt_template_payload"));
    assert!(calls(&bio, "decrypt_template_payload"));
    assert!(
        !calls(&bio, "encrypt_payload") && !calls(&bio, "decrypt_payload"),
        "the biometric store must not use the unbound payload codec"
    );

    let evd = production_source("crates/evidence-store/src/store.rs");
    assert!(calls(&evd, "encrypt_snapshot_payload"));
    assert!(calls(&evd, "decrypt_snapshot_payload"));
    assert!(
        !calls(&evd, "encrypt_payload") && !calls(&evd, "decrypt_payload"),
        "the evidence store must not use the unbound payload codec"
    );
}

#[test]
fn test_crypto_modules_pass_associated_data() {
    for rel in [
        "crates/biometric-store/src/crypto.rs",
        "crates/evidence-store/src/crypto.rs",
    ] {
        let src = production_source(rel);
        assert!(
            src.contains("Payload {"),
            "{rel} must encrypt and decrypt through aead::Payload with associated data"
        );
        assert!(
            src.contains("PAYLOAD_FORMAT_VERSION: u8 = 2"),
            "{rel} must declare the bound format version 2"
        );
    }
}

#[test]
fn test_storage_docs_state_the_rollback_limit() {
    for rel in [
        "Docs/BIOMETRIC_STORE_CRATE.md",
        "Docs/EVIDENCE_STORE_CRATE.md",
    ] {
        let text = fs::read_to_string(workspace_root().join(rel)).unwrap();
        assert!(
            text.contains("Associated Data") && text.contains("rollback"),
            "{rel} must document the AAD binding and what rollback it cannot detect"
        );
    }
}
