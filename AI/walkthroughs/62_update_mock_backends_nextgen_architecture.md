# Walkthrough 62: Update Mock Backends for Next-Gen Architecture

## Context & Objectives

Following the implementation of the 3-model neural pipeline (SCRFD 500M KPS unified detector in Issue #37, removal of the standalone landmark detector in Issue #38, 512D ArcFace w600k embedding extractor in Issue #39, MiniFASNetV2 PAD detector in Issue #40, 3-model pipeline restructuring in Issue #41, letterbox padding in Issue #42, and model registry IDs in Issue #43), the deterministic mock backends and unit/integration test suites across the workspace needed synchronization to reflect the new architecture.

**Issue #44 (GitHub #110)** delivers this synchronization:
- **`#44.1`**: Update `MockFaceDetector` to embed 5-point facial landmarks (`FaceLandmarks`) directly in `FaceDetection`. Specifically, `new_centered_face()` generates both a bounding box and canonical landmarks scaled to the box via a reusable helper `canonical_landmarks_for_box()`.
- **`#44.2`**: Standardize `MockEmbeddingExtractor` across all test files and CLI mock pipelines on 512 dimensions (`DEFAULT_DIM = 512`), eliminating remaining legacy 128D usages.
- **`#44.3`**: Verify `MockPadDetector` compatibility (output shape `[1, 3]` returning `PadResult` directly).
- **`#44.4`**: Purge obsolete `MockLandmarkDetector` instances from test setups (`pad_tests.rs`, `zeroize_tests.rs`), ensuring all pipeline tests construct cleanly with 3 backends (`detector`, `pad`, `extractor`).

---

## 1. Phase 1 — Architecture & Type Specifications

1. **`crates/inference-ort/src/mock.rs`**:
   - `MockFaceDetector::canonical_landmarks_for_box(face_box: &BoundingBox) -> FaceLandmarks`: Computes standard ArcFace / InsightFace 112x112 canonical reference points scaled linearly to any given face bounding box.
   - `MockFaceDetector::new_centered_face(width: u32, height: u32, score: f32) -> Self`: Constructs a centered bounding box and calls `canonical_landmarks_for_box` to populate `FaceDetection::with_landmarks(box_, score, landmarks)`.
2. **Phase 1.5 — Plan Evaluator Sub-Agent**:
   - Evaluated the plan across the 6 architectural pillars in `AI/plan_evaluator_report.md` with explicit verdict `VALIDATION_VERDICT: APPROVED`.

---

## 2. Phase 2 — Tester Contracts (TDD Red Phase)

Contractual tests were authored in `crates/inference-ort/tests/detector_tests.rs` before modifying production mock code:
- `test_mock_detector_returns_landmarks`: Asserts that `MockFaceDetector::new_centered_face` returns detections with embedded landmarks matching canonical coordinates scaled to the bounding box.
- `test_mock_face_detector_canonical_landmarks_for_box`: Validates `canonical_landmarks_for_box` on a 112x112 box against known ArcFace reference values.

### Initial Red Phase Execution
Running `cargo test -p soos-inference-ort --test detector_tests` failed as expected:
```text
error[E0599]: no function or associated item named `canonical_landmarks_for_box` found for struct `MockFaceDetector` in the current scope
   --> crates/inference-ort/tests/detector_tests.rs:248:32
    |
248 |     let lm = MockFaceDetector::canonical_landmarks_for_box(&box_);
    |                                ^^^^^^^^^^^^^^^^^^^^^^^^^^^ function or associated item not found in `MockFaceDetector`
```

---

## 3. Phase 3 — Security & Panic Safety Audit

- `#![forbid(unsafe_code)]` remains strictly enforced in `crates/inference-ort` and `crates/vision`.
- Pure arithmetic in landmark coordinate scaling without division by zero (`112.0 != 0.0`).
- Zero `unwrap()` or `expect()` in production library code.
- Zero sensitive data logging or exposure.

---

## 4. Phase 4 — Developer Implementation (Green Phase)

1. **`crates/inference-ort/src/mock.rs`**:
   - Implemented `canonical_landmarks_for_box(face_box: &BoundingBox) -> FaceLandmarks`.
   - Updated `new_centered_face` to use `canonical_landmarks_for_box`.
2. **Dimensionality Standardization (512D)**:
   - `crates/enrollment-cli/src/service.rs`: Updated mock pipeline extractor to `MockEmbeddingExtractor::new(512)`.
   - `crates/daemon/tests/pipeline_init_tests.rs`: Updated extractor to 512D.
   - `crates/daemon/tests/pipeline_integration_tests.rs`: Updated extractor to 512D.
   - `crates/enrollment-cli/tests/delete_tests.rs`: Updated extractor and template vectors to 512D.
   - `crates/enrollment-cli/tests/enroll_tests.rs`: Updated extractor and assertion contracts to 512D.
   - `crates/enrollment-cli/tests/list_tests.rs`: Updated extractor, template vectors, and assertion contracts to 512D.
   - `crates/enrollment-cli/tests/root_check_tests.rs`: Updated extractor to 512D.
   - `crates/enrollment-cli/tests/verify_tests.rs`: Updated standard and test extractors to 512D.
   - `crates/vision/tests/bench_tests.rs`: Updated extractor to 512D.
   - `crates/vision/tests/pad_tests.rs`: Updated `SpyEmbeddingExtractor` to 512D and assertions to 512D.
   - `crates/vision/tests/pipeline_tests.rs`: Updated `setup_pipeline` and all mock extractors and orthogonal vectors to 512D.
   - `crates/vision/tests/zeroize_tests.rs`: Updated extractor and zeroization outcome vector to 512D.
3. **Purged `MockLandmarkDetector` from Test Pipelines**:
   - `crates/vision/tests/pad_tests.rs`: Removed `MockLandmarkDetector` import, return tuple element, and instantiation from `create_test_pipeline`.
   - `crates/vision/tests/zeroize_tests.rs`: Removed `MockLandmarkDetector` import and instantiation; constructed detection with landmarks using `MockFaceDetector::canonical_landmarks_for_box`.

---

## 5. Phase 5 — Candid Reviewer Audit

The independent candid reviewer audited the diff across the 5 pillars, authoring `AI/candid_review_report.md` with:
```text
VERDICT: APPROVED
```

---

## 6. Phase 6 — Traceability & Verification

- **Backlog**: Sub-issues `#44.1`, `#44.2`, `#44.3`, `#44.4` marked complete in `AI/BACKLOG.md`.
- **Verification Matrix**: Criterion `NGM16` marked `✅ Verified` in `AI/VERIFICATION_MATRIX.md` and `AI/BACKLOG.md`.
- **Quality Gates**:
  - `cargo fmt -- --check`: Clean pass.
  - `cargo clippy --all-targets --all-features -- -D warnings`: Clean pass (0 warnings).
  - `cargo test --workspace`: Clean pass across all 200+ unit, integration, and invariant tests.
