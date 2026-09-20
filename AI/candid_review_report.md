# Candid Review Report

- **Date**: 2026-09-20
- **Target Branch / Commit**: `refactor/remove-ort-landmark-detector`
- **Audited Files**:
  - `crates/inference-ort/src/landmarks.rs`
  - `crates/inference-ort/src/lib.rs`
  - `crates/inference-ort/src/mock.rs`
  - `crates/inference-ort/tests/landmarks_tests.rs`
  - `crates/inference-ort/tests/zeroize_tests.rs`
  - `crates/daemon/src/pipeline.rs`
  - `crates/enrollment-cli/src/service.rs`
  - `Docs/INFERENCE_ORT_CRATE.md`
  - `scripts/sync_issue.py`

## 1. Executive Summary

This pull request addresses Issue #38 (`[inference-ort] Remove OrtLandmarkDetector (absorbed by SCRFD)`). SCRFD outputs 5-point facial landmarks directly as part of face detection, rendering the separate `OrtLandmarkDetector` and its ONNX session redundant. The implementation cleanly removes `OrtLandmarkDetector` from `soos-inference-ort` while strictly preserving core domain types (`FaceLandmarks`, `Point2f`) and the `LandmarkDetector` trait abstraction. Session loading in `soos-daemon` and `soos-enrollment-cli` is reduced from 4 sessions to 3 sessions at startup, matching the v2.0.0 manifest. `MockLandmarkDetector` is retained as a transitional adapter for `VisionPipeline` until Issue #41 restructures the pipeline constructor. Comprehensive contract and memory zeroization tests have been added and validated.

## 2. Deep Reasoning Audit

### Logic & Architecture
- **Pass**: State transitions, domain abstractions, and type preservation are verified. `FaceLandmarks` and `Point2f` retain all mathematical methods (`distance_to`, `eye_distance`, `roll_angle_rad`, `as_array`, `from_array`).
- **Pass**: Downstream consumers (`soos-daemon` and `soos-enrollment-cli`) eliminate `landmark_5point` ONNX session loading, correctly loading exactly 3 models at startup.

### PAM Concurrency & Deadlines
- **Pass**: The PAM module (`pam_soos.so`) is untouched. Zero Tokio or asynchronous runtimes are introduced.
- **Pass**: Removing the dedicated landmark ONNX inference pass saves ~20ms in the vision pipeline, contributing directly to the strict <150ms verification latency budget. Zero output stream pollution (`println!`, `dbg!`).

### Panic Safety & Fallback
- **Pass**: Zero `unwrap()` or `expect()` in production code. Removing `OrtLandmarkDetector` removes session lock poisoning and dimension indexing paths from `landmarks.rs`.
- **Pass**: All session loading and pipeline component constructions safely propagate errors via `Result`.

### Test Integrity & Anti-Weakening
- **Pass**: No existing tests were weakened, bypassed, or deleted.
- **Pass**: Added `crates/inference-ort/tests/landmarks_tests.rs` validating geometric accuracy, array roundtrips, and trait mock dispatch.
- **Pass**: `zeroize_tests.rs` was updated to test `OrtScrfdDetector::prepare_input` buffer scrubbing without weakening zeroization verification.

### Memory & Secret Bounds
- **Pass**: Eliminates intermediate 112×112 float allocations for landmark inference.
- **Pass**: Buffer zeroization on SCRFD input tensors (`640×640`) is verified on drop/clear per invariant `VZF3`. Zero secrets or embeddings leaked.

## 3. Detailed Findings & Action Items
- None. All quality checks, Clippy lints, and formatting requirements are satisfied.

## 4. Final Verdict
**VERDICT: APPROVED**
