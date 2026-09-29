# Candid Review Report

- **Date**: 2026-09-29
- **Target Branch / Commit**: `fix/gui-camera-auto-resolution-and-packaging`
- **Audited Files**:
  - `AI/BACKLOG.md`
  - `AI/plan_evaluator_report.md`
  - `crates/camera-v4l/src/v4l_impl.rs`
  - `crates/enrollment-cli/src/service.rs`
  - `crates/enrollment-cli/tests/device_resolution_tests.rs`
  - `crates/gui/src/ipc_camera.rs`
  - `crates/gui/src/main.rs`
  - `crates/gui/src/worker.rs`
  - `crates/gui/tests/layout_tests.rs`
  - `scripts/install.sh`
  - `scripts/sync_issue.py`
  - `scripts/uninstall.sh`
  - `tests/invariants/src/lib.rs`

## 1. Executive Summary

This pull request addresses Master Bug Fix Issue #50 (GitHub Issue #140), fixing the video feedback failure in `soos-gui` both when the daemon is running and when it is stopped:
1. **Device Resolution Auto-Detection (`#50.1`)**: `resolve_camera_device_from_config` in `crates/enrollment-cli/src/service.rs` now parses `camera_device = "auto"`, `"default"`, or empty strings by inspecting `sensor_preference` and invoking `soos_camera_v4l::enumerate_capture_devices()` and `select_camera_device()`, gracefully falling back to `/dev/v4l/by-id/` and `/dev/video0`.
2. **V4L2 Frame Buffer Bounding (`#50.2`)**: In `crates/camera-v4l/src/v4l_impl.rs`, frame payload retrieval slices the MMAP buffer strictly to `meta.bytesused`, eliminating trailing buffer padding that corrupted JPEG decoders and color converters on compressed camera streams.
3. **GUI Stream Resiliency & Neural Calibration (`#50.3`)**: In `crates/gui/src/worker.rs`, the vision worker implements a fail-safe fallback rendering raw camera frames converted to RGB24 whenever the neural pipeline encounters an analysis error, ensuring continuous visual feedback. In `main.rs`, direct camera initialization disables frame discard (`warmup_frames = 0`, `idle_timeout = Duration::ZERO`), and `OrtPadDetector` is configured with genuine live class index 1. In `ipc_camera.rs`, IPC read timeout is increased to 2500ms.
4. **System Packaging & Deployment Parity (`#50.4`)**: `scripts/install.sh` and `scripts/uninstall.sh` now include `soos-gui` in target bin installation and uninstallation, and invariant tests assert `soos-gui` lifecycle management.

## 2. Deep Reasoning Audit

### Logic & Architecture
- **Pass**: Camera device auto-detection correctly respects explicit non-auto paths, while properly resolving dynamic device nodes when `camera_device` is `"auto"` or `"default"`.
- **Pass**: Slicing MMAP buffers to `meta.bytesused` ensures exact payload delivery to downstream decoders without memory corruption or bounds out-of-range errors.
- **Pass**: Vision worker fallback in `soos-gui` preserves the responsiveness of the GUI preview canvas even if model inference or face detection encounters transient failures.

### PAM Concurrency & Deadlines
- **Pass**: No changes made to `crates/pam` or any PAM authentication pathway. The PAM module remains strictly synchronous with zero async runtimes and zero `stdout`/`stderr` pollution.

### Panic Safety & Fallback
- **Pass**: Production code across `crates/gui`, `crates/enrollment-cli`, and `crates/camera-v4l` contains zero `unwrap()` or `expect()`.
- **Pass**: `#![forbid(unsafe_code)]` remains strictly enforced in `crates/gui` and `crates/enrollment-cli`. The unsafe boundary in `crates/camera-v4l/src/v4l_impl.rs` is fully documented and unchanged.

### Test Integrity & Anti-Weakening
- **Pass**: Zero tests were modified, deleted, or weakened.
- **Pass**: New unit test suites were added:
  - `crates/enrollment-cli/tests/device_resolution_tests.rs` (4 tests validating explicit paths, auto resolution, default resolution, and fallback).
  - `crates/gui/tests/layout_tests.rs` (`test_gui_worker_fallback_renders_raw_rgb_on_pipeline_error`).
  - `tests/invariants/src/lib.rs` (`test_uninstall_restores_pam_config` checks `soos-gui` removal).
- **Pass**: All tests in the workspace pass cleanly (100% success rate).

### Memory & Secret Bounds
- **Pass**: All buffer slices are bounded by checked lengths and `meta.bytesused`.
- **Pass**: IPC preview message sizing remains strictly constrained by `MAX_PREVIEW_MESSAGE_SIZE` (2 MiB).
- **Pass**: No sensitive authentication credentials, passwords, or raw embeddings are exposed or logged.

## 3. Detailed Findings & Action Items
- None. Automated static checks (`./scripts/candid_review.sh`), code formatting (`cargo fmt --check`), clippy lints (`cargo clippy -- -D warnings`), and full workspace tests pass cleanly.

## 4. Final Verdict
**VERDICT: APPROVED**
