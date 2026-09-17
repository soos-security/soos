# `soos-inference-ort` Crate — Isolated ONNX Runtime CPU Inference Engine

## Overview

The `soos-inference-ort` crate provides a safe, robust, and isolated machine learning inference engine for `soos-daemon`. In accordance with `AI/ARCHITECTURE.md` §7, face authentication relies on local deep neural networks running on CPU with minimal latency.

The crate encapsulates:
1. **Cryptographic Model Attestation**: Enforcing that all ONNX models match expected SHA-256 checksums cataloged in `models/manifest.toml` before any execution session is instantiated.
2. **Face Detection**: UltraFace Slim 320 ONNX model with deterministic pure-Rust Non-Maximum Suppression (NMS).
3. **Landmark Estimation**: 5-point facial landmark detector (eyes, nose, mouth corners) for geometric alignment.
4. **Biometric Feature Extraction**: MobileFaceNet ArcFace-compatible embedding extractor generating L2-normalized 128D/512D vectors (Verification Matrix Criterion `V2`).
5. **Hardware-Free Deterministic Simulation**: Mocks (`MockFaceDetector`, `MockLandmarkDetector`, `MockEmbeddingExtractor`) for seamless headless execution in CI pipelines and developer environments.

---

## Architectural Invariants

1. **Daemon-Isolated Execution**: Only `soos-daemon` executes model inference. The PAM module (`pam_soos.so`) never links or loads ONNX Runtime.
2. **Absolute Prohibition of OpenCV**: Computer vision routines (color conversion, normalization, NMS, tensor preparation) are implemented in pure, safe Rust without OpenCV.
3. **Cryptographic Attestation (Global Security Invariant)**: Every model file is verified against its manifest SHA-256 hash prior to ONNX Runtime session creation. Tampered or corrupted model weights are rejected fail-closed.
4. **Strict Panic Denial**: Zero `unwrap()` or `expect()` in production pathways; `#![deny(clippy::unwrap_used, clippy::expect_used)]` is strictly enforced.
5. **Deterministic NMS**: Secondary coordinate sort tie-breaking eliminates floating point ambiguity, ensuring identical suppression decisions across platforms.
6. **L2 Normalization (Criterion V2)**: All output embedding vectors satisfy Euclidean norm $\approx 1.0$ within a margin of $10^{-5}$.

---

## Public API

### `ModelManifest` & `ModelRegistry`
```rust
pub struct ModelManifest {
    pub manifest: ManifestHeader,
    pub models: HashMap<String, ModelMetadata>,
}

impl ModelManifest {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, InferenceError>;
    pub fn compute_sha256<P: AsRef<Path>>(file_path: P) -> Result<String, std::io::Error>;
    pub fn verify_model_checksum<P: AsRef<Path>>(&self, id: &str, file_path: P) -> Result<(), InferenceError>;
    pub fn verify_directory<P: AsRef<Path>>(&self, dir: P) -> Result<(), InferenceError>;
}

pub struct ModelRegistry {
    // Config, manifest, and cached sessions
}

impl ModelRegistry {
    pub fn new(config: RegistryConfig) -> Result<Self, InferenceError>;
    pub fn verify_integrity(&self) -> Result<(), InferenceError>;
    pub fn get_or_load_session(&mut self, id: &str) -> Result<Arc<Mutex<Session>>, InferenceError>;
}
```

### `FaceDetector` Trait
```rust
pub trait FaceDetector: Send + Sync {
    fn detect(&self, rgb: &[u8], width: u32, height: u32) -> Result<Vec<FaceDetection>, InferenceError>;
}
```
Implemented by `OrtFaceDetector` (UltraFace Slim 320) and `MockFaceDetector`.

### `LandmarkDetector` Trait
```rust
pub trait LandmarkDetector: Send + Sync {
    fn detect_landmarks(
        &self,
        rgb: &[u8],
        width: u32,
        height: u32,
        face_box: &BoundingBox,
    ) -> Result<FaceLandmarks, InferenceError>;
}
```
Implemented by `OrtLandmarkDetector` (5-point regression) and `MockLandmarkDetector`.

### `EmbeddingExtractor` Trait
```rust
pub trait EmbeddingExtractor: Send + Sync {
    fn extract_embedding(
        &self,
        aligned_crop_rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<BiometricEmbedding, InferenceError>;
}
```
Implemented by `OrtEmbeddingExtractor` (MobileFaceNet ArcFace) and `MockEmbeddingExtractor`.

---

## Model Acquisition & Deployment

Production models are deployed to `/var/lib/soos/models/` via `scripts/download_models.sh`:
- Cryptographic SHA-256 verification against `models/manifest.toml`.
- File permissions enforced: `0644` (owner `root:root`).
- Manifest copied to `/var/lib/soos/models/manifest.toml`.
- Automated fail-fast startup verification: `soos-daemon` validates model integrity before opening the IPC socket.

```bash
# Production model download and installation (requires root)
sudo ./scripts/download_models.sh

# Validation of installed models
./scripts/download_models.sh --check-only
```

---

## Verification Matrix Mapping

| Matrix ID | Criterion | Verification Method | Status |
|---|---|---|---|
| **Global** | Each ONNX model is attested by manifest + SHA-256 checksum | `manifest_tests::test_parse_workspace_manifest_file`, `manifest_tests::test_verify_model_checksum_success_and_tamper_detection`, `registry_tests::test_registry_verify_integrity_missing_files_fails_closed` | Validated |
| **D14** | ONNX model download, SHA-256 verification, and fail-fast startup attestation | `model_deployment_tests::test_download_script_verifies_checksums`, `model_deployment_tests::test_daemon_refuses_start_with_missing_models`, `model_deployment_tests::test_daemon_refuses_start_with_tampered_models` | Validated |
| **V2** | L2-normalized embeddings (norm ≈ 1.0) | `embedding_tests::test_l2_norm_and_normalization_criterion_v2`, `proptest_suite::prop_embedding_normalization_criterion_v2` | Validated |
| **Invariant** | `#![forbid(unsafe_code)]` enabled | Invariant test & compile-time crate declaration | Validated |
| **Invariant** | Zero OpenCV across workspace | `tests/invariants::test_no_opencv_in_any_cargo_toml` | Validated |
