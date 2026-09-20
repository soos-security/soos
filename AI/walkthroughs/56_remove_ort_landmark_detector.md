# Walkthrough 56 — Remove OrtLandmarkDetector Absorbed by SCRFD

> **Date**: 2026-09-20  
> **Issue**: Issue #38 (`refactor/remove-ort-landmark-detector`, GitHub #104)  
> **Verification Matrix**: `NGM6`  
> **Scope**: `crates/inference-ort/src/landmarks.rs`, `crates/inference-ort/src/lib.rs`, `crates/inference-ort/src/mock.rs`, `crates/inference-ort/tests/landmarks_tests.rs`, `crates/inference-ort/tests/zeroize_tests.rs`, `crates/daemon/src/pipeline.rs`, `crates/enrollment-cli/src/service.rs`, `Docs/INFERENCE_ORT_CRATE.md`

---

## 1. Problem Statement & Motivation

With the introduction of SCRFD 500M KPS (Issue #37), 5-point facial keypoints (left eye, right eye, nose tip, left mouth corner, right mouth corner) are regressed simultaneously with face bounding boxes in a single forward pass.

As a result:
- The standalone `OrtLandmarkDetector` and its separate ONNX session (`landmark_5point.onnx`) became redundant.
- Running a separate landmark pass imposed unnecessary execution latency (~20ms) and memory overhead (~1.5MB ONNX session weights).
- However, core domain geometric types (`FaceLandmarks`, `Point2f`) and the `LandmarkDetector` trait abstraction are vital across the codebase and for simulation/testing (`MockLandmarkDetector`).

Issue #38 removes `OrtLandmarkDetector` while preserving all domain abstractions and reducing runtime model loading from 4 sessions to 3 sessions.

---

## 2. Multi-Agent TDD Cycle

### 2.1 Phase 1 — Architect Design
- **Preserved in `crates/inference-ort/src/landmarks.rs`**:
  - `Point2f`: 2D floating-point coordinates and `distance_to()` Euclidean distance calculation.
  - `FaceLandmarks`: 5-point landmark geometry with `as_array()`, `from_array()`, `eye_distance()`, and `roll_angle_rad()`.
  - `LandmarkDetector` trait: Trait interface for landmark estimation backends.
- **Removed from `landmarks.rs`**:
  - `OrtLandmarkDetector` struct and its implementation (`new`, `prepare_input`, `impl LandmarkDetector for OrtLandmarkDetector`).
  - Unused imports (`ort::session::Session`, `std::sync::{Arc, Mutex}`, `zeroize::{Zeroize, Zeroizing}`).
- **ModelRegistry Session Loading (#38.3)**:
  - Startup session initialization in `soos-daemon` and `soos-enrollment-cli` eliminates `landmark_5point` session loading.
  - Exactly 3 models are loaded at startup (`ultraface_slim_320` / `scrfd_500m_kps`, `minifasnet_pad` / `minifasnet_v2_pad`, `mobilefacenet_arcface` / `arcface_w600k_mbf`).
  - `VisionPipeline` consumes `MockLandmarkDetector::new_canonical()` as a transitional adapter until Issue #41 restructures `VisionPipeline::new()`.

### 2.2 Phase 1.5 — Plan Evaluation
- The Plan Evaluator Sub-Agent audited the design against `AI/ARCHITECTURE.md`, ADRs, and security invariants, issuing `VALIDATION_VERDICT: APPROVED` in `AI/plan_evaluator_report.md`.

### 2.3 Phase 2 — Tester Contracts (TDD Red Phase)
- Authored contract test suite `crates/inference-ort/tests/landmarks_tests.rs`:
  - `test_point2f_geometry_and_distance`: Verifies Euclidean distance calculations.
  - `test_face_landmarks_array_conversion_and_geometry`: Verifies canonical array order, roundtrip reconstruction, eye distance, and roll angles.
  - `test_landmark_detector_trait_mock_dispatch`: Verifies trait object dispatch through `Arc<dyn LandmarkDetector>`.
- Updated `crates/inference-ort/tests/zeroize_tests.rs`:
  - Replaced deleted `OrtLandmarkDetector::prepare_input` test with `OrtScrfdDetector::prepare_input` zeroization assertion, confirming that 640×640 float buffers are scrubbed on clear and drop per invariant `VZF3`.
- Verified the Red Phase: removing `OrtLandmarkDetector` from `lib.rs` caused compilation in consumer crates (`soos-daemon`, `soos-enrollment-cli`) to fail with unresolved imports as expected.

### 2.4 Phase 3 — Security & Panic Safety Audit
- Verified `#![forbid(unsafe_code)]` in `crates/inference-ort`.
- Confirmed zero `unwrap()` or `expect()` in production pathways.
- Confirmed zero `println!` or `dbg!` macro usages.

### 2.5 Phase 4 — Developer Implementation (Green Phase)
- Cleaned up `crates/inference-ort/src/landmarks.rs` and `crates/inference-ort/src/lib.rs`.
- Implemented `Default` for `MockLandmarkDetector` in `crates/inference-ort/src/mock.rs`.
- Updated `crates/daemon/src/pipeline.rs` and `crates/enrollment-cli/src/service.rs` to load 3 sessions.
- Formatted via `cargo fmt` and validated zero Clippy warnings (`cargo clippy --all-targets --all-features -- -D warnings`).

### 2.6 Phase 5 — Candid Reviewer Audit
- Cold diff analysis conducted against `origin/main` across 5 pillars.
- Authored `AI/candid_review_report.md` with `VERDICT: APPROVED`.

---

## 3. Verification Evidence

### 3.1 Unit and Integration Tests
```bash
cargo test -p soos-inference-ort --test landmarks_tests
cargo test -p soos-inference-ort --test zeroize_tests
cargo test -p soos-daemon
cargo test -p soos-enrollment-cli
cargo test --all-targets
```
All tests passed with 0 failures across all 11 workspace crates.

### 3.2 Linter & Formatter Verification
```bash
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
```
Exit code 0, clean build with zero warnings.

---

## 4. Summary of Changes

| Component | File | Changes |
|---|---|---|
| `inference-ort` | `src/landmarks.rs` | Removed `OrtLandmarkDetector`, kept `FaceLandmarks`, `Point2f`, `LandmarkDetector` |
| `inference-ort` | `src/lib.rs` | Removed `OrtLandmarkDetector` from public exports |
| `inference-ort` | `src/mock.rs` | Added `impl Default for MockLandmarkDetector` |
| `inference-ort` | `tests/landmarks_tests.rs` | Added contractual tests for preserved domain types & trait dispatch |
| `inference-ort` | `tests/zeroize_tests.rs` | Replaced landmark zeroize check with `OrtScrfdDetector::prepare_input` |
| `daemon` | `src/pipeline.rs` | Removed `landmark_5point` session loading; uses `MockLandmarkDetector` adapter |
| `enrollment-cli` | `src/service.rs` | Removed `landmark_5point` session loading; uses `MockLandmarkDetector` adapter |
| `scripts` | `scripts/sync_issue.py` | Registered Issue #38 -> GitHub #104 and topic branch mapping |
| `docs` | `Docs/INFERENCE_ORT_CRATE.md` | Documented removal of `OrtLandmarkDetector` and next-gen pipeline layout |
| `matrix` | `AI/VERIFICATION_MATRIX.md` | Added criterion `NGM6` verified status |
