# Walkthrough 21 — `inference-ort` Crate: ONNX Runtime Wrapper & Model Attestation

> **Date**: 2026-09-13  
> **Target**: Issue #6 (`inference-ort` Crate — ONNX Runtime Wrapper / GitHub Issue #13)  
> **Branch**: `feat/inference-ort`  
> **Verification Matrix**: Global Invariant (Manifest SHA-256 Attestation), V2 (L2 Normalization norm ≈ 1.0)  

---

## 1. Overview & Objectives

Issue #6 introduces `crates/inference-ort/` (`soos-inference-ort`), providing the machine learning inference foundation for `soos-daemon`. Authentication requires local deep neural networks running on CPU with minimal latency.

### Architectural Invariants Enforced
1. **Daemon-Isolated Inference**: Only the root daemon performs AI inference; the PAM module (`pam_soos.so`) never links or invokes ONNX Runtime.
2. **Cryptographic Attestation (Global Invariant)**: Every model file is cataloged in `models/manifest.toml` with expected SHA-256 checksums, licenses, and tensor shapes. `ModelRegistry` verifies the cryptographic hash of each model file before creating an ORT session, rejecting tampered or corrupt weights fail-closed.
3. **Absolute Prohibition of OpenCV**: Face detection preprocessing, 5-point landmark mapping, and feature extraction tensor pipelines are implemented in pure, safe Rust without OpenCV dependencies.
4. **Deterministic Rust NMS**: Non-Maximum Suppression (NMS) sorts candidates primarily by confidence score with secondary coordinate tie-breaking, ensuring 100% deterministic face selection.
5. **L2 Normalization (Criterion V2)**: Facial feature embeddings are strictly L2-normalized ($||v||_2 \approx 1.0$ within $10^{-5}$ tolerance), guaranteeing valid cosine similarity distance metrics.
6. **Hardware-Free Simulation**: Provides `MockFaceDetector`, `MockLandmarkDetector`, and `MockEmbeddingExtractor` for reliable, download-free automated testing across headless CI/CD environments.

---

## 2. Multi-Agent Implementation Cycle

### Phase 1 — Architect Sub-Agent
- Scaffolds `crates/inference-ort/` with `Cargo.toml`, configured with `ort = "2.0.0-rc.13"`, `ndarray = "0.17"`, `sha2 = "0.10"`, `toml = "0.8"`, `thiserror = "2"`, and `serde = "1"`.
- Specifies core domain abstractions:
  - `ModelManifest` & `ModelMetadata`: TOML parsing and SHA-256 file hashing.
  - `BoundingBox` & `FaceDetection`: 2D geometry, IoU, and bounding box clamping.
  - `nms`: deterministic pure-Rust Non-Maximum Suppression algorithm.
  - `FaceLandmarks` & `Point2f`: canonical 5-point facial landmark layout and roll angle calculation.
  - `BiometricEmbedding`: high-dimensional vector container with Euclidean L2-normalization and cosine similarity.
  - `ModelRegistry`: session caching and cryptographic integrity verification.
  - `FaceDetector`, `LandmarkDetector`, and `EmbeddingExtractor` traits.

### Phase 1.5 — Plan Evaluator Sub-Agent
- Conducted exhaustive compliance audit across the 6 architectural pillars.
- Authored `AI/plan_evaluations/03_inference_ort_plan_evaluation.md` with **`VALIDATION_VERDICT: APPROVED`**.

### Phase 2 — Tester Sub-Agent (TDD Red Phase)
- Authored 26 contractual unit, integration, and property tests:
  - `tests/manifest_tests.rs`: workspace manifest parsing, SHA-256 computation against known digests, tamper detection, missing model handling, invalid TOML parsing.
  - `tests/detector_tests.rs`: BoundingBox geometry and IoU, clamping, NMS suppression of overlapping boxes, preservation of disjoint faces, deterministic tie-breaking, empty input handling, and mock detector validation.
  - `tests/landmark_tests.rs`: Point2f Euclidean distance, 5-point landmark array round-tripping, roll angle calculation, and mock landmark detector scaling.
  - `tests/embedding_tests.rs`: L2 norm computation and normalization (Criterion `V2`), rejection of zero-vectors, cosine similarity properties (identity = 1.0, orthogonal = 0.0, opposite = -1.0, dimension mismatch), and mock extractor normalization.
  - `tests/registry_tests.rs`: registry initialization with workspace manifest, verification failure on missing files, and path resolution.
  - `tests/proptest_suite.rs`: property-based tests verifying IoU symmetry and bounds in $[0.0, 1.0]$, NMS invariants, embedding normalization invariant ($||v||_2 \approx 1.0$), and cosine similarity symmetry.

### Phase 3 — Auditor Sub-Agent
- Confirmed `#![forbid(unsafe_code)]` at the crate root.
- Verified zero `unwrap()` or `expect()` in production code.
- Confirmed zero stdout/stderr prints (`println!`, `eprintln!`, `dbg!`).
- Confirmed absence of forbidden crates (`opencv`, `nokhwa`).

### Phase 4 — Developer Sub-Agent (TDD Green Phase)
- Implemented `ModelManifest` and `ModelRegistry` with cryptographic SHA-256 verification.
- Implemented `OrtFaceDetector` with UltraFace Slim 320 priors generation and output decoding.
- Implemented `OrtLandmarkDetector` with 5-point regression mapping.
- Implemented `OrtEmbeddingExtractor` with MobileFaceNet ArcFace tensor normalization and unit L2 scaling.
- Implemented `MockFaceDetector`, `MockLandmarkDetector`, and `MockEmbeddingExtractor`.
- All 26 crate tests and 130+ workspace-wide tests passed cleanly.
- Clippy verified clean with `-D warnings`.
- Code formatted with `cargo fmt`.

### Phase 5 — Candid Reviewer Sub-Agent
- Executed cold diff review against `origin/main` via `./scripts/candid_review.sh`.
- Authored `AI/candid_review_report.md` with **`VERDICT: APPROVED`**.

---

## 3. Verification Evidence

| Matrix ID | Criterion | Test Evidence | Status |
|---|---|---|---|
| **Global** | Each ONNX model is attested by manifest + SHA-256 checksum | `manifest_tests::test_parse_workspace_manifest_file`, `manifest_tests::test_verify_model_checksum_success_and_tamper_detection`, `registry_tests::test_registry_verify_integrity_missing_files_fails_closed` | ☑ Validated |
| **V2** | L2-normalized embeddings (norm ≈ 1.0) | `embedding_tests::test_l2_norm_and_normalization_criterion_v2`, `embedding_tests::test_mock_embedding_extractor_criterion_v2`, `proptest_suite::prop_embedding_normalization_criterion_v2` | ☑ Validated |
| **Invariant** | `#![forbid(unsafe_code)]` declared | Invariant test & compile-time crate declaration | ☑ Validated |
| **Invariant** | Zero OpenCV across workspace | `tests/invariants::test_no_opencv_in_any_cargo_toml` | ☑ Validated |

---

## 4. Deliverables
- Crate: `crates/inference-ort/` (`Cargo.toml`, `src/lib.rs`, `src/error.rs`, `src/manifest.rs`, `src/detector.rs`, `src/landmarks.rs`, `src/embedding.rs`, `src/registry.rs`, `src/mock.rs`)
- Model Manifest: `models/manifest.toml`
- Test Suites: `crates/inference-ort/tests/` (`manifest_tests.rs`, `detector_tests.rs`, `landmark_tests.rs`, `embedding_tests.rs`, `registry_tests.rs`, `proptest_suite.rs`)
- Documentation: `Docs/INFERENCE_ORT_CRATE.md`
- Plan Evaluation: `AI/plan_evaluations/03_inference_ort_plan_evaluation.md`
- Walkthrough: `AI/walkthroughs/21_inference_ort_onnx_runtime_wrapper.md`
- Candid Review: `AI/candid_review_report.md`
- Verification Matrix: updated `AI/VERIFICATION_MATRIX.md`
