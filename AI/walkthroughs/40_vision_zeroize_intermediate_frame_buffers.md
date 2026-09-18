# Walkthrough 40: Complete Zeroization of Intermediate Frame Buffers

## 1. Overview & Architectural Motivation

Backlog Issue #24 (GitHub Issue #63) eliminates lingering biometric artifacts in freed heap memory across the vision perception and neural inference engines, enforcing strict adherence to `AI/ARCHITECTURE.md` §10 Memory Hygiene:
1. **Zeroization of RGB Frame Buffers (Sub-issue #24.1 / Criterion VZF1)**:
   - In `soos-vision`, camera frames are color-converted into RGB24 format via `convert_to_rgb()`. Previously, this returned a standard `Vec<u8>` heap buffer that remained un-zeroed upon deallocation after `VisionPipeline::process_frame()` completed.
   - The buffer is now wrapped in `zeroize::Zeroizing<Vec<u8>>` at inception inside `process_frame()`, guaranteeing that all raw facial pixels are wiped to zero on all return paths (nominal or early errors such as `NoFaceDetected` or `MultipleFacesDetected`).
   - Intermediate 112x112 aligned face crops are encapsulated within an RAII `AlignedCropGuard` that deterministically wipes the crop buffer upon spoof rejection (`PadFailed`) or feature extraction failure. Only upon full pipeline success is ownership transferred to `PipelineOutput`.
2. **`Zeroize` and `Drop` on `VerificationOutcome` (Sub-issue #24.2 / Criterion VZF2)**:
   - Previously, `PipelineOutput` implemented `Zeroize` and `Drop`, but the outer `VerificationOutcome` struct did not implement `Zeroize` or `Drop`.
   - `VerificationOutcome` now implements both `zeroize::Zeroize` (delegating to `self.output.zeroize()`) and `Drop`, ensuring that cloned and owned verification outcomes deterministically clear the underlying embedding vector and aligned crop.
3. **Zeroization of ONNX Input Tensor Buffers (Sub-issue #24.3 / Criterion VZF3)**:
   - In `soos-inference-ort`, `OrtFaceDetector`, `OrtEmbeddingExtractor`, `OrtLandmarkDetector`, and `OrtPadDetector` prepare normalized NCHW floating-point tensors containing face pixels.
   - Preprocessing logic was isolated into dedicated `prepare_input` methods returning `Zeroizing<Vec<f32>>`.
   - Session execution was refactored to use zero-copy borrowed slice tensor views (`ort::value::TensorRef::from_array_view`), eliminating intermediate `ndarray::Array4` heap allocations and ensuring that `input_data.zeroize()` runs immediately after `session.run()` returns, while `Zeroizing` guarantees zeroization on drop.

---

## 2. Key Changes by Component

### `crates/vision`
- [`pipeline.rs`](file:///home/hadrien/Project/soos/crates/vision/src/pipeline.rs):
  - Implemented `zeroize::Zeroize` and `Drop` for `VerificationOutcome`.
  - Added `AlignedCropGuard` to protect intermediate aligned face crops.
  - Wrapped `convert_to_rgb()` result in `Zeroizing<Vec<u8>>` in `VisionPipeline::process_frame()`.
- [`zeroize_tests.rs`](file:///home/hadrien/Project/soos/crates/vision/tests/zeroize_tests.rs):
  - Added contractual test suite asserting RGB buffer zeroization, `VerificationOutcome` zeroization on drop, and intermediate buffer wiping on spoof failure.

### `crates/inference-ort`
- [`detector.rs`](file:///home/hadrien/Project/soos/crates/inference-ort/src/detector.rs):
  - Added `OrtFaceDetector::prepare_input` returning `Zeroizing<Vec<f32>>`.
  - Converted `detect` to pass `TensorRef::from_array_view` and zeroize input buffer post-inference.
- [`embedding.rs`](file:///home/hadrien/Project/soos/crates/inference-ort/src/embedding.rs):
  - Added `OrtEmbeddingExtractor::prepare_input` returning `Zeroizing<Vec<f32>>`.
  - Converted `extract_embedding` to pass `TensorRef::from_array_view` and zeroize input buffer post-inference.
- [`landmarks.rs`](file:///home/hadrien/Project/soos/crates/inference-ort/src/landmarks.rs):
  - Added `OrtLandmarkDetector::prepare_input` returning `Zeroizing<Vec<f32>>`.
  - Converted `detect_landmarks` to pass `TensorRef::from_array_view` and zeroize input buffer post-inference.
- [`pad.rs`](file:///home/hadrien/Project/soos/crates/inference-ort/src/pad.rs):
  - Added `OrtPadDetector::prepare_input` returning `Zeroizing<Vec<f32>>`.
  - Converted `evaluate_liveness` to pass `TensorRef::from_array_view` and zeroize input buffer post-inference.
- [`zeroize_tests.rs`](file:///home/hadrien/Project/soos/crates/inference-ort/tests/zeroize_tests.rs):
  - Added contractual test `test_inference_input_buffers_zeroized` verifying normalized face pixels and bitwise zeroing across all four inference backends.

### `scripts/sync_issue.py`
- Registered `"fix/vision-zeroize-frames": 24` in `BRANCH_TO_ISSUE` for automated issue tracking.

### `AI/` Documentation
- Updated [`BACKLOG.md`](file:///home/hadrien/Project/soos/AI/BACKLOG.md) checking off sub-issues #24.1, #24.2, and #24.3.
- Updated [`VERIFICATION_MATRIX.md`](file:///home/hadrien/Project/soos/AI/VERIFICATION_MATRIX.md) adding Component `vision-zeroize-frames` (Criteria VZF1, VZF2, VZF3).

---

## 3. Verification & Test Evidence

### Contractual Test Suite (`crates/vision/tests/zeroize_tests.rs`)
1. `test_rgb_buffer_zeroized_after_pipeline`:
   - Validates that the RGB conversion buffer wrapped in `Zeroizing<Vec<u8>>` is bitwise zeroed across all byte positions upon calling `zeroize()`.
2. `test_verification_outcome_zeroize_on_drop`:
   - Validates that `VerificationOutcome` implements `Zeroize`, and calling `zeroize()` wipes both the inner `aligned_crop_rgb` and biometric embedding floats to zero.
3. `test_pipeline_zeroizes_intermediate_buffers_on_error`:
   - Validates that when presentation attack detection flags a spoof frame (`PadFailed`), intermediate buffers are safely deallocated and wiped via RAII guards.

### Contractual Test Suite (`crates/inference-ort/tests/zeroize_tests.rs`)
1. `test_inference_input_buffers_zeroized`:
   - Validates that `OrtFaceDetector::prepare_input`, `OrtEmbeddingExtractor::prepare_input`, `OrtLandmarkDetector::prepare_input`, and `OrtPadDetector::prepare_input` generate non-zero normalized float pixel tensors and strictly wipe all float elements to bitwise zero (`val.to_bits() == 0`).

### Quality Checks
- `cargo fmt --check`: Clean formatting across workspace.
- `cargo clippy --all-targets --all-features -- -D warnings`: Zero warnings.
- `cargo test --all-targets`: 100% test pass rate across all 11 workspace crates.
- `./scripts/candid_review.sh`: Passed all 7 pre-push architectural invariant audits.
