//! Daemon groundwork for the optional 4.0x MiniFASNetV1SE PAD ensemble member (GitHub #212,
//! review finding PAD-07; rows VMX4-VMX6).
//!
//! The vision crate already fuses extra PAD members (`VisionPipeline::with_additional_pad_model`,
//! `fuse_pad_results`). The daemon wires the optional member only when the deployed manifest
//! attests `minifasnet_v1se_pad`: the repository manifest does not (the export is not attested
//! yet), so production stays single-model by default. When the entry is present the member is
//! loaded through the attested registry, self-tested against its manifest output shape and
//! added at the 4.0 upstream scale; any failure refuses to start (fail closed).
//!
//! Kept in its own test binary (ORT sessions next to timing-sensitive tests made them flaky).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite utilizes direct assertions and unwraps"
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use soos_daemon::error::DaemonError;
use soos_daemon::pipeline::{
    attach_optional_pad_members, optional_pad_members, PAD_MODEL_ID, SECONDARY_PAD_BBOX_SCALE,
    SECONDARY_PAD_MODEL_ID,
};
use soos_inference_ort::mock::{MockEmbeddingExtractor, MockFaceDetector, MockPadDetector};
use soos_inference_ort::{InferenceError, ModelManifest, ModelRegistry, RegistryConfig};
use soos_vision::{VisionPipeline, VisionPipelineConfig};

#[path = "../../../tests/fixtures/pad_onnx.rs"]
mod pad_onnx;

fn shipped_manifest_source() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models/manifest.toml");
    std::fs::read_to_string(path).expect("repository models/manifest.toml must be readable")
}

fn pad_entry(id: &str, filename: &str, bytes: &[u8]) -> String {
    format!(
        "\n[models.{id}]\nid = \"{id}\"\nfilename = \"{filename}\"\nsha256 = \"{}\"\n\
         license = \"Apache-2.0\"\nsource_url = \"https://example.invalid/{filename}\"\n\
         description = \"PAD fixture\"\ninput_shape = [1, 3, 80, 80]\noutput_shapes = [[1, 3]]\n",
        ModelManifest::sha256_hex(bytes)
    )
}

/// Writes a models directory holding the primary PAD fixture and, optionally, a secondary one.
fn models_dir(dir: &Path, secondary: Option<&[u8]>) -> ModelRegistry {
    let primary = pad_onnx::three_class_pad_model();
    std::fs::write(dir.join("primary.onnx"), &primary).unwrap();
    let mut manifest = String::from("[manifest]\nversion = \"2.0.0\"\n");
    manifest.push_str(&pad_entry(PAD_MODEL_ID, "primary.onnx", &primary));
    if let Some(bytes) = secondary {
        std::fs::write(dir.join("secondary.onnx"), bytes).unwrap();
        manifest.push_str(&pad_entry(SECONDARY_PAD_MODEL_ID, "secondary.onnx", bytes));
    }
    std::fs::write(dir.join("manifest.toml"), manifest).unwrap();
    let registry = ModelRegistry::new(RegistryConfig::new(dir)).expect("registry");
    registry
        .verify_integrity()
        .expect("fixture models are attested");
    registry
}

fn mock_vision() -> VisionPipeline {
    VisionPipeline::new(
        Arc::new(MockFaceDetector::new_empty()),
        Arc::new(MockPadDetector::new_live()),
        Arc::new(MockEmbeddingExtractor::new_default()),
        VisionPipelineConfig::default(),
    )
}

#[test]
fn test_secondary_pad_member_constants_match_upstream() {
    assert_eq!(SECONDARY_PAD_MODEL_ID, "minifasnet_v1se_pad");
    assert_eq!(SECONDARY_PAD_BBOX_SCALE.to_bits(), 4.0f32.to_bits());
}

#[test]
fn test_optional_pad_members_disabled_by_default_in_shipped_manifest() {
    let manifest = ModelManifest::from_toml_str(&shipped_manifest_source()).unwrap();
    assert!(manifest.get_model(SECONDARY_PAD_MODEL_ID).is_none());
    assert!(optional_pad_members(&manifest).is_empty());
}

#[test]
fn test_optional_pad_members_enabled_when_manifest_attests_v1se() {
    let mut source = shipped_manifest_source();
    source.push_str(&pad_entry(
        SECONDARY_PAD_MODEL_ID,
        "minifasnet_v1se_80x80.onnx",
        b"x",
    ));
    let manifest = ModelManifest::from_toml_str(&source).unwrap();
    let members = optional_pad_members(&manifest);
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].0, SECONDARY_PAD_MODEL_ID);
    assert_eq!(members[0].1.to_bits(), SECONDARY_PAD_BBOX_SCALE.to_bits());
}

#[test]
fn test_attach_optional_pad_members_without_entry_keeps_single_model() {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = models_dir(dir.path(), None);
    let vision = attach_optional_pad_members(mock_vision(), &mut registry, 0.85)
        .expect("no optional member is not an error");
    assert_eq!(vision.pad_scales().len(), 1);
}

#[test]
fn test_attach_optional_pad_members_adds_v1se_at_upstream_scale() {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = models_dir(dir.path(), Some(&pad_onnx::three_class_pad_model()));
    let vision = attach_optional_pad_members(mock_vision(), &mut registry, 0.85)
        .expect("an attested, well-shaped secondary PAD model is wired");
    let scales = vision.pad_scales();
    assert_eq!(scales.len(), 2, "primary + secondary member: {scales:?}");
    assert_eq!(scales[1].to_bits(), SECONDARY_PAD_BBOX_SCALE.to_bits());
}

#[test]
fn test_attach_optional_pad_members_fails_closed_on_bad_secondary_head() {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = models_dir(dir.path(), Some(&pad_onnx::single_logit_pad_model()));
    match attach_optional_pad_members(mock_vision(), &mut registry, 0.85) {
        Err(DaemonError::Inference(InferenceError::ModelShapeMismatch { id, .. })) => {
            assert_eq!(id, SECONDARY_PAD_MODEL_ID);
        }
        Err(DaemonError::Inference(InferenceError::PadFailed(_))) => {}
        Err(other) => panic!("unexpected error: {other:?}"),
        Ok(_) => panic!("a secondary PAD model with a single-logit head must refuse to start"),
    }
}
