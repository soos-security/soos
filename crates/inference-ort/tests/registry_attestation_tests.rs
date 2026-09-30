//! Contractual tests binding the bytes ONNX Runtime loads to the bytes that were hashed
//! (review finding VIS-04, GitHub #246).
//!
//! Before the fix the registry hashed a model file by path and then re-opened the same path
//! for `commit_from_file`, and `verify_integrity` followed by `get_or_load_session` hashed every
//! model twice. The contract is now:
//! - a model file is read once, into memory, with a hard size bound;
//! - the SHA-256 is computed over those exact bytes;
//! - the ORT session is built from those same bytes (`commit_from_memory`);
//! - bytes verified by `verify_integrity` are consumed by the next `get_or_load_session`
//!   instead of being re-read and re-hashed from disk.
//!
//! A minimal hand-encoded `Identity` ONNX graph provides a real ORT session without any model
//! file under `/var/lib/soos`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::fs;
use std::path::Path;

use soos_inference_ort::error::InferenceError;
use soos_inference_ort::manifest::{ModelManifest, MAX_MODEL_FILE_BYTES};
use soos_inference_ort::registry::{ModelRegistry, RegistryConfig, SharedSession};
use tempfile::tempdir;

const MODEL_ID: &str = "identity";
const MODEL_FILE: &str = "identity.onnx";

// ---------------------------------------------------------------------------
// Minimal ONNX model encoder (Identity graph x[1,3] -> y[1,3])
// ---------------------------------------------------------------------------

fn varint(mut value: u64, out: &mut Vec<u8>) {
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn field_varint(field: u64, value: u64, out: &mut Vec<u8>) {
    varint(field << 3, out);
    varint(value, out);
}

fn field_bytes(field: u64, bytes: &[u8], out: &mut Vec<u8>) {
    varint((field << 3) | 2, out);
    varint(bytes.len() as u64, out);
    out.extend_from_slice(bytes);
}

fn float_value_info(name: &str) -> Vec<u8> {
    let mut dims = Vec::new();
    for dim_value in [1u64, 3u64] {
        let mut dim = Vec::new();
        field_varint(1, dim_value, &mut dim);
        field_bytes(1, &dim, &mut dims);
    }
    let mut tensor_type = Vec::new();
    field_varint(1, 1, &mut tensor_type);
    field_bytes(2, &dims, &mut tensor_type);
    let mut type_proto = Vec::new();
    field_bytes(1, &tensor_type, &mut type_proto);
    let mut value_info = Vec::new();
    field_bytes(1, name.as_bytes(), &mut value_info);
    field_bytes(2, &type_proto, &mut value_info);
    value_info
}

fn identity_model() -> Vec<u8> {
    let mut node = Vec::new();
    field_bytes(1, b"x", &mut node);
    field_bytes(2, b"y", &mut node);
    field_bytes(4, b"Identity", &mut node);

    let mut graph = Vec::new();
    field_bytes(1, &node, &mut graph);
    field_bytes(2, b"soos_identity", &mut graph);
    field_bytes(11, &float_value_info("x"), &mut graph);
    field_bytes(12, &float_value_info("y"), &mut graph);

    let mut opset = Vec::new();
    field_varint(2, 13, &mut opset);

    let mut model = Vec::new();
    field_varint(1, 7, &mut model);
    field_bytes(7, &graph, &mut model);
    field_bytes(8, &opset, &mut model);
    model
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Writes the identity model and a manifest attesting it; returns the registry.
fn attested_registry(dir: &Path) -> ModelRegistry {
    let model = identity_model();
    fs::write(dir.join(MODEL_FILE), &model).expect("write model");
    let hash = ModelManifest::compute_sha256(dir.join(MODEL_FILE)).expect("hash model");
    let manifest = format!(
        r#"
[manifest]
version = "2.0.0"

[models.{MODEL_ID}]
id = "{MODEL_ID}"
filename = "{MODEL_FILE}"
sha256 = "{hash}"
license = "MIT"
source_url = "file:///dev/null"
description = "Identity test graph"
input_shape = [1, 3]
output_shapes = [[1, 3]]
"#
    );
    let manifest_path = dir.join("manifest.toml");
    fs::write(&manifest_path, manifest).expect("write manifest");
    ModelRegistry::new(RegistryConfig::with_manifest(dir, &manifest_path)).expect("registry")
}

/// Runs the identity session and returns its output.
fn run_identity(session: &SharedSession, input: [f32; 3]) -> Vec<f32> {
    let tensor = ort::value::TensorRef::from_array_view(([1usize, 3], input.as_slice()))
        .expect("input tensor");
    let mut guard = session.lock().expect("session mutex");
    let outputs = guard.run(ort::inputs![tensor]).expect("identity run");
    let (_, value) = outputs.into_iter().next().expect("one output");
    let (_, data) = value.try_extract_tensor::<f32>().expect("f32 output");
    data.to_vec()
}

fn garbage() -> Vec<u8> {
    vec![0xA5u8; 4096]
}

// ---------------------------------------------------------------------------
// ModelManifest::read_verified_model
// ---------------------------------------------------------------------------

/// The verified read returns exactly the bytes that were hashed.
#[test]
fn test_read_verified_model_returns_exactly_the_hashed_bytes() {
    let dir = tempdir().expect("tempdir");
    let registry = attested_registry(dir.path());
    let bytes = registry
        .manifest()
        .read_verified_model(MODEL_ID, dir.path().join(MODEL_FILE))
        .expect("attested model must verify");
    assert_eq!(bytes, identity_model());
    assert_eq!(
        ModelManifest::sha256_hex(&bytes),
        registry.manifest().get_model(MODEL_ID).unwrap().sha256,
        "sha256_hex must hash the in-memory bytes exactly like compute_sha256 hashes the file"
    );
}

/// Tampered bytes fail closed with `ChecksumMismatch` and return no bytes.
#[test]
fn test_read_verified_model_rejects_tampered_bytes() {
    let dir = tempdir().expect("tempdir");
    let registry = attested_registry(dir.path());
    fs::write(dir.path().join(MODEL_FILE), garbage()).expect("tamper");
    let err = registry
        .manifest()
        .read_verified_model(MODEL_ID, dir.path().join(MODEL_FILE))
        .expect_err("tampered model must fail");
    assert!(
        matches!(err, InferenceError::ChecksumMismatch { ref id, .. } if id == MODEL_ID),
        "expected ChecksumMismatch, got {err:?}"
    );
}

/// A missing file keeps the historical `ModelNotFound` error.
#[test]
fn test_read_verified_model_missing_file_is_model_not_found() {
    let dir = tempdir().expect("tempdir");
    let registry = attested_registry(dir.path());
    let err = registry
        .manifest()
        .read_verified_model(MODEL_ID, dir.path().join("absent.onnx"))
        .expect_err("missing model must fail");
    assert!(
        matches!(err, InferenceError::ModelNotFound { .. }),
        "expected ModelNotFound, got {err:?}"
    );
}

/// The in-memory read is bounded: a file larger than the limit is rejected before it is
/// buffered, whatever its hash.
#[test]
fn test_read_verified_model_enforces_size_bound() {
    let dir = tempdir().expect("tempdir");
    let registry = attested_registry(dir.path());
    let model_len = identity_model().len() as u64;
    let err = registry
        .manifest()
        .read_verified_model_with_limit(MODEL_ID, dir.path().join(MODEL_FILE), model_len - 1)
        .expect_err("oversized model must fail");
    assert!(
        matches!(err, InferenceError::ModelIo { ref id, .. } if id == MODEL_ID),
        "expected ModelIo for an oversized model, got {err:?}"
    );
    // Exactly at the limit is accepted.
    registry
        .manifest()
        .read_verified_model_with_limit(MODEL_ID, dir.path().join(MODEL_FILE), model_len)
        .expect("a model exactly at the limit must verify");
}

/// The production bound admits the largest attested model (136.6 MB ArcFace ResNet34) with
/// headroom, and stays far below anything that could exhaust daemon memory.
#[test]
fn test_max_model_file_bytes_bounds_the_attested_models() {
    const LARGEST_ATTESTED_MODEL_BYTES: u64 = 136_619_444;
    const {
        assert!(MAX_MODEL_FILE_BYTES >= 2 * LARGEST_ATTESTED_MODEL_BYTES);
        assert!(MAX_MODEL_FILE_BYTES <= 1024 * 1024 * 1024);
    }
}

// ---------------------------------------------------------------------------
// ModelRegistry: hash-once, load-what-was-hashed
// ---------------------------------------------------------------------------

/// Hash-before-load ordering: bytes that do not match the manifest never reach ORT, and a
/// failed load is not cached.
#[test]
fn test_registry_rejects_tampered_bytes() {
    let dir = tempdir().expect("tempdir");
    let mut registry = attested_registry(dir.path());
    fs::write(dir.path().join(MODEL_FILE), garbage()).expect("tamper");
    for _ in 0..2 {
        let err = registry
            .get_or_load_session(MODEL_ID)
            .expect_err("tampered model must never load");
        assert!(
            matches!(err, InferenceError::ChecksumMismatch { .. }),
            "expected ChecksumMismatch, got {err:?}"
        );
    }
}

/// The session built by the registry comes from the verified in-memory bytes: once loaded,
/// overwriting the file on disk has no effect on it.
#[test]
fn test_registry_session_is_independent_of_the_file_after_load() {
    let dir = tempdir().expect("tempdir");
    let mut registry = attested_registry(dir.path());
    let session = registry
        .get_or_load_session(MODEL_ID)
        .expect("attested model must load");
    fs::write(dir.path().join(MODEL_FILE), garbage()).expect("overwrite");
    assert_eq!(run_identity(&session, [1.0, 2.0, 3.0]), vec![1.0, 2.0, 3.0]);
    let cached = registry
        .get_or_load_session(MODEL_ID)
        .expect("cached session must be returned");
    assert!(std::sync::Arc::ptr_eq(&session, &cached));
}

/// `verify_integrity` hashes each model once and hands the verified bytes to the next
/// `get_or_load_session`: the file is not re-opened (a change on disk between the two calls
/// can therefore neither be loaded nor trigger a second hash), and the session runs the
/// verified graph.
#[test]
fn test_registry_loads_bytes_verified_by_integrity_check_without_rehash() {
    let dir = tempdir().expect("tempdir");
    let mut registry = attested_registry(dir.path());
    registry
        .verify_integrity()
        .expect("attested model must verify");
    // Replace the file after attestation: the load must use the attested bytes.
    fs::write(dir.path().join(MODEL_FILE), garbage()).expect("overwrite");
    let session = registry
        .get_or_load_session(MODEL_ID)
        .expect("session must be built from the bytes verified by verify_integrity");
    assert_eq!(run_identity(&session, [4.0, 5.0, 6.0]), vec![4.0, 5.0, 6.0]);
}

/// Verified bytes are single-use: they are held only between `verify_integrity` and the load
/// that consumes them (bounded memory), and a registry without a prior `verify_integrity`
/// still verifies on load.
#[test]
fn test_registry_verified_bytes_are_single_use() {
    let dir = tempdir().expect("tempdir");
    let mut registry = attested_registry(dir.path());
    assert_eq!(registry.pending_verified_models(), 0);
    registry.verify_integrity().expect("verify");
    assert_eq!(
        registry.pending_verified_models(),
        1,
        "verify_integrity must retain the verified bytes of every manifest model"
    );
    registry.get_or_load_session(MODEL_ID).expect("load");
    assert_eq!(
        registry.pending_verified_models(),
        0,
        "the load must consume (drop) the verified bytes"
    );

    // A fresh registry on the tampered directory must fail at load (no cached bytes).
    fs::write(dir.path().join(MODEL_FILE), garbage()).expect("tamper");
    let mut fresh = ModelRegistry::new(RegistryConfig::new(dir.path())).expect("registry");
    let err = fresh
        .get_or_load_session(MODEL_ID)
        .expect_err("tampered model must not load in a fresh registry");
    assert!(
        matches!(err, InferenceError::ChecksumMismatch { .. }),
        "expected ChecksumMismatch, got {err:?}"
    );
}

/// `verify_integrity` still fails closed on a tampered model and caches nothing for it.
#[test]
fn test_registry_verify_integrity_rejects_tampered_bytes() {
    let dir = tempdir().expect("tempdir");
    let mut registry = attested_registry(dir.path());
    fs::write(dir.path().join(MODEL_FILE), garbage()).expect("tamper");
    let err = registry
        .verify_integrity()
        .expect_err("tampered model must fail integrity");
    assert!(
        matches!(err, InferenceError::ChecksumMismatch { .. }),
        "expected ChecksumMismatch, got {err:?}"
    );
    assert_eq!(registry.pending_verified_models(), 0);
    let err = registry
        .get_or_load_session(MODEL_ID)
        .expect_err("tampered model must not load after a failed integrity check");
    assert!(matches!(err, InferenceError::ChecksumMismatch { .. }));
}
