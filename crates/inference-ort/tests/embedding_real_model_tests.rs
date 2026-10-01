//! Real-model evidence for the attested face embedding network (review finding VIS-03, GitHub #191;
//! SFace switch, GitHub #278).
//!
//! Since GitHub #278 the attested embedding model is OpenCV Zoo SFace 2021dec (`sface_2021dec`,
//! 38,696,353 bytes): graph input `data` = `[1, 3, 112, 112]` (NCHW, RGB, raw 0..255, the graph
//! applies `(x - 127.5) / 128` itself), output `fc1` = `[1, 128]`. Every other embedding test runs
//! against mocks or synthetic graphs, so this target pins the real metadata and exercises the
//! production registry + extractor on it (the retired ArcFace ResNet34 is covered by
//! `embedding_preprocessing_evaluation_tests` against `models/retired_models.toml`).
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

/// Manifest identifier of the embedding model.
const EMBEDDING_MODEL_ID: &str = "sface_2021dec";
/// File name of the embedding model inside the models directory.
const EMBEDDING_MODEL_FILE: &str = "sface_2021dec.onnx";
/// Exact size of the attested embedding model file.
const EMBEDDING_MODEL_BYTES: u64 = 38_696_353;
/// Default installation directory of the models.
const DEFAULT_MODELS_DIR: &str = "/var/lib/soos/models";
/// Embedding dimensionality produced by the attested network.
const EMBEDDING_DIM: usize = 128;
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
        "attested file is the 38.7 MB OpenCV Zoo SFace 2021dec export"
    );
    assert_eq!(inputs.len(), 1, "exactly one graph input");
    assert_eq!(outputs.len(), 1, "exactly one graph output");

    let (in_name, in_shape) = &inputs[0];
    assert_eq!(in_name, "data", "SFace input name");
    assert_eq!(in_shape.len(), 4, "input is a rank-4 image tensor");
    assert_eq!(
        in_shape.as_slice(),
        &[1, 3, 112, 112],
        "input is NCHW [1, 3, 112, 112] (batch fixed to 1)"
    );

    let (out_name, out_shape) = &outputs[0];
    assert_eq!(out_name, "fc1");
    assert_eq!(out_shape.len(), 2);
    assert_eq!(
        out_shape.as_slice(),
        &[1, i64::try_from(EMBEDDING_DIM).expect("dim fits i64")],
        "output is [1, 128]"
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
    assert_eq!(meta.input_layout, TensorLayout::Nchw);
    let (inputs, outputs) = session_shapes(&session);
    let inputs: Vec<Vec<i64>> = inputs.into_iter().map(|(_, s)| s).collect();
    let outputs: Vec<Vec<i64>> = outputs.into_iter().map(|(_, s)| s).collect();
    meta.validate_session_shapes(&inputs, &outputs)
        .expect("committed manifest shapes must match the attested session");
}

#[test]
fn test_registry_rejects_real_embedding_model_under_nhwc_manifest() {
    if !model_present(
        EMBEDDING_MODEL_FILE,
        "test_registry_rejects_real_embedding_model_under_nhwc_manifest",
    ) {
        return;
    }
    // Same checksum, but a manifest claiming the retired ArcFace NHWC layout: the registry must
    // fail closed instead of handing out a session the extractor would mis-feed.
    let mut manifest = ModelManifest::from_file(repo_manifest_path()).expect("manifest");
    manifest
        .models
        .get_mut(EMBEDDING_MODEL_ID)
        .expect("entry")
        .input_layout = TensorLayout::Nhwc;
    let mut registry = ModelRegistry::with_manifest(
        RegistryConfig::with_manifest(models_dir(), repo_manifest_path()),
        manifest,
    );
    match registry.get_or_load_session(EMBEDDING_MODEL_ID) {
        Err(InferenceError::ModelShapeMismatch { id, .. }) => assert_eq!(id, EMBEDDING_MODEL_ID),
        Err(other) => panic!("expected ModelShapeMismatch, got {other:?}"),
        Ok(_) => panic!("an NHWC manifest must not attest the NCHW embedding graph"),
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
fn test_real_embedding_extractor_uses_nchw_and_emits_normalized_128d() {
    let Some(session) = load_real_embedding_session("test_real_embedding_extractor") else {
        return;
    };
    let extractor = OrtEmbeddingExtractor::new(session);
    assert_eq!(
        extractor.input_layout(),
        Some(TensorLayout::Nchw),
        "the extractor must detect the NCHW layout of the attested graph"
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
    // still load the checksum-attested NCHW model (layout unspecified, not asserted).
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
        .expect("a legacy manifest without input_layout must load the attested NCHW model");
    assert_eq!(
        OrtEmbeddingExtractor::new(session).input_layout(),
        Some(TensorLayout::Nchw)
    );
}

// ---------------------------------------------------------------------------
// SFace pre-processing contract on the real graph (GitHub #278, SFC5 / SFC6)
// ---------------------------------------------------------------------------

fn read_varint(buf: &[u8], pos: &mut usize) -> u64 {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = buf[*pos];
        *pos += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return value;
        }
    }
    panic!("varint longer than 10 bytes");
}

/// Length-delimited fields (`wire type 2`) of one protobuf message, as `(field, payload)`.
fn length_delimited_fields(buf: &[u8]) -> Vec<(u64, &[u8])> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < buf.len() {
        let key = read_varint(buf, &mut pos);
        match key & 7 {
            0 => {
                read_varint(buf, &mut pos);
            }
            1 => pos += 8,
            5 => pos += 4,
            2 => {
                let len = usize::try_from(read_varint(buf, &mut pos)).expect("length fits usize");
                let end = pos.checked_add(len).expect("length overflow");
                assert!(end <= buf.len(), "truncated protobuf field");
                out.push((key >> 3, &buf[pos..end]));
                pos = end;
            }
            other => panic!("unsupported protobuf wire type {other}"),
        }
    }
    out
}

fn utf8(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).expect("ONNX strings are UTF-8")
}

/// `(op_type, inputs, outputs)` of one graph node.
type GraphNode = (String, Vec<String>, Vec<String>);

/// Nodes and scalar float initializers (`name -> value`) of the ONNX graph.
fn graph_nodes_and_scalars(model: &[u8]) -> (Vec<GraphNode>, Vec<(String, f32)>) {
    // ModelProto 7 = graph; GraphProto 1 = node, 5 = initializer; NodeProto 1/2/4 =
    // input/output/op_type; TensorProto 8 = name, 4 = float_data (packed), 9 = raw_data.
    let graph = length_delimited_fields(model)
        .into_iter()
        .find(|(field, _)| *field == 7)
        .map(|(_, payload)| payload)
        .expect("ModelProto has a graph");
    let mut nodes = Vec::new();
    let mut scalars = Vec::new();
    for (field, payload) in length_delimited_fields(graph) {
        match field {
            1 => {
                let mut node: GraphNode = (String::new(), Vec::new(), Vec::new());
                for (f, p) in length_delimited_fields(payload) {
                    match f {
                        1 => node.1.push(utf8(p)),
                        2 => node.2.push(utf8(p)),
                        4 => node.0 = utf8(p),
                        _ => {}
                    }
                }
                nodes.push(node);
            }
            5 => {
                let mut name = None;
                let mut value = None;
                for (f, p) in length_delimited_fields(payload) {
                    match f {
                        8 => name = Some(utf8(p)),
                        4 | 9 if p.len() == 4 => {
                            value = Some(f32::from_le_bytes([p[0], p[1], p[2], p[3]]));
                        }
                        _ => {}
                    }
                }
                if let (Some(name), Some(value)) = (name, value) {
                    scalars.push((name, value));
                }
            }
            _ => {}
        }
    }
    (nodes, scalars)
}

/// SFC5: the real SFace graph normalizes its raw input itself: `data` feeds only a `Sub` by
/// 127.5 whose output feeds a `Mul` by 1/128. The extractor must therefore feed raw 0..255.
#[test]
fn test_real_sface_graph_normalizes_in_graph() {
    if !model_present(
        EMBEDDING_MODEL_FILE,
        "test_real_sface_graph_normalizes_in_graph",
    ) {
        return;
    }
    let path = models_dir().join(EMBEDDING_MODEL_FILE);
    let size = std::fs::metadata(&path).expect("model metadata").len();
    assert_eq!(
        size, EMBEDDING_MODEL_BYTES,
        "only the attested file is parsed"
    );
    let model = std::fs::read(&path).expect("read model");
    let (nodes, scalars) = graph_nodes_and_scalars(&model);
    let scalar = |name: &str| {
        scalars
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| *v)
            .unwrap_or_else(|| panic!("scalar initializer {name} missing"))
    };

    let consumers: Vec<&GraphNode> = nodes
        .iter()
        .filter(|n| n.1.iter().any(|i| i == "data"))
        .collect();
    assert_eq!(consumers.len(), 1, "exactly one operator consumes data");
    let sub = consumers[0];
    assert_eq!(sub.0, "Sub", "data first goes through a Sub");
    assert_eq!(sub.1[0], "data");
    assert!(
        (scalar(&sub.1[1]) - 127.5).abs() < 1e-6,
        "the Sub subtracts 127.5"
    );

    let sub_out = &sub.2[0];
    let muls: Vec<&GraphNode> = nodes
        .iter()
        .filter(|n| n.1.iter().any(|i| i == sub_out))
        .collect();
    assert_eq!(muls.len(), 1, "exactly one operator consumes the Sub");
    assert_eq!(muls[0].0, "Mul", "the Sub feeds a Mul");
    let factor = muls[0]
        .1
        .iter()
        .find(|i| *i != sub_out)
        .expect("Mul factor");
    assert!(
        (scalar(factor) - 1.0 / 128.0).abs() < 1e-9,
        "the Mul scales by 1/128"
    );
    println!("REAL SFACE GRAPH: data -> Sub(127.5) -> Mul(1/128); the extractor feeds raw 0..255");
}

/// SFC6: the production extractor reproduces the OpenCV `FaceRecognizerSF::feature` recipe
/// (`blobFromImage(aligned, 1, Size(112, 112), Scalar(0, 0, 0), swapRB = true)`: RGB planes, raw
/// 0..255, NCHW) on the real session, then L2-normalizes.
#[test]
fn test_raw_opencv_recipe_matches_the_production_extractor() {
    let Some(session) =
        load_real_embedding_session("test_raw_opencv_recipe_matches_the_production_extractor")
    else {
        return;
    };
    let extractor = OrtEmbeddingExtractor::new(session.clone());
    let crop = gradient_112();
    let production = extractor
        .extract_embedding(&crop, 112, 112)
        .expect("production extractor");

    let plane = 112 * 112;
    let mut input = vec![0.0f32; 3 * plane];
    for (i, px) in crop.as_chunks::<3>().0.iter().enumerate() {
        input[i] = f32::from(px[0]);
        input[plane + i] = f32::from(px[1]);
        input[2 * plane + i] = f32::from(px[2]);
    }
    let tensor = ort::value::TensorRef::from_array_view(([1usize, 3, 112, 112], input.as_slice()))
        .expect("input tensor");
    let raw = {
        let mut guard = session.lock().expect("session mutex");
        let outputs = guard.run(ort::inputs![tensor]).expect("inference");
        outputs
            .values()
            .next()
            .expect("one output")
            .try_extract_tensor::<f32>()
            .expect("f32 output")
            .1
            .to_vec()
    };
    assert_eq!(raw.len(), EMBEDDING_DIM);
    let norm = raw.iter().map(|v| v * v).sum::<f32>().sqrt();
    assert!(norm.is_finite() && norm > 0.0, "raw output norm must be positive");
    let cos: f32 = raw
        .iter()
        .zip(production.as_slice())
        .map(|(r, p)| r / norm * p)
        .sum();
    assert!(
        cos > 0.9999,
        "the OpenCV RGB raw NCHW recipe must reproduce the production extractor (cos = {cos})"
    );
}
