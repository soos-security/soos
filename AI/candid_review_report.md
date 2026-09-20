# Candid Review Report

- **Date**: 2026-09-20
- **Target Branch / Commit**: `refactor/model-ids-nextgen`
- **Audited Files**:
  - `crates/daemon/src/pipeline.rs`
  - `crates/daemon/tests/model_deployment_tests.rs`
  - `crates/enrollment-cli/src/lib.rs`
  - `crates/enrollment-cli/src/service.rs`
  - `crates/enrollment-cli/tests/model_id_tests.rs`
  - `Docs/ENROLLMENT_CLI.md`
  - `scripts/sync_issue.py`

## 1. Executive Summary

This pull request completes Issue #43 / GitHub #109 by migrating all model registry identifiers and ONNX Runtime session initializations across `soos-daemon` and `soos-enrollment-cli` to the 3-model next-generation architecture (`scrfd_500m_kps`, `minifasnet_v2_pad`, `arcface_w600k_mbf`). The legacy landmark model (`landmark_5point`) session and constants have been completely excised, and `OrtScrfdDetector` is properly instantiated with error propagation via `?`. All contractual test assertions and documentation have been synchronized with `models/manifest.toml` v2.0.0.

## 2. Deep Reasoning Audit

### Logic & Architecture
- **Pass**:
  - `crates/daemon/src/pipeline.rs` loads exactly 3 sessions: `"scrfd_500m_kps"`, `"minifasnet_v2_pad"`, and `"arcface_w600k_mbf"`.
  - Detector initialization invokes `soos_inference_ort::OrtScrfdDetector::new(...)` which performs multi-stride 9-tensor validation and passes 5-point landmarks directly to `VisionPipeline`.
  - `crates/enrollment-cli/src/service.rs` updates `MODEL_ID_FACE_DETECTOR`, `MODEL_ID_PAD`, and `MODEL_ID_EMBEDDING`, eliminates `MODEL_ID_LANDMARKS`, and updates `REQUIRED_MODEL_IDS` to length 3.
  - Lazy initialization in `build_store_only` remains completely isolated from camera and model loading.

### PAM Concurrency & Deadlines
- **Pass**:
  - Changes are strictly isolated to the privileged daemon and enrollment CLI binaries; the synchronous PAM module (`crates/pam`) is untouched.
  - Verification latency is reduced due to eliminating the redundant landmark localization inference stage.
  - Zero stdout/stderr stream pollution or unapproved asynchronous runtimes.

### Panic Safety & Fallback
- **Pass**:
  - `OrtScrfdDetector::new` produces `Result<Self, InferenceError>`, safely mapped via `?` operator into `DaemonError` and `EnrollmentCliError`.
  - Zero instances of `unwrap()`, `expect()`, `panic!()`, or unhandled stubs in production paths.
  - Missing or tampered models fail closed during cryptographic attestation (`registry.verify_integrity()?`).

### Test Integrity & Anti-Weakening
- **Pass**:
  - Pre-existing tests were not weakened or bypassed.
  - Contractual test `test_enrollment_cli_model_ids_match_manifest` was updated to assert the 3-model v2.0.0 manifest specification.
  - Added negative test `test_enrollment_cli_legacy_model_ids_absent` to actively prevent regression to legacy model IDs.
  - `model_deployment_tests.rs` tests missing and tampered next-gen models against v2.0.0 SHA-256 digests.

### Memory & Secret Bounds
- **Pass**:
  - `OrtScrfdDetector` maintains input buffer zeroization (`Zeroizing<Vec<f32>>`) post-inference.
  - Zero sensitive credentials, passwords, or raw biometric embeddings are exposed or logged.
  - `#![forbid(unsafe_code)]` remains strictly enforced across all business and computational crates.

## 3. Detailed Findings & Action Items
- None. All architectural invariants, security constraints, and workspace conventions are fully respected.

## 4. Final Verdict
**VERDICT: APPROVED**
