# `soos-inference-ort` Crate — Isolated ONNX Runtime CPU Inference Engine

## Overview

The `soos-inference-ort` crate provides a safe, robust, and isolated machine learning inference engine for `soos-daemon`. In accordance with `AI/ARCHITECTURE.md` §7, face authentication relies on local deep neural networks running on CPU with minimal latency.

The crate encapsulates:
1. **Cryptographic Model Attestation**: Enforcing that all ONNX models match expected SHA-256 checksums cataloged in `models/manifest.toml` v2.0.0 before any execution session is instantiated.
2. **Face Detection + Landmarks**: SCRFD 500M KPS ONNX model with multi-stride (8/16/32) distance-to-border box decoding, letterbox padding, BGR input normalization, and embedded 5-point facial keypoints. Replaces the legacy UltraFace Slim 320 + separate landmark model architecture.
3. **Landmark Domain Types**: `FaceLandmarks`, `Point2f`, and `LandmarkDetector` trait for geometric alignment. In the 3-model pipeline, landmark regression is absorbed directly into SCRFD face detection (`OrtScrfdDetector`); `MockLandmarkDetector` provided for deterministic simulation.
4. **Biometric Feature Extraction**: ArcFace embedding extractor generating L2-normalized 512D vectors with symmetric `[-1.0, +1.0]` normalization `(pixel - 127.5) / 127.5` (Verification Matrix Criteria `V2` and `NGM7`). The attested model (manifest id `arcface_w600k_mbf`, a historical name) is a Keras ArcFace **ResNet34** exported with tf2onnx (~34.1 M parameters, 136.6 MB) whose input `input_1` is **NHWC** `[N, 112, 112, 3]` and output `embedding` `[N, 512]` (GitHub #191, ADR 2026-09-30).
5. **Presentation Attack Detection (Anti-Spoofing)**: MiniFASNetV2 80×80 BGR anti-spoofing model with `pixel / 255.0` normalization into `[0.0, 1.0]`, live class index fixed by `DEFAULT_MINIFASNET_LIVE_CLASS_INDEX = 1` (`[PrintPhoto, Live, ScreenReplay]`, single source of truth for every binary), and deterministic post-inference buffer zeroization (Verification Matrix Criteria `NGM8`, `NGM9`, `NGM10`, `PLC1`–`PLC3`).
6. **Hardware-Free Deterministic Simulation**: Mocks (`MockFaceDetector`, `MockLandmarkDetector`, `MockEmbeddingExtractor`, `MockPadDetector`) for seamless headless execution in CI pipelines and developer environments.

---

## Architectural Invariants

1. **Daemon-Isolated Execution**: Only `soos-daemon` executes model inference. The PAM module (`pam_soos.so`) never links or loads ONNX Runtime.
2. **Absolute Prohibition of OpenCV**: Computer vision routines (color conversion, normalization, NMS, tensor preparation) are implemented in pure, safe Rust without OpenCV.
3. **Cryptographic Attestation (Global Security Invariant)**: Every model file is verified against its manifest SHA-256 hash prior to ONNX Runtime session creation. Tampered or corrupted model weights are rejected fail-closed.
3a. **Hashed Bytes Are Loaded Bytes (GitHub #246, VIS-04)**: `ModelManifest::read_verified_model` opens a model file once, checks type and size on the opened handle, reads at most `MAX_MODEL_FILE_BYTES` (512 MiB) into memory and hashes exactly those bytes; `ModelRegistry::get_or_load_session` builds the session with `commit_from_memory` from them and never re-opens the path (`commit_from_file` is banned from the crate by the `vision_attestation_contract` invariant). `ModelRegistry::verify_integrity` retains the verified bytes of every manifest model until the next `get_or_load_session` for that id consumes them, so each model is hashed once at startup (previously twice); on any integrity failure nothing is retained. `pending_verified_models()` reports the retained count. Peak memory during startup is the sum of the attested model sizes (about 141 MB) plus ONNX Runtime's own copy of the model being built.
3b. **Shape Attestation (GitHub #191)**: After the session is built, `ModelRegistry::get_or_load_session` validates its I/O tensor shapes with `ModelMetadata::validate_session_shapes`: exactly one input whose physical dims equal `expected_input_dims()` (the logical `[N, C, H, W]` `input_shape` permuted by `input_layout`, `NCHW` by default or `NHWC`), and, when `output_shapes` is declared, the same number of outputs matching in order. Negative (symbolic) session dims are wildcards; ranks and every other dim, including zero, must be equal. Any mismatch returns `InferenceError::ModelShapeMismatch { id, detail }` and the session is not cached. `ModelMetadata::input_layout_declared` records whether the entry declared `input_layout`. An entry without `input_layout` (a manifest installed by an earlier release) has its layout *unspecified*: the input may be the logical shape in NCHW or NHWC order, rank and every dim are still enforced, the SHA-256 already binds the exact file, and `ModelRegistry` logs a one-time warning that the manifest predates layout attestation. An explicit `input_layout` is always enforced and a wrong value fails closed.
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
    pub fn sha256_hex(bytes: &[u8]) -> String;
    pub fn read_verified_model<P: AsRef<Path>>(&self, id: &str, file_path: P) -> Result<Vec<u8>, InferenceError>;
    pub fn read_verified_model_with_limit<P: AsRef<Path>>(&self, id: &str, file_path: P, max_bytes: u64) -> Result<Vec<u8>, InferenceError>;
}

pub const MAX_MODEL_FILE_BYTES: u64 = 512 * 1024 * 1024;

pub struct ModelRegistry {
    // Config, manifest, and cached sessions
}

impl ModelRegistry {
    pub fn new(config: RegistryConfig) -> Result<Self, InferenceError>;
    pub fn verify_integrity(&self) -> Result<(), InferenceError>;
    pub fn get_or_load_session(&mut self, id: &str) -> Result<Arc<Mutex<Session>>, InferenceError>;
    pub fn pending_verified_models(&self) -> usize;
}

pub enum TensorLayout { Nchw /* "NCHW", default */, Nhwc /* "NHWC" */ }

impl ModelMetadata {
    pub fn expected_input_dims(&self) -> Result<Vec<usize>, InferenceError>;
    pub fn validate_session_shapes(&self, inputs: &[Vec<i64>], outputs: &[Vec<i64>]) -> Result<(), InferenceError>;
}
```

### `FaceDetector` Trait
```rust
pub trait FaceDetector: Send + Sync {
    fn detect(&self, rgb: &[u8], width: u32, height: u32) -> Result<Vec<FaceDetection>, InferenceError>;
}
```
Implemented by `OrtScrfdDetector` (production: SCRFD 500M KPS with multi-stride output parsing, 5-point landmark integration, letterbox padding, and BGR normalization) and `MockFaceDetector`. `OrtFaceDetector` is no longer a detector: its UltraFace inference path was removed (GitHub #249); only the legacy `prepare_input` helper remains because the acceptance test `zeroize_tests::test_inference_input_buffers_zeroized` still calls it, pending that test being re-pointed to the SCRFD input path.

`OrtScrfdDetector` incorporates:
- BGR channel ordering and `(pixel - 127.5) / 128.0` normalization.
- Aspect ratio-preserving letterbox padding to 640×640 with zero-padded borders.
- 9-output multi-stride tensor decoding (strides 8, 16, 32) using distance-to-border box regression.
- 5-point facial keypoints integration directly into `FaceDetection.landmarks`.
- Coordinate un-projection mapping detections back to original camera resolution.
- Startup validation in `OrtScrfdDetector::new` (`validate_output_dims`, GitHub #249): exactly 9 rank-3 outputs, batch dim 1 or symbolic, concrete channel dims with exactly 3 score (1), 3 bbox (4) and 3 keypoint (10) heads, and every concrete anchor dim one of 12800/3200/800 without duplicates. The attested graph reports `[-1, -1, k]`, so per-stride anchor counts are enforced on every frame by `detect` (missing tensor -> `TensorError`).
- Explicit score activation (GitHub #247, VIS-05): `ScoreActivation::Probability` (default; the attested graph ends in `Sigmoid`) or `ScoreActivation::Logit`, chosen once with `with_score_activation`, never per element. `detect` decodes with `decode_stride_checked`, which rejects the frame with `TensorError` on any non-finite or out-of-range score. The historical `decode_stride` entry point infers one activation per tensor (`ScoreActivation::infer`), which keeps its mapping monotonic.

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
Implemented by `MockLandmarkDetector` for testing and deterministic simulation (in the next-gen 3-model pipeline, facial landmarks are absorbed directly into `OrtScrfdDetector`).

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
Implemented by `OrtEmbeddingExtractor` (ArcFace ResNet34 512D; `is_nhwc()` reports the layout detected from the session input) and `MockEmbeddingExtractor` (defaulting to 512D).

### Embedding real-model evidence (`tests/embedding_real_model_tests.rs`, GitHub #191)

Gated exactly like the PAD real-model target (`SOOS_MODELS_DIR`, default `/var/lib/soos/models`;
an absent model prints `SKIPPED`; `SOOS_REQUIRE_REAL_MODELS=1` makes absence a failure). It loads
`arcface_w600k_mbf.onnx` through `ModelRegistry` with the committed manifest and pins: file size
136,619,444 bytes, one input `input_1` `[-1, 112, 112, 3]`, one output `embedding` `[-1, 512]`;
the committed manifest validates against the session while an NCHW manifest for the same file
is rejected with `ModelShapeMismatch`; a legacy manifest without `input_layout` still loads it; all three shipped models pass shape validation; the
production extractor detects NHWC and emits a finite, deterministic, L2-normalized 512D vector on
a synthetic crop; and a latency report (3 warm-ups + 20 timed runs, one intra-op thread) asserts
p95 below the daemon's `MAX_INFERENCE_ESTIMATE_MS` (1000 ms). Measured on the development host:
p50 127.5 ms, p95 170.9 ms.

### `PadDetector` Trait
```rust
pub trait PadDetector: Send + Sync {
    fn evaluate_liveness(
        &self,
        rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<PadResult, InferenceError>;
}
```
Implemented by `OrtPadDetector` (MiniFASNetV2 80×80 BGR) and `MockPadDetector`.

`OrtPadDetector` incorporates:
- BGR channel ordering with `pixel / 255.0` normalization into `[0.0, 1.0]`.
- 80×80 NCHW tensor layout (`[1, 3, 80, 80]`).
- A crop that is not already 80×80 is resampled bilinearly with half-pixel centres (the `cv2.resize` `INTER_LINEAR` convention, GitHub #213) instead of top-left nearest-neighbour; an 80×80 crop is copied exactly, so the golden logits of `pad_real_model_tests` are unchanged. Evidence: `pad_resample_tests`.
- Live class index `DEFAULT_MINIFASNET_LIVE_CLASS_INDEX = 1` (MiniFASNetV2 `[PrintPhoto, Live, ScreenReplay]`, `crates/inference-ort/src/pad.rs`). It is the single source of truth: `soos-daemon` (`pipeline::build_pad_detector`), `soos-enroll` (`service::build_pad_detector`) and `soos-gui` construct the detector with `OrtPadDetector::new`. The explicit-index constructors `new_with_class_index` / `with_live_class_index` are test-only; the repository invariant `test_no_pad_live_class_index_override_outside_tests` rejects them in production code (GitHub #146, ADR 2026-09-29).
- `SharedSession` (`Arc<Mutex<ort::session::Session>>`) is the session handle type returned by `ModelRegistry::get_or_load_session` and accepted by every detector constructor.
- Softmax probability interpretation into `PadResult` with ordinal non-live attack detection (`PrintPhoto` vs `ScreenReplay`).
- Immediate deterministic post-inference zeroization of input tensors (`Zeroizing<Vec<f32>>`).
- Output contract (GitHub #214, PAD-09): every inference must produce exactly `MINIFASNET_CLASS_COUNT = 3` logits and `live_class_index` must address one of them (`validate_pad_output_contract`). Any other length, or an out-of-range index in `interpret_probabilities`, is `InferenceError::PadFailed`, never a spoof or live verdict (previously an out-of-range index silently read `p_live = 0.0`, and a single-logit head scored every frame live because softmax over one logit is 1.0).
- Startup self-test: `OrtPadDetector::self_test(expected_classes)` runs one inference on a fixed synthetic 80×80 grey fixture and returns a `PadSelfTestReport` (`class_count`, `live_class_index`, `liveness_threshold`). `pad_class_count_from_manifest` derives `expected_classes` from the manifest `output_shapes` (`[[1, 3]]`; an omitted entry falls back to 3; any other class count or more than one declared output fails closed). `soos-daemon` runs it through `pipeline::validate_pad_detector` inside `initialize_pipeline`, logs the class index and threshold at info level, and refuses to start on failure.

### PAD real-model evidence (`tests/pad_real_model_tests.rs`, GitHub #172)

Every other PAD test runs `MockPadDetector` or an in-memory identity graph, so only this target
observes the shipped MiniFASNetV2 network. It loads `minifasnet_v2_80x80.onnx` through
`ModelRegistry` attested against the committed `models/manifest.toml` (a present model with a
wrong SHA-256 is a hard failure, never a skip) and drives the production `OrtPadDetector`.

| Variable | Default | Effect |
|---|---|---|
| `SOOS_MODELS_DIR` | `/var/lib/soos/models` | Models directory. Model tests print `SKIPPED` and pass when the PAD model file is absent (CI runners). |
| `SOOS_REQUIRE_REAL_MODELS` | unset | `1` turns a missing model into a hard failure (use on a provisioned host). |
| `SOOS_PAD_CORPUS_DIR` | unset | Enables the corpus APCER / BPCER measurement. Unset: `SKIPPED`. |

Model-only checks (no biometric data, run whenever the model is installed):
- I/O metadata: input `[N, 3, 80, 80]`, output `[N, 3]` (the installed export reports `N = -1`).
- Determinism: identical input gives bit-identical, finite logits; softmax sums to 1.
- Golden logits on three synthetic patterns (uniform grey, gradient, 160×160 checkerboard resized by
  `prepare_input`), tolerance `1e-3`, argmax pinned. A red/blue channel swap moves the logits by about
  0.12, far above the tolerance, so a preprocessing regression is caught.
- `OrtPadDetector::evaluate_liveness` score equals `softmax(logits)[DEFAULT_MINIFASNET_LIVE_CLASS_INDEX]`
  and the synthetic non-face patterns are never accepted as live at the shipped threshold 0.85
  (recorded `p_live` about 0.005–0.006, argmax class 2 / ScreenReplay).

These checks do **not** establish APCER / BPCER. Only the corpus test does.

#### Capturing a PAD corpus (outside the repository)

Face crops are biometric data at rest: the corpus lives outside the repository in a root-only
directory (for example `/var/lib/soos/pad-corpus`, mode `0700`) and must never be added to version
control. Only derived numbers (the `GOLDEN <class>/<file> argmax=… logits=[…]` lines the test prints)
may be recorded in the repository.

```
$SOOS_PAD_CORPUS_DIR/
├── bona_fide/   # >= 20 genuine live crops (RGB and IR, as the pipeline feeds them to PAD)
├── print/       # >= 20 printed-photo presentation attacks
└── screen/      # >= 20 screen / video replay presentation attacks
```

- Format: binary PPM (`P6`, maxval 255), RGB byte order, edge at most 1024 px. Use neutral file
  names (`s_001.ppm`), never user names.
- Content: the PAD crop exactly as the vision pipeline produces it (SCRFD box expanded by
  `pad_bbox_scale` = 2.7 via `soos_vision::crop::pad_crop_window`, the upstream-parity window of
  GitHub #213, before the 80×80 resize). Capture on the target
  laptop camera across several sessions, lighting conditions and distances.
- Run: `SOOS_PAD_CORPUS_DIR=/var/lib/soos/pad-corpus cargo test --locked -p soos-inference-ort --test pad_real_model_tests -- --nocapture`.
- Hard ceilings at the shipped threshold 0.85: APCER at most 5 % per attack species (print, screen),
  BPCER at most 10 %. Fewer than 20 crops in a class fails the test; an empty class counts as a
  100 % error rate, never as 0 %.
- There is no crop-export tool yet; crops must currently be produced out of band (follow-up).

`tests/physical/adversarial_test.sh --mock` runs the plumbing and real-model targets and prints
`SIMULATION – no security metrics`; it never reports APCER / BPCER. Without `--mock`, the physical
session runs as root (`sudo tests/physical/adversarial_test.sh`, after building `soos-enroll` as
your user), because `soos-enroll` refuses a non-root caller and has no bypass flag. It verifies
against the provisioned template store (`-b`/`-k` select another one) and refuses to start when
the target UID is not enrolled. A `verify` call that fails without printing a verdict (camera,
model or store error) aborts the session with exit 2, so an error is never counted as a
rejected attack. `tests/invariants/src/physical_contract.rs` checks every `soos-enroll` flag used
by the physical scripts against the clap definition (GitHub #186).

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
| **NGM7** | ArcFace 512D embeddings and symmetric `[-1.0, +1.0]` normalization | `embedding_tests::test_embedding_normalization_symmetric_range`, `embedding_tests::test_mock_embedding_default_512d` | Validated |
| **NGM8** | MiniFASNetV2 80×80 BGR tensor preparation, `[0.0, 1.0]` normalization, and buffer zeroization | `pad_tests::test_pad_prepare_input_80x80_bgr`, `pad_tests::test_pad_normalization_0_1_range`, `pad_tests::test_pad_invalid_dimensions_message_80x80`, `zeroize_tests::test_inference_input_buffers_zeroized` | Validated |
| **NGM9** | `live_class_index` defaults to `DEFAULT_MINIFASNET_LIVE_CLASS_INDEX` (1) with ordinal spoof attack classification | `pad_tests::test_pad_default_live_class_index_is_one`, `pad_tests::test_pad_class_ordering_live_index_0`, `pad_tests::test_pad_class_ordering_configurable` | Validated |
| **NGM10** | Fail-closed empty probability handling, panic safety, and numerical softmax stability | `pad_tests::test_softmax_numerical_stability`, `pad_tests::test_mock_pad_detector_*` | Validated |
| **ASG1** | Class 1 = live, class 2 = screen replay is a spoof with the default index | `pad_tests::test_screen_replay_detected_as_spoof_with_default_index` | Validated |
| **PLC1–PLC3** | Production wiring never overrides the live class index (daemon, enrollment CLI, repository invariant) | `pad_wiring_tests::test_pipeline_pad_detector_uses_default_live_class_index` (daemon), `pad_wiring_tests::test_enrollment_pad_detector_uses_default_live_class_index` (enrollment CLI), `soos-invariants::tests::test_no_pad_live_class_index_override_outside_tests` | Verified |
| **EMR1–EMR7** | Manifest shape attestation and truthful embedding model metadata (GitHub #191) | `manifest_shape_tests::*`, `embedding_real_model_tests::*` (see `AI/VERIFICATION_MATRIX.md`) | ✅ Verified |
| **Invariant** | `#![forbid(unsafe_code)]` enabled | Invariant test & compile-time crate declaration | Validated |
| **Invariant** | Zero OpenCV across workspace | `tests/invariants::test_no_opencv_in_any_cargo_toml` | Validated |
