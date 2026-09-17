# Walkthrough 32 — Fail-Closed Dispatcher and Production Pipeline Initialization

- **Issue**: #17 (`fix(daemon): Fail-closed dispatcher and production pipeline initialization`)
- **Branch**: `fix/daemon-fail-closed`
- **Component**: `daemon`
- **Verification Criteria**: `D12`, `D13`

---

## 1. Overview & Problem Statement

Prior to this issue, the background daemon `soos-daemon` had two critical limitations:
1. **Authentication Bypass Vulnerability**: In `crates/daemon/src/dispatcher.rs` (lines 497–506), the dispatcher contained a development skeleton fallback that returned `(Verdict::Allow, ReasonClass::FaceMatch)` whenever the socket was ready, even if no pipeline was configured. This constituted a complete authentication bypass.
2. **Missing Production Pipeline Initialization**: The daemon binary entry point (`crates/daemon/src/main.rs`) initialized only the socket listener and swap protection without instantiating the camera manager, machine learning model registry, ONNX Runtime sessions, biometric template store, evidence store, or policy engine. Furthermore, all daemon parameters were hardcoded without TOML configuration file support.

This issue eliminates the fail-open fallback, implements full production pipeline initialization, adds TOML configuration parsing, and introduces `--mock-camera` support for development and CI environments.

---

## 2. Multi-Agent TDD Implementation

### Phase 1: Specification & Architecture
- **Fail-Closed Dispatcher**: Mapped `pipeline == None` directly to `(Verdict::Unavailable, ReasonClass::InternalError)`.
- **TOML Configuration Parser**: Implemented serde deserialization for `/etc/soos/daemon.toml`, covering socket parameters, dispatcher connection limits and deadlines, camera device path, mock camera simulation, model registry directory, biometric store path, evidence retention and daily capture caps, policy cosine and PAD thresholds, and UID rate limits.
- **Pipeline Initialization Sequence**:
  1. Camera capture manager initialization (`V4lCameraManager::spawn` or `MockCameraManager::new` if `--mock-camera`).
  2. Loading or generating cryptographic master keys for biometrics (`BiometricStore`) and evidence (`EvidenceStore`).
  3. Configuring the authorization policy engine and per-UID rate limiter.
  4. Cryptographic attestation: loading `models/manifest.toml` via `ModelRegistry` and asserting SHA-256 integrity over model files.
  5. Instantiating isolated CPU ORT sessions for UltraFace Slim 320, InsightFace 5-point landmarks, MiniFASNet PAD, and MobileFaceNet feature extraction.
  6. Assembling high-level `VisionPipeline` and wiring into `PipelineComponents`.
  7. Registering pipeline with `ConnectionDispatcher::with_pipeline`.
  8. Deferring socket binding and readiness declaration until after pipeline validation succeeds (fail-closed startup).

### Phase 1.5: Plan Evaluation
- The Plan Evaluator Sub-Agent reviewed the proposed plan across 6 architectural pillars in `AI/plan_evaluator_report.md` and issued `VALIDATION_VERDICT: APPROVED`.

### Phase 2: Contractual Tests (Red Phase)
- Authored automated tests before production code:
  - `dispatcher_tests::test_dispatcher_no_pipeline_returns_unavailable_not_allow`: Verified that uninitialized pipeline returns `Unavailable` and `InternalError` (initial test run confirmed failure of `Allow`).
  - `config_tests::test_config_file_parsing_complete`: Validated complete TOML deserialization.
  - `config_tests::test_config_defaults_when_file_absent`: Validated default values when file is absent.
  - `config_tests::test_config_file_invalid_syntax_fails_closed`: Validated error handling on invalid TOML.
  - `pipeline_init_tests::test_daemon_startup_initializes_all_pipeline_components`: Validated multi-component initialization, atomic readiness, and end-to-end request dispatching.
  - `pipeline_init_tests::test_mock_camera_flag_uses_mock_manager`: Validated mock camera activation without hardware devices.
  - `pipeline_init_tests::test_pipeline_init_missing_models_fails_closed`: Validated fail-closed error reporting when models are absent.

### Phase 3: Security & Panic Safety Audit
- Validated zero `unwrap()` or `expect()` in daemon production code.
- Confirmed `#![forbid(unsafe_code)]` preserved on binary and library.
- Confirmed zero sensitive data (embeddings, raw frames) logged or exposed.

### Phase 4: Developer Implementation (Green Phase)
- Updated `crates/daemon/Cargo.toml` with `clap`, `serde`, and `toml`.
- Implemented `Inference` and `Config` variants in `DaemonError`.
- Implemented TOML deserialization and loader methods on `DaemonConfig`.
- Replaced skeleton fallback in `dispatcher.rs` with `Unavailable / InternalError`.
- Implemented `initialize_pipeline` in `pipeline.rs`.
- Updated `main.rs` with `clap::Parser`, `--config`, and `--mock-camera` flags, executing fail-closed pipeline initialization prior to socket binding.
- Re-exported `PipelineConfig` and `initialize_pipeline` in `lib.rs`.

### Phase 5: Candid Review
- Impartial review conducted and recorded in `AI/candid_review_report.md` with `VERDICT: APPROVED`.

---

## 3. Verification & Test Evidence

All 19 test targets across the workspace compile and pass cleanly:
```bash
cargo test -p soos-daemon
```
Results:
- `config_tests`: 3 passed, 0 failed
- `dispatcher_tests`: 6 passed, 0 failed
- `pipeline_init_tests`: 3 passed, 0 failed
- `pipeline_integration_tests`: 7 passed, 0 failed
- `health_tests`: 3 passed, 0 failed
- `peercred_tests`: 4 passed, 0 failed
- `socket_tests`: 4 passed, 0 failed
- `hardening_tests`: 5 passed, 0 failed
- `logging_audit_test`: 1 passed, 0 failed
- `systemd_test`: 1 passed, 0 failed

Full workspace suite (`cargo test --all-targets --all-features`): 100% green.
Code formatting (`cargo fmt --check`): clean.
Clippy lints (`cargo clippy --all-targets --all-features -- -D warnings`): zero warnings.
