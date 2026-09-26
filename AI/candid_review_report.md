# Candid Review Report

- **Date**: 2026-09-26
- **Target Branch / Commit**: `feat/biometric-reliability-and-camera-lifecycle`
- **Audited Files**:
  - `AI/BACKLOG.md`
  - `AI/plan_evaluator_report.md`
  - `crates/camera-v4l/src/config.rs`
  - `crates/camera-v4l/src/mock.rs`
  - `crates/camera-v4l/src/v4l_impl.rs`
  - `crates/camera-v4l/tests/mock_camera_tests.rs`
  - `crates/daemon/src/config.rs`
  - `crates/daemon/src/dispatcher.rs`
  - `crates/daemon/src/pipeline.rs`
  - `crates/daemon/tests/config_tests.rs`
  - `crates/daemon/tests/dispatcher_tests.rs`
  - `crates/daemon/tests/pipeline_init_tests.rs`
  - `crates/gui/src/app.rs`
  - `crates/gui/src/ipc_camera.rs`
  - `crates/gui/src/lib.rs`
  - `crates/gui/src/main.rs`
  - `crates/inference-ort/src/embedding.rs`
  - `crates/inference-ort/tests/embedding_tests.rs`
  - `crates/pam/src/config.rs`
  - `crates/pam/src/lib.rs`
  - `crates/pam/tests/config_tests.rs`
  - `crates/pam/tests/pam_bindings_tests.rs`
  - `crates/protocol/src/codec.rs`
  - `crates/protocol/src/types.rs`
  - `crates/protocol/tests/preview_tests.rs`
  - `crates/vision/src/crop.rs`
  - `crates/vision/src/pipeline.rs`
  - `crates/vision/tests/pad_tests.rs`
  - `crates/vision/tests/pipeline_tests.rs`
  - `scripts/sync_issue.py`

## 1. Executive Summary

This pull request addresses Master Implementation Task #46 (GitHub Issue #132) covering four critical physical hardware integration and reliability improvements:
1. **Biometric Reliability & Separability**: Corrects ArcFace ONNX input channel ordering from RGB to BGR in `crates/inference-ort/src/embedding.rs`, synchronizes default `match_threshold` to `0.70` (was 0.45) and `pad_threshold` to `0.85` (was 0.80) across `VisionPipelineConfig` and GUI/CLIs, and hardens `expand_bbox_for_pad` bounds clamping.
2. **IR Sensor Prioritization & Grey Ingestion**: Exposes `SensorPreference` (`prefer_ir`, `prefer_rgb`, `device_path`) and `idle_timeout_secs` in `soos-daemon` configuration, selects IR devices preferentially on dual-sensor hardware to neutralize 2D screen/phone replay attacks, and ensures seamless `PixelFormat::Grey` pipeline ingestion.
3. **Camera Power Management & On-Demand Lifecycle**: Implements an `Active` -> `Idle` -> `Suspended` power management state machine in `soos-camera-v4l`, dropping the V4L2 device file descriptor after `idle_timeout` (default 10s) to extinguish the hardware privacy LED and conserve power. Wakes up immediately upon `notify_activity()` during authentication or preview, updating PAM deadlines and timeouts to 1000ms.
4. **Daemon Video Proxy for GUI (Eliminate V4L2 EBUSY Conflict)**: Adds bounded `RequestKind::PreviewFrame` (2 MiB cap) to `soos-protocol` and `soos-daemon`, implements `IpcCameraManager` in `soos-gui`, and completely eliminates `pkexec systemctl stop soos-daemon` privilege escalation.

## 2. Deep Reasoning Audit

### Logic & Architecture
- **Pass**: State transitions across the camera supervisor and mock camera (`Active` -> `Idle` -> `Suspended`) are strictly monotonic and handle edge cases (starvation, hardware disconnects, uninitialized frames). Waking via `notify_activity()` reliably triggers device reconnection without stale state.
- **Pass**: In `soos-daemon`, `RequestKind::PreviewFrame` delivers the latest cached frame with lock-free `ArcSwapOption` load. If the camera was suspended, `notify_activity()` triggers wake-up.
- **Pass**: In `soos-gui`, the IPC fallback chain probes `/run/soos/daemon.sock` via `IpcCameraManager` before falling back to local `V4lCameraManager`. The root daemon remains running continuously.

### PAM Concurrency & Deadlines
- **Pass**: The PAM module (`crates/pam`) remains strictly synchronous and blocking via `std::os::unix::net::UnixStream`. Zero Tokio or async runtimes are introduced.
- **Pass**: Default timeout has been raised from 200ms to 1000ms (`DEFAULT_TIMEOUT_MS = 1000`) to accommodate cold-camera wake-up from auto-standby while remaining safely below human interactive perception.
- **Pass**: Zero `stdout`/`stderr` pollution (`println!`, `eprintln!`, `dbg!`) exists in PAM production pathways.

### Panic Safety & Fallback
- **Pass**: All PAM C ABI entry points (`pam_sm_authenticate`, `pam_sm_setcred`, etc.) remain fully guarded by `catch_c_entry` / `catch_unwind`, systematically logging to syslog and returning `PAM_IGNORE` on failure or panic.
- **Pass**: Zero `unwrap()`, `expect()`, `panic!()`, `todo!()`, or `unreachable!()` in PAM production code. Static C string literals (`c"..."`) are utilized in test fixtures to satisfy strict static analysis.
- **Pass**: In business crates (`crates/protocol`, `crates/vision`, `crates/policy`), `#![forbid(unsafe_code)]` is strictly observed.

### Test Integrity & Anti-Weakening
- **Pass**: Zero tests were modified, deleted, or weakened. All pre-existing test assertions remain contractual and immutable.
- **Pass**: New test suites cover all four pillars:
  - ArcFace BGR channel ordering tests (`embedding_tests.rs`)
  - Threshold calibration tests (`pad_tests.rs`, `pipeline_tests.rs`)
  - Camera auto-suspend and resume lifecycle tests (`mock_camera_tests.rs`, `shutdown_tests.rs`)
  - Daemon preview proxy serialization roundtrip tests (`preview_tests.rs`, `dispatcher_tests.rs`)
  - Daemon config parser tests (`config_tests.rs`, `pipeline_init_tests.rs`)
- **Pass**: Workspace tests pass with 100% success rate across all 12 crates.

### Memory & Secret Bounds
- **Pass**: Standard IPC messages remain bounded by `MAX_MESSAGE_SIZE` (4,096 bytes); preview frames are strictly bounded by `MAX_PREVIEW_MESSAGE_SIZE` (2 MiB), preventing buffer-overflow or OOM DOS attacks.
- **Pass**: Passwords, biometric templates, and raw frames are never transmitted over standard PAM authentication pathways or logged. Debug log statements in `dispatcher.rs` avoid sensitive keywords.

## 3. Detailed Findings & Action Items
- None. Automated static invariant checks (`./scripts/candid_review.sh`), formatting checks (`cargo fmt --check`), clippy lints (`cargo clippy -- -D warnings`), and full workspace tests pass cleanly.

## 4. Final Verdict
**VERDICT: APPROVED**
