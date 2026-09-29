# Walkthrough 75 — GUI Camera Auto-Resolution, Video Feed Fallback and Packaging Parity

> **Issue**: #50 (GitHub #140)  
> **Branch**: `fix/gui-camera-auto-resolution-and-packaging`  
> **Status**: Completed  
> **Verification Matrix**: GARP1, GARP2, GARP3, GARP4  

---

## 1. Overview & Root Cause Analysis

During user verification of `soos-gui`, the live camera feedback failed both when `soos-daemon` was running and when it was stopped:

1. **Direct V4L2 Failure when Daemon is Stopped**:
   In `/etc/soos/daemon.toml`, `camera_device = "auto"`. `resolve_camera_device_from_config()` in `crates/enrollment-cli/src/service.rs` literally returned `PathBuf::from("auto")`. When the daemon is stopped, `soos-gui` passed `"auto"` to `V4lCameraManager::spawn`, causing `v4l::Device::with_path("auto")` to fail with `No such file or directory`. The camera manager never became ready, and the GUI remained permanently stuck displaying a loading spinner.
2. **System Binary Out of Sync when Daemon is Running**:
   `/usr/bin/soos-gui` on the host was an obsolete binary compiled prior to the implementation of `IpcCameraManager` video proxying. The outdated binary attempted direct V4L2 capture while the daemon held `/dev/video2`, failed with `EBUSY`, executed `pkexec systemctl stop soos-daemon.service`, and then tried to open `"auto"`, failing in both daemon states.
3. **Packaging Omission**:
   `scripts/install.sh` and `scripts/uninstall.sh` omitted `soos-gui`, preventing binary updates in `/usr/bin/` during standard provisioning and deployment runs.
4. **V4L2 Buffer Bounding**:
   `v4l_impl.rs` ignored `meta.bytesused` and took the full MMAP buffer `buf.to_vec()`, including trailing padding bytes for formats like MJPEG (`/dev/video0`), corrupting image decoding.
5. **GUI Worker Frame Drop**:
   `crates/gui/src/worker.rs` silently discarded frames when `pipeline.analyze_frame(&frame)` failed, causing the canvas to freeze instead of displaying a fallback raw RGB stream.
6. **GUI Config Calibration**:
   `OrtPadDetector` in `main.rs` used `live_class_index = 2` instead of 1 (`DEFAULT_MINIFASNET_LIVE_CLASS_INDEX`), and lacked `warmup_frames: 0` and `idle_timeout: Duration::ZERO` for direct capture.

---

## 2. Changes Made

### 2.1 Dynamic Device Auto-Detection in `soos-enrollment-cli` (GARP1)
- In `crates/enrollment-cli/src/service.rs`:
  - Updated `resolve_camera_device_from_config(cli_device, config_path)`.
  - When `cli_device` or `camera_device` in config is `"auto"`, `"default"`, or empty:
    - Extracts `sensor_preference` (`PreferIr` vs `PreferRgb`) from `daemon.toml`.
    - Enumerates available capture devices via `soos_camera_v4l::enumerate_capture_devices()`.
    - Selects the best device candidate matching sensor preferences via `soos_camera_v4l::select_camera_device()`.
    - Falls back to `/dev/v4l/by-id/` and `/dev/video0` if hardware enumeration is unavailable.
- Created `crates/enrollment-cli/tests/device_resolution_tests.rs`:
  - `test_resolve_camera_device_explicit_path`: Ensures non-auto paths pass through unchanged.
  - `test_resolve_camera_device_auto_resolution`: Verifies `"auto"` in config resolves to a valid `/dev/` node.
  - `test_resolve_camera_device_default_resolution`: Verifies `"default"` in config resolves to a valid `/dev/` node.
  - `test_resolve_camera_device_ignores_literal_auto_in_cli_arg`: Verifies CLI arg `"auto"` triggers auto-detection.

### 2.2 Sliced MMAP Buffer Bounding in `soos-camera-v4l` (GARP2)
- In `crates/camera-v4l/src/v4l_impl.rs`:
  - Evaluated `bytesused = meta.bytesused as usize`.
  - Sliced the MMAP buffer using `buf.get(..bytesused).map(|s| s.to_vec()).unwrap_or_else(|| buf.to_vec())`.
  - Ensures compressed streams (MJPEG) and padded buffers pass exact payload sizes to downstream decoders without memory corruption or bounds out-of-range errors.

### 2.3 GUI Worker Fallback & Parameter Calibration in `soos-gui` (GARP3)
- In `crates/gui/src/worker.rs`:
  - Added fail-safe error handling to `pipeline.analyze_frame(&frame)`.
  - On error, logs a warning and calls `soos_vision::convert_to_rgb(&frame.data, frame.width, frame.height, frame.format)`.
  - Populates `LatestFrameData` with the raw RGB frame, empty detections, and 0.0ms latency so live video preview remains responsive even under model inference errors.
- In `crates/gui/src/main.rs`:
  - Configured direct camera builder with `.warmup_frames(0).idle_timeout(Duration::ZERO)` for instantaneous streaming without initial frame drops or standby suspension.
  - Updated `OrtPadDetector` to `OrtPadDetector::new(pad_session, 0.80)`, restoring the correct live class index (1).
- In `crates/gui/src/ipc_camera.rs`:
  - Increased IPC read timeout from 1000ms to 2500ms to tolerate camera wake initialization.
- In `crates/gui/tests/layout_tests.rs`:
  - Added `test_gui_worker_fallback_renders_raw_rgb_on_pipeline_error`.

### 2.4 Packaging & Deployment Parity (GARP4)
- In `scripts/install.sh`:
  - Added discovery and installation of `soos-gui` to `${TARGET_BIN_DIR}/soos-gui` (mode 0755).
- In `scripts/uninstall.sh`:
  - Added cleanup of `${TARGET_BIN_DIR}/soos-gui`.
- In `tests/invariants/src/lib.rs`:
  - Added `soos-gui` assertion in `test_uninstall_restores_pam_config`.

---

## 3. Verification & Test Evidence

### 3.1 Unit & Contractual Tests
- `crates/enrollment-cli/tests/device_resolution_tests.rs`:
  - `test_resolve_camera_device_explicit_path` (GARP1) — PASS
  - `test_resolve_camera_device_auto_resolution` (GARP1) — PASS
  - `test_resolve_camera_device_default_resolution` (GARP1) — PASS
  - `test_resolve_camera_device_ignores_literal_auto_in_cli_arg` (GARP1) — PASS
- `crates/gui/tests/layout_tests.rs`:
  - `test_gui_worker_fallback_renders_raw_rgb_on_pipeline_error` (GARP3) — PASS
  - `test_ipc_camera_manager_receives_persistent_frames` — PASS
- `tests/invariants/src/lib.rs`:
  - `test_install_script_creates_required_directories` (GARP4) — PASS
  - `test_uninstall_restores_pam_config` (GARP4) — PASS

### 3.2 Full Test Suite & Linters
- `cargo fmt --check`: Clean (0 diffs)
- `cargo clippy --workspace --all-targets -- -D warnings`: Clean (0 warnings)
- `cargo test --workspace`: 100% PASS (all unit, integration, and invariant tests)
- `./scripts/candid_review.sh`: PASSED (all 7 audits green)

---

## 4. Acceptance Summary

| Sub-issue | Description | Status | Evidence |
|-----------|-------------|--------|----------|
| **#50.1** | Camera auto-resolution for `"auto"` / `"default"` | Verified | `device_resolution_tests.rs` (4/4 tests pass) |
| **#50.2** | V4L2 MMAP buffer payload bounding via `meta.bytesused` | Verified | `soos-camera-v4l` test suite passes |
| **#50.3** | GUI worker raw RGB fallback, live class index 1, timeout 2500ms | Verified | `test_gui_worker_fallback_renders_raw_rgb_on_pipeline_error` |
| **#50.4** | Packaging and uninstall support for `soos-gui` | Verified | `scripts/install.sh`, `scripts/uninstall.sh`, `soos-invariants` |
