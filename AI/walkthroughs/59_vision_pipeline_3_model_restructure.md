# Walkthrough 59 — Restructure VisionPipeline for 3-Model Architecture

> **Date**: 2026-09-20  
> **Issue**: Issue #41 (`refactor/vision-pipeline-3-model`, GitHub #107)  
> **Verification Matrix**: `NGM11`, `NGM12`, `NGM13`  
> **Scope**: `crates/vision/src/pipeline.rs`, `crates/vision/src/crop.rs`, `crates/vision/src/error.rs`, `crates/vision/src/lib.rs`, `crates/vision/tests/pipeline_tests.rs`, `crates/vision/tests/pad_tests.rs`, `crates/vision/tests/zeroize_tests.rs`, `crates/vision/tests/bench_tests.rs`, `crates/inference-ort/src/mock.rs`, `crates/daemon/src/pipeline.rs`, `crates/daemon/tests/pipeline_init_tests.rs`, `crates/daemon/tests/pipeline_integration_tests.rs`, `crates/enrollment-cli/src/service.rs`, `crates/enrollment-cli/tests/`, `AI/VERIFICATION_MATRIX.md`, `AI/BACKLOG.md`

---

## 1. Problem Statement & Motivation

Prior to this refactoring, `VisionPipeline` orchestrated 4 distinct neural inference backends:
```text
Frame -> RGB -> FaceDetector -> LandmarkDetector -> PadDetector -> EmbeddingExtractor
```
With the integration of the next-generation **SCRFD** detector (Issue #37), face detection and 5-point landmark prediction are unified into a single inference pass. Furthermore, **MiniFASNetV2** (Issue #40) requires an expanded bounding box context crop (2.7× from face center, resized to 80×80) rather than the tightly aligned 112×112 crop used by ArcFace feature extraction.

`VisionPipeline` has been restructured to:
1. Construct with exactly 3 neural backends: `FaceDetector`, `PadDetector`, and `EmbeddingExtractor`.
2. Extract 5-point landmarks directly from the primary `FaceDetection` result.
3. Compute a 2.7× expanded bounding box centered on the face and clamped to image bounds.
4. Crop and resize the expanded bounding box to 80×80 for Presentation Attack Detection (PAD).
5. Retain 112×112 affine alignment using landmarks for ArcFace biometric embedding extraction.
6. Provide distinct configuration parameters in `VisionPipelineConfig` for PAD vs embedding crops (`pad_target_width: 80`, `pad_target_height: 80`, `pad_bbox_scale: 2.7`).
7. Update all downstream workspace consumers (`daemon`, `enrollment-cli`, and test suites) to preserve workspace-wide build integrity.

---

## 2. Multi-Agent TDD Cycle

### 2.1 Phase 1 — Architect Design
- **`crates/vision/src/crop.rs`**:
  - `expand_bbox_for_pad(bbox: &BoundingBox, scale: f32, img_w: u32, img_h: u32) -> BoundingBox`: Computes center-expanded box and clamps coordinates to `[0, img_w]` and `[0, img_h]`.
  - `crop_and_resize(rgb: &[u8], img_w: u32, img_h: u32, bbox: &BoundingBox, target_w: u32, target_h: u32) -> Result<Vec<u8>, VisionError>`: Bilinear interpolation crop-and-resize with zero padding for out-of-bounds regions.
- **`crates/vision/src/error.rs`**:
  - Added `VisionError::MissingLandmarks` variant to fail closed if a detection candidate lacks 5-point landmarks.
- **`crates/vision/src/pipeline.rs`**:
  - Removed `landmarks: Arc<dyn LandmarkDetector>` field and constructor parameter.
  - Added `pad_target_width: u32` (80), `pad_target_height: u32` (80), `pad_bbox_scale: f32` (2.7) to `VisionPipelineConfig`.
  - Restructured `process_frame()` to extract landmarks from detection, crop 80×80 expanded context for PAD, evaluate PAD, and only proceed to 112×112 alignment and embedding extraction if PAD passes.
- **`crates/inference-ort/src/mock.rs`**:
  - Updated `MockFaceDetector::new_centered_face()` to generate canonical 5-point landmarks within detections.

### 2.2 Phase 1.5 — Plan Evaluation
- The Plan Evaluator Sub-Agent audited the implementation plan against `AI/ARCHITECTURE.md` across the 6 architectural pillars, issuing `AI/plan_evaluator_report.md` with explicit `VALIDATION_VERDICT: APPROVED`.

### 2.3 Phase 2 — Tester Contracts (TDD Red Phase)
- Authored contractual unit and invariant tests in `crates/vision/tests/pipeline_tests.rs`:
  - `test_expand_bbox_centered`: Validates center preservation and 2.7× dimension scaling.
  - `test_expand_bbox_clamped_to_image`: Validates boundary clamping against negative coordinates and frame dimensions.
  - `test_pipeline_constructs_with_three_backends`: Verifies constructor signature taking (detector, pad, extractor, config).
  - `test_pipeline_extracts_landmarks_from_detection`: Asserts that output landmarks match detection landmarks.
  - `test_pipeline_fails_when_detection_lacks_landmarks`: Asserts fail-closed `VisionError::MissingLandmarks`.
  - `test_pipeline_pad_receives_expanded_crop`: Uses `SpyPadDetector` to verify receiving 80×80 crop (19,200 bytes).
  - `test_pipeline_embedding_receives_aligned_crop`: Uses `SpyExtractor` to verify receiving 112×112 crop (37,632 bytes).
  - `test_pipeline_config_defaults_3_model`: Asserts default values for PAD width/height/scale and embedding width/height.
- Ran `cargo check --test pipeline_tests` and verified that tests failed as expected prior to production code changes (Red Phase confirmed).

### 2.4 Phase 3 — Security & Panic Safety Audit
- Verified `#![forbid(unsafe_code)]` in `crates/vision`.
- Confirmed zero `unwrap()` or `expect()` in production library code.
- Confirmed memory zeroization: intermediate `pad_crop` wrapped in `Zeroizing`, and `PipelineOutput` and `VerificationOutcome` implement deterministic zeroization on drop.
- Verified checked arithmetic on buffer dimension calculations.

### 2.5 Phase 4 — Developer Implementation (Green Phase)
- Implemented `crates/vision/src/crop.rs`, `crates/vision/src/error.rs`, `crates/vision/src/pipeline.rs`, and `crates/vision/src/lib.rs`.
- Conducted workspace-wide cross-crate refactoring:
  - Updated `crates/daemon/src/pipeline.rs`, `crates/daemon/tests/pipeline_init_tests.rs`, and `crates/daemon/tests/pipeline_integration_tests.rs`.
  - Updated `crates/enrollment-cli/src/service.rs` and all 5 test files in `crates/enrollment-cli/tests/`.
  - Updated `crates/vision/tests/pad_tests.rs`, `zeroize_tests.rs`, and `bench_tests.rs`.
- Verified all workspace tests pass cleanly: 13 passed in `pipeline_tests`, 6 in `pad_tests`, 3 in `zeroize_tests`, 1 in `bench_tests`, 4 in `align_tests`, 12 in `color_tests`, 7 in `matcher_tests`.

### 2.6 Phase 5 — Candid Reviewer Audit
- Executed cold diff review against `origin/main`.
- Authored `AI/candid_review_report.md` with explicit `VERDICT: APPROVED`.

---

## 3. Verification Evidence

| ID | Criterion | Evidence | Status |
|---|---|---|---|
| NGM11 | `VisionPipeline` constructs with 3 backends | `pipeline_tests::test_pipeline_constructs_with_three_backends` | ✅ Verified |
| NGM12 | Pipeline extracts landmarks from `FaceDetection` | `pipeline_tests::test_pipeline_extracts_landmarks_from_detection`, `test_pipeline_fails_when_detection_lacks_landmarks` | ✅ Verified |
| NGM13 | PAD receives 80×80 expanded crop; embedding receives 112×112 aligned crop | `pipeline_tests::test_pipeline_pad_receives_expanded_crop`, `test_pipeline_embedding_receives_aligned_crop`, `test_expand_bbox_centered`, `test_expand_bbox_clamped_to_image` | ✅ Verified |
