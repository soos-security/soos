# Plan Evaluation Report — Issue #37: [inference-ort] SCRFD Face Detector

- **Target Issue**: Issue #37 (`[inference-ort] Implement SCRFD face detector with multi-stride output parsing`)
- **Evaluator**: Plan Evaluator Sub-Agent (`plan-evaluator`)
- **Date**: 2026-09-20
- **Evaluation Status**: VALIDATION_VERDICT: APPROVED

---

## 1. Architectural Alignment & Threat Model
- The proposed implementation introduces `OrtScrfdDetector` implementing the `FaceDetector` trait inside `crates/inference-ort`.
- SCRFD 500M KPS unifies face bounding box detection and 5-point facial landmark estimation into a single forward pass, aligning with `AI/walkthroughs/53_nextgen_model_migration_architecture.md` and ADR `[2026-09-20] SCRFD Detection Architecture`.
- The design maintains strict separation of concerns: inference is isolated within `crates/inference-ort`, consuming pure RGB frame slices and returning geometric detections (`FaceDetection`).
- Retains compatibility for legacy consumers while preparing for subsequent pipeline modernization (Issues #38, #41, #43).

## 2. PAM Real-Time Latency & Concurrency
- Eliminates the separate landmark detection pass, reducing vision pipeline latency by ~20ms.
- Pure Rust letterbox resize and coordinate un-projection are direct numerical operations (<0.2ms total overhead), well within the allocated 2ms letterbox latency budget.
- SCRFD forward pass is estimated at ~3.6–5ms on CPU at 640×640 resolution, safely within the total 150ms verification budget.
- Zero Tokio or asynchronous runtimes introduced; synchronous blocking execution model preserved.
- Zero output stream pollution (`println!`, `eprintln!`, `dbg!`).

## 3. Panic Safety & Fail-Closed Behavior
- Zero `unwrap()` or `expect()` in library production code (`detector.rs`).
- All bounds checks (`rgb.len() == expected_len`, grid coordinate indexing, output tensor counts) return typed `InferenceError` variants (`InvalidBufferSize`, `TensorError`, `DetectionFailed`).
- Startup validation in `OrtScrfdDetector::new()` validates output tensor count (exactly 9 outputs) and structural shapes at initialization time, preventing runtime shape panics.
- Errors fail closed, preventing unauthenticated access.

## 4. Dependency Isolation & Banned Crates
- Strictly respects prohibitions: zero `opencv` or `nokhwa`.
- Letterboxing, BGR conversion, and coordinate un-projection are implemented purely in safe Rust.
- `#![forbid(unsafe_code)]` enforced in `crates/inference-ort`.

## 5. Memory & Secret Bounds
- Input float tensors are enclosed in `zeroize::Zeroizing<Vec<f32>>` containers and explicitly scrubbed post-inference, satisfying invariant `VZF3`.
- Sensitive frame data and biometric coordinates are bounded and never logged.

## 6. Test Integrity & TDD Contracts
- Comprehensive contractual unit, synthetic decoding, and property tests specified for Phase 2 before production implementation:
  - `test_letterbox_preserves_aspect_ratio`
  - `test_letterbox_unproject_roundtrip`
  - `test_prepare_input_bgr_channel_ordering`
  - `test_scrfd_decode_stride8_known_output`
  - `test_scrfd_decode_all_strides`
  - `test_unproject_coordinates_match_original_image`
  - `test_face_detection_carries_landmarks`
  - `test_scrfd_rejects_invalid_output_count`
- Zero test weakening of existing test suites; all prior workspace tests preserved.
- Downstream consumer fixtures updated to satisfy the Cross-Crate Refactoring Invariant.

---

## Conclusion & Verdict

The implementation plan is fully compliant with all architectural invariants, security guidelines, and ADRs.

VALIDATION_VERDICT: APPROVED
