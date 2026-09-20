# Walkthrough 61: Update Model Registry IDs for 3-Model Architecture

## Context & Objectives

Following the implementation of the 3-model neural pipeline components (SCRFD 500M KPS unified detector in Issue #37, landmark detector removal in Issue #38, 512D w600k embedding extractor in Issue #39, MiniFASNetV2 PAD detector in Issue #40, and pipeline restructuring in Issue #41), the higher-level service entry points (`soos-daemon` and `soos-enrollment-cli`) retained legacy model identifiers and session initialization routines.

**Issue #43 (GitHub #109)** unifies model session loading across the daemon and enrollment CLI with `models/manifest.toml` v2.0.0:
- Update model ID strings:
  - `"ultraface_slim_320"` → `"scrfd_500m_kps"`
  - `"minifasnet_pad"` → `"minifasnet_v2_pad"`
  - `"mobilefacenet_arcface"` → `"arcface_w600k_mbf"`
- Completely remove `"landmark_5point"` session loading and constants.
- Instantiate `OrtScrfdDetector` via `OrtScrfdDetector::new(session, conf, iou)?` with error propagation into `DaemonError` and `EnrollmentCliError`.
- Cleanly synchronize contractual test suites, documentation, and verification matrices.

---

## 1. Phase 1 — Architecture & Type Specifications

The architectural design establishes:
1. **Daemon Pipeline (`crates/daemon/src/pipeline.rs`)**:
   - Exactly 3 ORT sessions are loaded during startup: `scrfd_500m_kps`, `minifasnet_v2_pad`, and `arcface_w600k_mbf`.
   - Detector is instantiated as `soos_inference_ort::OrtScrfdDetector::new(det_session, config.vision.min_face_confidence, 0.45)?`.
   - Inferences error is propagated via `DaemonError::Inference`.

2. **Enrollment CLI Service (`crates/enrollment-cli/src/service.rs`)**:
   - `MODEL_ID_FACE_DETECTOR: &str = "scrfd_500m_kps"`
   - `MODEL_ID_PAD: &str = "minifasnet_v2_pad"`
   - `MODEL_ID_EMBEDDING: &str = "arcface_w600k_mbf"`
   - `MODEL_ID_LANDMARKS` is deleted entirely.
   - `REQUIRED_MODEL_IDS: [&str; 3] = [MODEL_ID_FACE_DETECTOR, MODEL_ID_PAD, MODEL_ID_EMBEDDING]`
   - `build_full_service` instantiates `OrtScrfdDetector::new(det_session, 0.70, 0.40)?`.
   - `build_store_only` preserves lazy initialization without model loading.

3. **Plan Evaluator Sub-Agent**:
   - Audited the implementation across the 6 architectural pillars in `AI/plan_evaluator_report.md` with `VALIDATION_VERDICT: APPROVED`.

---

## 2. Phase 2 — Tester Contracts (TDD Red Phase)

Contractual tests in `crates/enrollment-cli/tests/model_id_tests.rs` and `crates/daemon/tests/model_deployment_tests.rs` were updated to enforce the next-generation architecture:
- `test_enrollment_cli_model_ids_match_manifest`: Asserted `REQUIRED_MODEL_IDS` contains the 3 next-gen IDs and verifies cryptographic attestation against `models/manifest.toml` v2.0.0.
- `test_enrollment_cli_legacy_model_ids_absent`: Negative assertion verifying that none of the legacy model IDs exist in `REQUIRED_MODEL_IDS`.
- `test_daemon_refuses_start_with_missing_models`: Validated fail-closed startup when `scrfd_500m_kps` is absent from disk.
- `test_daemon_refuses_start_with_tampered_models`: Validated fail-closed startup when `scrfd_500m_kps` checksum does not match expected digest.

### Initial Red Phase Execution
Running `cargo test -p soos-enrollment-cli --test model_id_tests` failed as expected:
```text
running 3 tests
test test_camera_device_path_uses_stable_by_id ... ok
test test_enrollment_cli_legacy_model_ids_absent ... FAILED
test test_enrollment_cli_model_ids_match_manifest ... FAILED

failures:
---- test_enrollment_cli_legacy_model_ids_absent stdout ----
thread 'test_enrollment_cli_legacy_model_ids_absent' panicked at:
Legacy model 'ultraface_slim_320' must NOT be in REQUIRED_MODEL_IDS
---- test_enrollment_cli_model_ids_match_manifest stdout ----
thread 'test_enrollment_cli_model_ids_match_manifest' panicked at:
assertion `left == right` failed
  left: "ultraface_slim_320"
 right: "scrfd_500m_kps"
```

---

## 3. Phase 3 & 4 — Developer Implementation (TDD Green Phase)

The production code was updated to turn all test contracts green:
1. In `crates/daemon/src/pipeline.rs`:
   - Updated model session keys to `"scrfd_500m_kps"`, `"minifasnet_v2_pad"`, and `"arcface_w600k_mbf"`.
   - Replaced `OrtFaceDetector::new` with `OrtScrfdDetector::new(...)`.
2. In `crates/enrollment-cli/src/service.rs`:
   - Updated constants `MODEL_ID_FACE_DETECTOR`, `MODEL_ID_PAD`, `MODEL_ID_EMBEDDING`.
   - Removed `MODEL_ID_LANDMARKS`.
   - Reduced `REQUIRED_MODEL_IDS` array size to 3.
   - Updated `build_full_service` to construct `OrtScrfdDetector` with error propagation.
3. In `crates/enrollment-cli/src/lib.rs`:
   - Removed `MODEL_ID_LANDMARKS` from module re-exports.
4. In `Docs/ENROLLMENT_CLI.md`:
   - Updated Section 4 model attestation list to describe the 3-model next-gen architecture.

### Green Phase Execution
Running `cargo test -p soos-enrollment-cli` and `cargo test -p soos-daemon`:
```text
test test_camera_device_path_uses_stable_by_id ... ok
test test_enrollment_cli_legacy_model_ids_absent ... ok
test test_enrollment_cli_model_ids_match_manifest ... ok
test result: ok. 3 passed; 0 failed; 0 ignored; finished in 0.00s
```
Full workspace tests: 100% passed (0 failed). Zero clippy warnings across `--all-targets --all-features`.

---

## 4. Phase 5 — Candid Pre-Push Review

The Candid Reviewer Sub-Agent audited the raw diff against `origin/main` across the 5 pillars:
- **Logic & Architecture**: Exactly 3 sessions loaded; landmarks parsed from SCRFD detection; lazy init preserved.
- **PAM Concurrency & Deadlines**: PAM module untouched; verification latency optimized.
- **Panic Safety & Fallback**: All constructors return `Result` propagated via `?`; zero unwrap/expect.
- **Test Integrity**: Negative and positive test contracts assert v2.0.0 manifest without test weakening.
- **Memory & Secret Bounds**: Zeroization on drop preserved; no sensitive data on wire or in logs.
- Formal report written in `AI/candid_review_report.md` with `VERDICT: APPROVED`.

---

## 5. Phase 6 — Verification & Traceability

- Updated criteria `EN7` and added `NGM15` in `AI/VERIFICATION_MATRIX.md`.
- Dual synchronization performed via `python3 scripts/sync_issue.py`:
  - Checked off sub-issues #43.1, #43.2, #43.3 in `AI/BACKLOG.md`.
  - Updated GitHub Issue #109 body and added progress comments.
