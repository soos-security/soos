//! Contract tests for manifest-declared tensor shapes (review finding VIS-03, GitHub #191).
//!
//! Before this contract `input_shape` / `output_shapes` in `models/manifest.toml` were purely
//! decorative: `ModelRegistry` verified checksums only, so a model with the attested hash but
//! unexpected I/O shapes was caught at first inference at best (or silently mis-fed through the
//! NCHW fallback of the embedding extractor). The contract is:
//! - `input_shape` is the logical `[N, C, H, W]` shape; the optional `input_layout` field
//!   (`"NCHW"` by default, or `"NHWC"`) states the physical tensor layout the ONNX graph uses;
//! - [`ModelMetadata::validate_session_shapes`] compares the physical input dims and every output
//!   against the session, treating symbolic (dynamic, negative) session dims as wildcards, and
//!   fails closed with [`InferenceError::ModelShapeMismatch`] on any disagreement;
//! - the committed manifest describes the shipped embedding model truthfully (VIS-03).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::path::PathBuf;

use soos_inference_ort::error::InferenceError;
use soos_inference_ort::manifest::{ModelManifest, ModelMetadata, TensorLayout};

fn workspace_manifest() -> ModelManifest {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models/manifest.toml");
    ModelManifest::from_file(path).expect("committed models/manifest.toml must parse")
}

fn metadata(input_shape: &[usize], layout: Option<&str>, outputs: &[&[usize]]) -> ModelMetadata {
    let layout_line = layout.map_or(String::new(), |l| format!("input_layout = \"{l}\"\n"));
    let outputs_toml = outputs
        .iter()
        .map(|o| format!("{o:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    let toml = format!(
        r#"
[manifest]
version = "2.0.0"

[models.m]
id = "m"
filename = "m.onnx"
sha256 = "0000000000000000000000000000000000000000000000000000000000000000"
license = "MIT"
source_url = "https://example.invalid/m.onnx"
description = "test model"
input_shape = {input_shape:?}
{layout_line}output_shapes = [{outputs_toml}]
"#
    );
    ModelManifest::from_toml_str(&toml)
        .expect("test manifest must parse")
        .get_model("m")
        .expect("model m")
        .clone()
}

fn assert_mismatch(result: Result<(), InferenceError>, what: &str) {
    match result {
        Err(InferenceError::ModelShapeMismatch { id, detail }) => {
            assert_eq!(id, "m");
            assert!(
                !detail.is_empty(),
                "{what}: mismatch detail must not be empty"
            );
        }
        other => panic!("{what}: expected ModelShapeMismatch, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Layout parsing
// ---------------------------------------------------------------------------

#[test]
fn test_input_layout_defaults_to_nchw() {
    let meta = metadata(&[1, 3, 80, 80], None, &[&[1, 3]]);
    assert_eq!(meta.input_layout, TensorLayout::Nchw);
    assert_eq!(
        meta.expected_input_dims().expect("rank 4"),
        vec![1, 3, 80, 80]
    );
}

#[test]
fn test_input_layout_nhwc_permutes_logical_shape() {
    let meta = metadata(&[1, 3, 112, 112], Some("NHWC"), &[&[1, 512]]);
    assert_eq!(meta.input_layout, TensorLayout::Nhwc);
    assert_eq!(
        meta.expected_input_dims().expect("rank 4"),
        vec![1, 112, 112, 3],
        "NHWC physical dims are [N, H, W, C] of the logical [N, C, H, W] shape"
    );
}

#[test]
fn test_input_layout_nhwc_requires_rank_four() {
    let meta = metadata(&[1, 512], Some("NHWC"), &[]);
    assert_mismatch(meta.expected_input_dims().map(|_| ()), "rank-2 NHWC input");
}

#[test]
fn test_unknown_input_layout_is_rejected_at_parse_time() {
    let toml = r#"
[manifest]
version = "2.0.0"

[models.m]
id = "m"
filename = "m.onnx"
sha256 = "0000000000000000000000000000000000000000000000000000000000000000"
license = "MIT"
source_url = "https://example.invalid/m.onnx"
description = "test model"
input_shape = [1, 3, 112, 112]
input_layout = "CHWN"
"#;
    assert!(
        matches!(
            ModelManifest::from_toml_str(toml),
            Err(InferenceError::ManifestParse(_))
        ),
        "an unknown layout must fail manifest parsing"
    );
}

// ---------------------------------------------------------------------------
// Session shape validation
// ---------------------------------------------------------------------------

#[test]
fn test_validate_exact_static_shapes_ok() {
    let meta = metadata(&[1, 3, 80, 80], None, &[&[1, 3]]);
    meta.validate_session_shapes(&[vec![1, 3, 80, 80]], &[vec![1, 3]])
        .expect("identical static shapes must validate");
}

#[test]
fn test_validate_dynamic_dims_are_wildcards() {
    // Shipped models declare symbolic batch dims (`batch_size`, `unk__556`, `?`), which ORT
    // reports as -1.
    let meta = metadata(&[1, 3, 112, 112], Some("NHWC"), &[&[1, 512]]);
    meta.validate_session_shapes(&[vec![-1, 112, 112, 3]], &[vec![-1, 512]])
        .expect("dynamic batch dims must be accepted");

    let scrfd = metadata(&[1, 3, 640, 640], None, &[&[1, 12800, 1], &[1, 3200, 4]]);
    scrfd
        .validate_session_shapes(&[vec![-1, 3, -1, -1]], &[vec![-1, -1, 1], vec![-1, -1, 4]])
        .expect("fully dynamic spatial dims must be accepted");
}

#[test]
fn test_validate_layout_mismatch_fails_closed() {
    // The VIS-03 case: manifest declares NCHW, the attested graph is NHWC.
    let meta = metadata(&[1, 3, 112, 112], None, &[&[1, 512]]);
    assert_mismatch(
        meta.validate_session_shapes(&[vec![-1, 112, 112, 3]], &[vec![-1, 512]]),
        "NCHW manifest vs NHWC session",
    );
}

#[test]
fn test_validate_concrete_dim_mismatch_fails_closed() {
    let meta = metadata(&[1, 3, 80, 80], None, &[&[1, 3]]);
    assert_mismatch(
        meta.validate_session_shapes(&[vec![1, 3, 96, 96]], &[vec![1, 3]]),
        "spatial mismatch",
    );
    assert_mismatch(
        meta.validate_session_shapes(&[vec![1, 3, 80, 80]], &[vec![1, 2]]),
        "class count mismatch",
    );
}

#[test]
fn test_validate_rank_mismatch_fails_closed() {
    let meta = metadata(&[1, 3, 80, 80], None, &[&[1, 3]]);
    assert_mismatch(
        meta.validate_session_shapes(&[vec![3, 80, 80]], &[vec![1, 3]]),
        "input rank",
    );
    assert_mismatch(
        meta.validate_session_shapes(&[vec![1, 3, 80, 80]], &[vec![3]]),
        "output rank",
    );
}

#[test]
fn test_validate_input_and_output_counts_fail_closed() {
    let meta = metadata(&[1, 3, 80, 80], None, &[&[1, 3]]);
    assert_mismatch(
        meta.validate_session_shapes(&[], &[vec![1, 3]]),
        "zero inputs",
    );
    assert_mismatch(
        meta.validate_session_shapes(&[vec![1, 3, 80, 80], vec![1]], &[vec![1, 3]]),
        "two inputs",
    );
    assert_mismatch(
        meta.validate_session_shapes(&[vec![1, 3, 80, 80]], &[vec![1, 3], vec![1, 3]]),
        "extra output",
    );
    assert_mismatch(
        meta.validate_session_shapes(&[vec![1, 3, 80, 80]], &[]),
        "missing output",
    );
}

#[test]
fn test_validate_without_declared_outputs_checks_input_only() {
    // `output_shapes` is optional in the schema; when absent only the input is pinned.
    let meta = metadata(&[1, 3, 80, 80], None, &[]);
    meta.validate_session_shapes(&[vec![1, 3, 80, 80]], &[vec![1, 3], vec![7]])
        .expect("undeclared outputs are not validated");
}

#[test]
fn test_validate_zero_session_dim_is_not_a_wildcard() {
    let meta = metadata(&[1, 3, 80, 80], None, &[&[1, 3]]);
    assert_mismatch(
        meta.validate_session_shapes(&[vec![1, 3, 0, 80]], &[vec![1, 3]]),
        "zero dim",
    );
}

// ---------------------------------------------------------------------------
// Committed manifest truthfulness (VIS-03)
// ---------------------------------------------------------------------------

#[test]
fn test_workspace_manifest_declares_embedding_layout_nhwc() {
    let manifest = workspace_manifest();
    let arcface = manifest
        .get_model("arcface_w600k_mbf")
        .expect("embedding model entry");
    assert_eq!(
        arcface.input_layout,
        TensorLayout::Nhwc,
        "the attested tf2onnx ArcFace graph takes input_1 as [N, 112, 112, 3]"
    );
    assert_eq!(
        arcface.expected_input_dims().expect("rank 4"),
        vec![1, 112, 112, 3]
    );
}

#[test]
fn test_workspace_manifest_embedding_description_is_truthful() {
    let manifest = workspace_manifest();
    let description = &manifest
        .get_model("arcface_w600k_mbf")
        .expect("embedding model entry")
        .description;
    for needle in ["ResNet34", "NHWC", "tf2onnx", "512"] {
        assert!(
            description.contains(needle),
            "embedding description must state '{needle}': {description}"
        );
    }
    assert!(
        !description.contains("MobileFaceNet"),
        "the attested embedding model is not a MobileFaceNet: {description}"
    );
}

#[test]
fn test_workspace_manifest_nchw_models_keep_default_layout() {
    let manifest = workspace_manifest();
    for id in ["scrfd_500m_kps", "minifasnet_v2_pad"] {
        let meta = manifest.get_model(id).expect("model entry");
        assert_eq!(
            meta.input_layout,
            TensorLayout::Nchw,
            "{id} is an NCHW PyTorch export"
        );
    }
}
