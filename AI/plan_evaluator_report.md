# Plan Evaluation Report — Issue #38: [inference-ort] Remove OrtLandmarkDetector (absorbed by SCRFD)

- **Target Issue**: Issue #38 (`[inference-ort] Remove OrtLandmarkDetector (absorbed by SCRFD) #104`)
- **Evaluator**: Plan Evaluator Sub-Agent (`plan-evaluator`)
- **Date**: 2026-09-20
- **Evaluation Status**: VALIDATION_VERDICT: APPROVED

---

## 1. Architectural Alignment & Threat Model
- The proposed implementation removes the redundant `OrtLandmarkDetector` struct and its `impl LandmarkDetector` from `crates/inference-ort/src/landmarks.rs`, reflecting the absorption of 5-point facial landmarks into `OrtScrfdDetector` (Issue #37).
- Core domain types (`FaceLandmarks`, `Point2f`) and the `LandmarkDetector` trait abstraction are strictly preserved, aligning with `AI/walkthroughs/53_nextgen_model_migration_architecture.md` and ADR `[2026-09-20] Next-Generation AI Models`.
- Startup initialization in `soos-daemon` and `enrollment-cli` eliminates `landmark_5point` ONNX session loading, reducing startup models from 4 to 3 (`scrfd_500m_kps`, `minifasnet_v2_pad`, `arcface_w600k_mbf`) and matching the v2.0.0 manifest.
- Downstream integration with `VisionPipeline` is cleanly sustained via `MockLandmarkDetector` as a transitional adapter until Issue #41 restructures `VisionPipeline::new()`.

## 2. PAM Real-Time Latency & Concurrency
- Eliminating the standalone landmark inference forward pass saves ~20ms in the vision pipeline, directly contributing to the <150ms verification latency budget.
- Memory footprint is reduced by unloading the separate landmark ONNX session (~1.5MB RAM freed).
- Zero Tokio runtimes or asynchronous primitives introduced.
- Zero output stream pollution (`println!`, `eprintln!`, `dbg!`).

## 3. Panic Safety & Fail-Closed Behavior
- Zero `unwrap()` or `expect()` in production code.
- Removing `OrtLandmarkDetector` from `landmarks.rs` leaves only pure geometric data structures (`Point2f`, `FaceLandmarks`) and trait definitions, eliminating potential runtime session lock poisoning or tensor shape error pathways from this module.
- All session loading and startup logic in `daemon` and `enrollment-cli` propagate errors via standard `Result<T, E>`.

## 4. Dependency Isolation & Banned Crates
- Zero `opencv` or `nokhwa`.
- `#![forbid(unsafe_code)]` remains strictly enforced in `crates/inference-ort`.
- Unused session and synchronization imports (`ort::session::Session`, `std::sync::{Arc, Mutex}`, `zeroize`) are removed from `landmarks.rs`.

## 5. Memory & Secret Bounds
- Eliminates 112×112 intermediate float tensors previously allocated by `OrtLandmarkDetector::prepare_input`.
- Zeroization test suite (`crates/inference-ort/tests/zeroize_tests.rs`) is updated to assert deterministic memory scrubbing on `OrtScrfdDetector::prepare_input` (640×640 float buffer), maintaining invariant `VZF3`.

## 6. Test Integrity & TDD Contracts
- Explicit contractual tests authoring for Phase 2:
  - `test_point2f_geometry_and_distance`
  - `test_face_landmarks_array_conversion_and_angles`
  - `test_landmark_detector_trait_mock_dispatch`
  - `test_scrfd_input_buffer_zeroized` in `zeroize_tests.rs`
- Zero test weakening of existing test suites across the workspace.
- Preserves all mock implementations (`MockLandmarkDetector`) and existing vision pipeline test coverage.

---

## Conclusion & Verdict

The implementation plan satisfies all 6 architectural pillars, complies with ADRs and project invariants, and provides seamless transitional continuity across workspace crates.

VALIDATION_VERDICT: APPROVED
