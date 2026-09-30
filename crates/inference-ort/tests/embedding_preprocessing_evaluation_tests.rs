//! Real-model evaluation of the embedding pre-processing contract (GitHub #278, fourth item,
//! follow-up of ADR 2026-09-30 "Face Embedding Model Identity & Retention", matrix SFX1-SFX3).
//!
//! The attested `arcface_w600k_mbf.onnx` (SHA-256 `ffe014a4...683db`) is `arc.onnx` from the
//! Hugging Face repository `garavv/arcface-onnx` (revision `224c23c`, whose LFS object carries
//! the same SHA-256). Its model card documents RGB input normalized as `(x - 127.5) / 128.0`,
//! while the production extractor feeds B, G, R normalized as `(x - 127.5) / 127.5`. These tests
//! record, on the real network and with synthetic non-biometric inputs only:
//!
//! 1. that the graph carries no in-graph normalization (the first operators are a `Transpose`
//!    of `input_1` feeding a `Conv`), so the extractor's arithmetic is the only normalization;
//! 2. that the `127.5` versus `128.0` divisor is template-neutral;
//! 3. that the channel order is not template-neutral: swapping it measurably moves the
//!    embedding, so a switch is a template-format change that must be decided together with
//!    re-enrollment and threshold recalibration (see the ADR for the decision left to the owner).
//!
//! Synthetic patterns cannot tell which channel order the network was trained with, nor how far
//! a real face embedding moves; that needs labelled real captures (tracked as a follow-up).
//!
//! Gating is identical to `embedding_real_model_tests`: models directory `SOOS_MODELS_DIR`,
//! falling back to `/var/lib/soos/models`; an absent model prints `SKIPPED` and returns, unless
//! `SOOS_REQUIRE_REAL_MODELS=1`. No embedding value is committed or logged; only cosine
//! similarities between embeddings of synthetic patterns are printed.

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

use soos_inference_ort::embedding::{EmbeddingExtractor, OrtEmbeddingExtractor};
use soos_inference_ort::registry::{ModelRegistry, RegistryConfig, SharedSession};

/// Manifest identifier of the embedding model (historical name, kept for compatibility).
const EMBEDDING_MODEL_ID: &str = "arcface_w600k_mbf";
/// File name of the embedding model inside the models directory.
const EMBEDDING_MODEL_FILE: &str = "arcface_w600k_mbf.onnx";
/// Exact size of the attested embedding model file; the protobuf walk reads no other file.
const EMBEDDING_MODEL_BYTES: u64 = 136_619_444;
/// Default installation directory of the models.
const DEFAULT_MODELS_DIR: &str = "/var/lib/soos/models";
/// Graph input name of the Keras/tf2onnx export.
const GRAPH_INPUT: &str = "input_1";
/// Aligned crop side.
const SIDE: usize = 112;

/// Divisor used by the production extractor.
const PRODUCTION_STD: f32 = 127.5;
/// Divisor documented by the upstream model card.
const UPSTREAM_STD: f32 = 128.0;
/// Two embeddings closer than this are the same template for any usable match threshold.
const TEMPLATE_NEUTRAL_COS: f32 = 0.999;

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

fn model_present(test_name: &str) -> bool {
    let dir = models_dir();
    if dir.join(EMBEDDING_MODEL_FILE).is_file() {
        return true;
    }
    assert!(
        !std::env::var("SOOS_REQUIRE_REAL_MODELS").is_ok_and(|v| v == "1"),
        "SOOS_REQUIRE_REAL_MODELS=1 but {EMBEDDING_MODEL_FILE} is missing from {}",
        dir.display()
    );
    println!(
        "SKIPPED {test_name}: {EMBEDDING_MODEL_FILE} not found in {} (set SOOS_MODELS_DIR)",
        dir.display()
    );
    false
}

/// Loads the session through the production registry (SHA-256 and shape attestation).
fn load_session(test_name: &str) -> Option<SharedSession> {
    if !model_present(test_name) {
        return None;
    }
    let mut registry = ModelRegistry::new(RegistryConfig::with_manifest(
        models_dir(),
        repo_manifest_path(),
    ))
    .expect("committed models/manifest.toml must parse");
    Some(
        registry
            .get_or_load_session(EMBEDDING_MODEL_ID)
            .expect("installed embedding model must match the committed manifest"),
    )
}

// ---------------------------------------------------------------------------
// Minimal, bounded ONNX protobuf walk (ModelProto.graph -> GraphProto.node)
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

/// Length-delimited fields (`wire type 2`) of one message, as `(field number, payload)`.
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

struct Node {
    op_type: String,
    inputs: Vec<String>,
    outputs: Vec<String>,
}

fn utf8(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).expect("ONNX strings are UTF-8")
}

fn graph_nodes(model: &[u8]) -> Vec<Node> {
    // ModelProto field 7 = graph; GraphProto field 1 = node; NodeProto fields 1/2/4 =
    // input/output/op_type.
    let graph = length_delimited_fields(model)
        .into_iter()
        .find(|(field, _)| *field == 7)
        .map(|(_, payload)| payload)
        .expect("ModelProto has a graph");
    length_delimited_fields(graph)
        .into_iter()
        .filter(|(field, _)| *field == 1)
        .map(|(_, node)| {
            let mut parsed = Node {
                op_type: String::new(),
                inputs: Vec::new(),
                outputs: Vec::new(),
            };
            for (field, payload) in length_delimited_fields(node) {
                match field {
                    1 => parsed.inputs.push(utf8(payload)),
                    2 => parsed.outputs.push(utf8(payload)),
                    4 => parsed.op_type = utf8(payload),
                    _ => {}
                }
            }
            parsed
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Raw pre-processing arms (same session, same 112x112 input, no resampling)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum ChannelOrder {
    Bgr,
    Rgb,
}

/// Runs the real session on an NHWC tensor built from `crop_rgb` with the given channel order
/// and divisor, and returns the L2-normalized embedding.
fn embed_raw(session: &SharedSession, crop_rgb: &[u8], order: ChannelOrder, std: f32) -> Vec<f32> {
    assert_eq!(crop_rgb.len(), SIDE * SIDE * 3);
    let mut input = Vec::with_capacity(crop_rgb.len());
    for px in crop_rgb.as_chunks::<3>().0 {
        let (first, third) = match order {
            ChannelOrder::Bgr => (px[2], px[0]),
            ChannelOrder::Rgb => (px[0], px[2]),
        };
        for value in [first, px[1], third] {
            input.push((f32::from(value) - 127.5) / std);
        }
    }
    let tensor =
        ort::value::TensorRef::from_array_view(([1usize, SIDE, SIDE, 3], input.as_slice()))
            .expect("input tensor");
    let mut guard = session.lock().expect("session mutex");
    let outputs = guard
        .run(ort::inputs![tensor])
        .expect("real model inference");
    let raw = outputs
        .values()
        .next()
        .expect("one output")
        .try_extract_tensor::<f32>()
        .expect("f32 output")
        .1
        .to_vec();
    let norm = raw.iter().map(|v| v * v).sum::<f32>().sqrt();
    assert!(
        norm.is_finite() && norm > 0.0,
        "embedding norm must be positive"
    );
    raw.iter().map(|v| v / norm).collect()
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Synthetic, non-biometric 112x112 RGB patterns with strongly different R and B planes.
fn synthetic_crops() -> Vec<(&'static str, Vec<u8>)> {
    let mut gradient = Vec::with_capacity(SIDE * SIDE * 3);
    let mut warm_disc = Vec::with_capacity(SIDE * SIDE * 3);
    let mut stripes = Vec::with_capacity(SIDE * SIDE * 3);
    for y in 0..SIDE {
        for x in 0..SIDE {
            gradient.extend_from_slice(&[(x * 2) as u8, (y * 2) as u8, ((x + y) % 256) as u8]);

            let dx = x.abs_diff(56);
            let dy = y.abs_diff(60);
            let inside = dx * dx + (dy * dy * 3) / 4 < 42 * 42;
            warm_disc.extend_from_slice(if inside {
                &[210, 150, 120]
            } else {
                &[40, 60, 90]
            });

            let band = ((x / 8) % 2 == 0) as u8;
            stripes.extend_from_slice(&[200 * band + 20, 100, 220 - 200 * band]);
        }
    }
    vec![
        ("gradient", gradient),
        ("warm_disc", warm_disc),
        ("stripes", stripes),
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_real_embedding_graph_has_no_in_graph_normalization() {
    if !model_present("test_real_embedding_graph_has_no_in_graph_normalization") {
        return;
    }
    let path = models_dir().join(EMBEDDING_MODEL_FILE);
    let size = std::fs::metadata(&path).expect("model metadata").len();
    assert_eq!(
        size, EMBEDDING_MODEL_BYTES,
        "only the attested file is parsed (bounded read)"
    );
    let model = std::fs::read(&path).expect("read model");
    let nodes = graph_nodes(&model);
    println!("REAL EMBEDDING GRAPH: {} nodes", nodes.len());
    assert!(!nodes.is_empty(), "graph must hold nodes");

    let consumers = |tensor: &str| -> Vec<&Node> {
        nodes
            .iter()
            .filter(|n| n.inputs.iter().any(|i| i == tensor))
            .collect()
    };

    let first = consumers(GRAPH_INPUT);
    assert_eq!(
        first.len(),
        1,
        "exactly one operator consumes {GRAPH_INPUT}"
    );
    assert_eq!(
        first[0].op_type, "Transpose",
        "the graph input only goes through the NHWC->NCHW transpose"
    );
    let transposed = first[0].outputs.first().expect("transpose output").clone();
    let second = consumers(&transposed);
    assert_eq!(
        second.len(),
        1,
        "exactly one operator consumes the transpose"
    );
    assert_eq!(
        second[0].op_type, "Conv",
        "the transposed input feeds the first convolution directly (no Sub/Mul/Div/Add \
         normalization inside the graph)"
    );
    println!(
        "REAL EMBEDDING GRAPH: {GRAPH_INPUT} -> {} -> {}; normalization is entirely external",
        first[0].op_type, second[0].op_type
    );
}

#[test]
fn test_raw_production_arm_matches_the_production_extractor() {
    let Some(session) = load_session("test_raw_production_arm_matches_the_production_extractor")
    else {
        return;
    };
    let extractor = OrtEmbeddingExtractor::new(session.clone());
    for (name, crop) in synthetic_crops() {
        let production = extractor
            .extract_embedding(&crop, SIDE as u32, SIDE as u32)
            .expect("production extractor");
        let raw = embed_raw(&session, &crop, ChannelOrder::Bgr, PRODUCTION_STD);
        let cos = cosine(production.as_slice(), &raw);
        assert!(
            cos > 0.9999,
            "{name}: the BGR / 127.5 arm must reproduce the production extractor (cos = {cos})"
        );
    }
}

#[test]
fn test_real_embedding_preprocessing_sensitivity_report() {
    let Some(session) = load_session("test_real_embedding_preprocessing_sensitivity_report") else {
        return;
    };
    let mut worst_divisor = 1.0f32;
    let mut worst_order = 1.0f32;
    for (name, crop) in synthetic_crops() {
        let production = embed_raw(&session, &crop, ChannelOrder::Bgr, PRODUCTION_STD);
        let divisor_only = embed_raw(&session, &crop, ChannelOrder::Bgr, UPSTREAM_STD);
        let order_only = embed_raw(&session, &crop, ChannelOrder::Rgb, PRODUCTION_STD);
        let upstream = embed_raw(&session, &crop, ChannelOrder::Rgb, UPSTREAM_STD);
        let c_divisor = cosine(&production, &divisor_only);
        let c_order = cosine(&production, &order_only);
        let c_upstream = cosine(&production, &upstream);
        println!(
            "PREPROCESSING SENSITIVITY [{name}]: cos(BGR/127.5, BGR/128) = {c_divisor:.5}, \
             cos(BGR/127.5, RGB/127.5) = {c_order:.5}, cos(BGR/127.5, RGB/128) = {c_upstream:.5}"
        );
        worst_divisor = worst_divisor.min(c_divisor);
        worst_order = worst_order.min(c_order);
    }
    assert!(
        worst_divisor > TEMPLATE_NEUTRAL_COS,
        "the 127.5 vs 128 divisor must be template-neutral (worst cos = {worst_divisor})"
    );
    assert!(
        worst_order < TEMPLATE_NEUTRAL_COS,
        "the channel order must not be template-neutral, so a switch is a template-format \
         change (worst cos = {worst_order})"
    );
}
