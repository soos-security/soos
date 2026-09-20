# Candid Review Report

- **Date**: 2026-09-20
- **Target Branch / Commit**: `refactor/vision-pipeline-3-model`
- **Audited Files**:
  - `crates/vision/src/crop.rs`
  - `crates/vision/src/error.rs`
  - `crates/vision/src/lib.rs`
  - `crates/vision/src/pipeline.rs`
  - `crates/vision/tests/pipeline_tests.rs`
  - `crates/vision/tests/pad_tests.rs`
  - `crates/vision/tests/zeroize_tests.rs`
  - `crates/vision/tests/bench_tests.rs`
  - `crates/inference-ort/src/mock.rs`
  - `crates/daemon/src/pipeline.rs`
  - `crates/daemon/tests/pipeline_init_tests.rs`
  - `crates/daemon/tests/pipeline_integration_tests.rs`
  - `crates/enrollment-cli/src/service.rs`
  - `crates/enrollment-cli/tests/delete_tests.rs`
  - `crates/enrollment-cli/tests/enroll_tests.rs`
  - `crates/enrollment-cli/tests/list_tests.rs`
  - `crates/enrollment-cli/tests/root_check_tests.rs`
  - `crates/enrollment-cli/tests/verify_tests.rs`
  - `scripts/sync_issue.py`

## 1. Executive Summary
This pull request cleanly restructures `VisionPipeline` from a 4-inference pipeline to the next-generation 3-model architecture mandated by ADR [2026-09-20]. The separate landmark detection model has been removed from `VisionPipeline`, with 5-point facial landmarks extracted directly from the primary `FaceDetection` candidate populated by SCRFD. Furthermore, the Presentation Attack Detection (PAD) stage now receives a 2.7x expanded bounding box context crop resized to 80x80 pixels, while feature embedding extraction retains the canonical 112x112 affine-aligned crop. Downstream consumers across `soos-daemon` and `soos-enrollment-cli` have been systematically updated to preserve workspace-wide build and test integrity.

## 2. Deep Reasoning Audit

### Logic & Architecture
- **Pass**: State transitions and pipeline stages execute in strict security sequence: color conversion -> single-face detection invariant -> landmark extraction check -> 2.7x expanded bbox PAD evaluation -> 112x112 affine alignment -> biometric embedding extraction.
- **Pass**: `expand_bbox_for_pad` accurately computes center-expanded coordinates clamped to `[0, img_w]` and `[0, img_h]`.
- **Pass**: `crop_and_resize` properly validates buffer dimensions, preventing integer overflow with checked arithmetic (`checked_mul`), and applies bilinear interpolation with zero-padding for out-of-bounds regions.

### PAM Concurrency & Deadlines
- **Pass**: No asynchronous runtime (Tokio) or blocking without deadlines in PAM pathways.
- **Pass**: Zero standard output or error stream pollution (`println!`, `dbg!`) introduced.
- **Pass**: Pipeline benchmark confirms full perception pipeline executes well within the 150ms budget (~80ms).

### Panic Safety & Fallback
- **Pass**: Zero `unwrap()` or `expect()` introduced in production code.
- **Pass**: Missing landmarks in `FaceDetection` fail closed with explicit `VisionError::MissingLandmarks` rather than panicking.
- **Pass**: `#![forbid(unsafe_code)]` remains strictly enforced in `soos-vision`.

### Test Integrity & Anti-Weakening
- **Pass**: Pre-written contractual tests in `crates/vision/tests/pipeline_tests.rs` comprehensively cover 3-backend construction, landmark extraction from detection, centered and clamped bbox expansion, 80x80 PAD crop reception, 112x112 embedding crop reception, and config defaults.
- **Pass**: Existing tests were updated exclusively to adapt to the 3-backend constructor signature without altering or relaxing any security threshold, assertion, or invariant.

### Memory & Secret Bounds
- **Pass**: Intermediate `pad_crop` buffer is wrapped in `Zeroizing` and automatically scrubbed on drop.
- **Pass**: `AlignedCropGuard` guarantees zeroization of aligned crops if processing terminates prematurely.
- **Pass**: No raw embeddings or facial frames are logged or leaked across module boundaries.

## 3. Detailed Findings & Action Items
- None. All checks and security invariants pass cleanly with zero compiler warnings.

## 4. Final Verdict
**VERDICT: APPROVED**
