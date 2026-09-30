//! Real-model evidence for the attested face embedding network (review finding VIS-03, GitHub #191).
//!
//! The manifest id `arcface_w600k_mbf` historically claimed an InsightFace "MobileFaceNet w600k"
//! (~3.6 MB, NCHW). The attested file is in fact a 136,619,444-byte tf2onnx export of a Keras
//! ArcFace **ResNet34** (graph input `input_1` = `[N, 112, 112, 3]`, NHWC; output `embedding` =
//! `[N, 512]`). Every other embedding test runs against mocks or synthetic graphs, so this target
//! pins the real metadata and exercises the production registry + extractor on it.
//!
//! Gating (CI stays green on runners without models), identical to `pad_real_model_tests`:
//! models directory `SOOS_MODELS_DIR`, falling back to `/var/lib/soos/models`; an absent model
//! prints `SKIPPED` and returns, unless `SOOS_REQUIRE_REAL_MODELS=1`, in which case absence is a
//! hard failure. Only synthetic, non-biometric inputs are used; no embedding value is committed.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::print_stdout,
    reason = "Evidence test suite utilizes direct assertions, unwraps and prints a measurement report"
)]

use std::path::{Path, PathBuf};
use std::time::Instant;

use soos_inference_ort::embedding::{EmbeddingExtractor, OrtEmbeddingExtractor};
use soos_inference_ort::error::InferenceError;
use soos_inference_ort::manifest::{ModelManifest, TensorLayout};
use soos_inference_ort::registry::{ModelRegistry, RegistryConfig, SharedSession};

/// Manifest identifier of the embedding model (historical name, kept for compatibility).
const EMBEDDING_MODEL_ID: &str = "arcface_w600k_mbf";
/// File name of the embedding model inside the models directory.
const EMBEDDING_MODEL_FILE: &str = "arcface_w600k_mbf.onnx";
/// Exact size of the attested embedding model file.
const EMBEDDING_MODEL_BYTES: u64 = 136_619_444;
/// Default installation directory of the models.
const DEFAULT_MODELS_DIR: &str = "/var/lib/soos/models";
/// Embedding dimensionality produced by the attested network.
const EMBEDDING_DIM: usize = 512;
/// Upper bound of the daemon's inference admission estimate
/// (`soos_daemon::inference::MAX_INFERENCE_ESTIMATE_MS`). A single embedding step slower than
/// this can never fit a decision budget.
const MAX_INFERENCE_ESTIMATE_MS: f64 = 1000.0;
/// Timed iterations of the latency report (after warm-up).
const LATENCY_ITERATIONS: usize = 20;
/// Untimed warm-up iterations.
const LATENCY_WARMUP: usize = 3;

// ---------------------------------------------------------------------------
// Gating helpers
// ---------------------------------------------------------------------------

fn models_dir() -> PathBuf {
    std::env::var_os("SOOS_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_MODELS_DIR))
}

fn repo_manifest_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models/manifest.toml")
}

fn real_models_required() -> bool {
    std::env::var("SOOS_REQUIRE_REAL_MODELS").is_ok_and(|v| v == "1")
}

/// Returns `true` when `file` is installed; otherwise prints `SKIPPED` (or fails when real
/// models are required).
fn model_present(file: &str, test_name: &str) -> bool {
    let dir = models_dir();
    if dir.join(file).is_file() {
        return true;
    }
    assert!(
        !real_models_required(),
        "SOOS_REQUIRE_REAL_MODELS=1 but {file} is missing from {}",
        dir.display()
    );
    println!(
        "SKIPPED {test_name}: {file} not found in {} (set SOOS_MODELS_DIR)",
        dir.display()
    );
    false
}

fn repo_registry() -> ModelRegistry {
    ModelRegistry::new(RegistryConfig::with_manifest(
        models_dir(),
        repo_manifest_path(),
    ))
    .expect("committed models/manifest.toml must parse")
}

/// Loads the real embedding session through the production registry (checksum + shape
/// attestation against the committed manifest), or `None` when the model is not installed.
fn load_real_embedding_session(test_name: &str) -> Option<SharedSession> {
    if !model_present(EMBEDDING_MODEL_FILE, test_name) {
        return None;
    }
    let session = repo_registry()
        .get_or_load_session(EMBEDDING_MODEL_ID)
        .expect("installed embedding model must match the committed manifest SHA-256 and shapes");
    Some(session)
}

/// Named tensor shapes as reported by the session (`name`, dims; negative dims are symbolic).
type NamedShapes = Vec<(String, Vec<i64>)>;

fn session_shapes(session: &SharedSession) -> (NamedShapes, NamedShapes) {
    let guard = session.lock().expect("session mutex");
    let collect = |name: &str, dtype: &ort::value::ValueType| {
        (
            name.to_string(),
            dtype.tensor_shape().expect("tensor value").to_vec(),
        )
    };
    let inputs = guard
        .inputs()
        .iter()
        .map(|i| collect(i.name(), i.dtype()))
        .collect();
    let outputs = guard
        .outputs()
        .iter()
        .map(|o| collect(o.name(), o.dtype()))
        .collect();
    (inputs, outputs)
}

/// Synthetic, non-biometric 112x112 RGB gradient.
fn gradient_112() -> Vec<u8> {
    let mut buf = Vec::with_capacity(112 * 112 * 3);
    for y in 0..112usize {
        for x in 0..112usize {
            buf.push((x * 2) as u8);
            buf.push((y * 2) as u8);
            buf.push(((x + y) % 256) as u8);
        }
    }
    buf
}

fn percentile(sorted_ms: &[f64], p: f64) -> f64 {
    // Nearest-rank on the sorted samples, without float-to-int casts.
    let last = sorted_ms.len() - 1;
    let rank = (0..=last)
        .find(|&i| i as f64 >= last as f64 * p - 0.5)
        .unwrap_or(last);
    sorted_ms[rank]
}

// ---------------------------------------------------------------------------
// Metadata pins
// ---------------------------------------------------------------------------

#[test]
fn test_real_embedding_model_io_metadata_is_pinned() {
    let Some(session) = load_real_embedding_session("test_real_embedding_model_io_metadata") else {
        return;
    };
    let size = std::fs::metadata(models_dir().join(EMBEDDING_MODEL_FILE))
        .expect("model metadata")
        .len();
    let (inputs, outputs) = session_shapes(&session);
    println!("REAL EMBEDDING MODEL metadata: {size} bytes, inputs {inputs:?}, outputs {outputs:?}");

    assert_eq!(
        size, EMBEDDING_MODEL_BYTES,
        "attested file is the 136.6 MB tf2onnx ResNet34 export"
    );
    assert_eq!(inputs.len(), 1, "exactly one graph input");
    assert_eq!(outputs.len(), 1, "exactly one graph output");

    let (in_name, in_shape) = &inputs[0];
    assert_eq!(in_name, "input_1", "Keras/tf2onnx input name");
    assert_eq!(in_shape.len(), 4, "input is a rank-4 image tensor");
    assert!(in_shape[0] < 0, "batch dim is symbolic: {in_shape:?}");
    assert_eq!(
        &in_shape[1..],
        &[112, 112, 3],
        "input is NHWC [N, 112, 112, 3]"
    );

    let (out_name, out_shape) = &outputs[0];
    assert_eq!(out_name, "embedding");
    assert_eq!(out_shape.len(), 2);
    assert!(out_shape[0] < 0, "batch dim is symbolic: {out_shape:?}");
    assert_eq!(
        out_shape[1],
        i64::try_from(EMBEDDING_DIM).expect("dim fits i64"),
        "output is [N, 512]"
    );
}

#[test]
fn test_real_embedding_committed_manifest_matches_session() {
    let Some(session) = load_real_embedding_session("test_real_embedding_committed_manifest")
    else {
        return;
    };
    let manifest = ModelManifest::from_file(repo_manifest_path()).expect("manifest");
    let meta = manifest.get_model(EMBEDDING_MODEL_ID).expect("entry");
    assert_eq!(meta.input_layout, TensorLayout::Nhwc);
    let (inputs, outputs) = session_shapes(&session);
    let inputs: Vec<Vec<i64>> = inputs.into_iter().map(|(_, s)| s).collect();
    let outputs: Vec<Vec<i64>> = outputs.into_iter().map(|(_, s)| s).collect();
    meta.validate_session_shapes(&inputs, &outputs)
        .expect("committed manifest shapes must match the attested session");
}

#[test]
fn test_registry_rejects_real_embedding_model_under_nchw_manifest() {
    if !model_present(
        EMBEDDING_MODEL_FILE,
        "test_registry_rejects_real_embedding_model_under_nchw_manifest",
    ) {
        return;
    }
    // Same checksum, but a manifest claiming the historical NCHW layout: the registry must fail
    // closed instead of handing out a session the extractor would mis-feed.
    let mut manifest = ModelManifest::from_file(repo_manifest_path()).expect("manifest");
    manifest
        .models
        .get_mut(EMBEDDING_MODEL_ID)
        .expect("entry")
        .input_layout = TensorLayout::Nchw;
    let mut registry = ModelRegistry::with_manifest(
        RegistryConfig::with_manifest(models_dir(), repo_manifest_path()),
        manifest,
    );
    match registry.get_or_load_session(EMBEDDING_MODEL_ID) {
        Err(InferenceError::ModelShapeMismatch { id, .. }) => assert_eq!(id, EMBEDDING_MODEL_ID),
        Err(other) => panic!("expected ModelShapeMismatch, got {other:?}"),
        Ok(_) => panic!("an NCHW manifest must not attest the NHWC embedding graph"),
    }
}

#[test]
fn test_real_models_all_pass_committed_manifest_shape_validation() {
    for (id, file) in [
        ("scrfd_500m_kps", "scrfd_500m_kps.onnx"),
        ("minifasnet_v2_pad", "minifasnet_v2_80x80.onnx"),
        (EMBEDDING_MODEL_ID, EMBEDDING_MODEL_FILE),
    ] {
        if !model_present(
            file,
            "test_real_models_all_pass_committed_manifest_shape_validation",
        ) {
            continue;
        }
        repo_registry()
            .get_or_load_session(id)
            .unwrap_or_else(|e| panic!("{id} must load under the committed manifest: {e}"));
    }
}

// ---------------------------------------------------------------------------
// Production extractor on the real network
// ---------------------------------------------------------------------------

#[test]
fn test_real_embedding_extractor_uses_nhwc_and_emits_normalized_512d() {
    let Some(session) = load_real_embedding_session("test_real_embedding_extractor") else {
        return;
    };
    let extractor = OrtEmbeddingExtractor::new(session);
    assert!(
        extractor.is_nhwc(),
        "the extractor must detect the NHWC layout of the attested graph"
    );

    let crop = gradient_112();
    let first = extractor
        .extract_embedding(&crop, 112, 112)
        .expect("real model inference");
    let second = extractor
        .extract_embedding(&crop, 112, 112)
        .expect("real model inference");

    assert_eq!(first.len(), EMBEDDING_DIM);
    assert!(first.as_slice().iter().all(|v| v.is_finite()));
    assert!(first.is_normalized(1e-4), "embedding must be L2-normalized");
    let cos = first.cosine_similarity(&second).expect("same dimension");
    assert!(
        cos > 0.9999,
        "ORT CPU inference must be deterministic on one host (cos = {cos})"
    );
}

#[test]
fn test_real_embedding_latency_report() {
    let Some(session) = load_real_embedding_session("test_real_embedding_latency_report") else {
        return;
    };
    let extractor = OrtEmbeddingExtractor::new(session);
    let crop = gradient_112();
    for _ in 0..LATENCY_WARMUP {
        extractor
            .extract_embedding(&crop, 112, 112)
            .expect("warm-up inference");
    }
    let mut samples_ms = Vec::with_capacity(LATENCY_ITERATIONS);
    for _ in 0..LATENCY_ITERATIONS {
        let started = Instant::now();
        extractor
            .extract_embedding(&crop, 112, 112)
            .expect("timed inference");
        samples_ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    samples_ms.sort_by(f64::total_cmp);
    let p50 = percentile(&samples_ms, 0.50);
    let p95 = percentile(&samples_ms, 0.95);
    println!(
        "REAL EMBEDDING LATENCY (1 intra-op thread, {LATENCY_ITERATIONS} runs): \
         p50 = {p50:.1} ms, p95 = {p95:.1} ms, min = {:.1} ms, max = {:.1} ms",
        samples_ms[0],
        samples_ms[samples_ms.len() - 1]
    );
    assert!(
        p95 < MAX_INFERENCE_ESTIMATE_MS,
        "embedding p95 {p95:.1} ms exceeds the daemon's maximal inference estimate"
    );
}

#[test]
fn test_real_embedding_model_loads_under_manifest_without_input_layout() {
    if !model_present(
        EMBEDDING_MODEL_FILE,
        "test_real_embedding_model_loads_under_manifest_without_input_layout",
    ) {
        return;
    }
    // A manifest installed by an earlier release has no `input_layout` line. The daemon must
    // still load the checksum-attested NHWC model (layout unspecified, not asserted).
    let committed = std::fs::read_to_string(repo_manifest_path()).expect("manifest text");
    let legacy: String = committed
        .lines()
        .filter(|line| !line.trim_start().starts_with("input_layout"))
        .map(|line| format!("{line}\n"))
        .collect();
    assert!(
        !legacy.contains("input_layout ="),
        "legacy manifest must omit input_layout"
    );
    let manifest = ModelManifest::from_toml_str(&legacy).expect("legacy manifest parses");
    assert!(
        !manifest
            .get_model(EMBEDDING_MODEL_ID)
            .expect("entry")
            .input_layout_declared
    );
    let mut registry = ModelRegistry::with_manifest(
        RegistryConfig::with_manifest(models_dir(), repo_manifest_path()),
        manifest,
    );
    let session = registry
        .get_or_load_session(EMBEDDING_MODEL_ID)
        .expect("a legacy manifest without input_layout must load the attested NHWC model");
    assert!(OrtEmbeddingExtractor::new(session).is_nhwc());
}
