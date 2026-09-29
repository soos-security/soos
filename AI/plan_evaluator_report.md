# Plan Evaluation Report — Issue #50: Camera Auto-Device Resolution, GUI Video Feed Fallback & Packaging Synchronization

**Target Issue**: Backlog Issue #50 / GitHub Issue #140  
**Topic Branch**: `fix/gui-camera-auto-resolution-and-packaging`  
**Evaluator**: Plan Evaluator Sub-Agent  
**Date**: 2026-09-29  

---

## 1. Executive Summary & Compliance Overview

This report evaluates the architectural plan to resolve camera auto-detection failure in `resolve_camera_device_from_config()`, direct V4L2 streaming failures on `"auto"` device paths, GUI video display freeze under pipeline error, un-sliced V4L2 MMAP buffer boundaries, and packaging synchronization for `soos-gui`.

The plan addresses:
1. **Dynamic Hardware Auto-Detection**: Updating `resolve_camera_device_from_config()` in `crates/enrollment-cli` to handle `"auto"`, `"default"`, or omitted device paths by parsing `sensor_preference` and invoking `enumerate_capture_devices()` + `select_camera_device()`, ensuring that non-daemon direct camera access in `soos-gui` and `soos-enroll` correctly binds to physical camera nodes (e.g. `/dev/video2` IR) instead of literal `"auto"`.
2. **Exact Frame Buffer Bounding**: Slicing MMAP buffers to `meta.bytesused` in `crates/camera-v4l/src/v4l_impl.rs` when `bytesused > 0 && bytesused <= buf.len()`, eliminating uninitialized MMAP padding trailing behind compressed MJPEG and raw camera frames.
3. **Resilient Video Streaming Fallback**: Updating `crates/gui/src/worker.rs` so that if `pipeline.analyze_frame(&frame)` errors (e.g. transient model inference error or invalid bbox crop), the worker safely converts the frame to RGB and continues updating `LatestFrameData` with empty detections, ensuring live video feedback never drops or hangs on the loading spinner.
4. **GUI Configuration & MiniFASNet Calibration**: Aligning `OrtPadDetector` in `crates/gui/src/main.rs` to `DEFAULT_MINIFASNET_LIVE_CLASS_INDEX` (1), configuring `warmup_frames: 0` and `idle_timeout: Duration::ZERO` for direct GUI capture, and increasing IPC stream read timeout to 2500ms to eliminate wake race conditions.
5. **Installer & Packaging Synchronization**: Adding `soos-gui` to `scripts/install.sh` and `scripts/uninstall.sh` to ensure binary deployments keep `/usr/bin/soos-gui` synchronized with workspace builds.

---

## 2. Evaluation Across the 6 Architectural Pillars

### Pillar 1: Architectural Alignment & Threat Model
- **Socket Permissions & IPC Protocol**: Communication remains strictly bounded over `/run/soos/daemon.sock` (`0660 root:soos`). All framed requests continue to enforce `MAX_MESSAGE_SIZE` (4 KiB) and `MAX_PREVIEW_MESSAGE_SIZE` (2 MiB).
- **UID Verification**: Kernel `SO_PEERCRED` validation remains strictly enforced on all authentication and diagnostic requests.
- **Biometric Boundary**: Biometric templates remain encrypted in `/var/lib/soos/biometrics/` (`0600 root:root`). Direct and proxied preview streams handle diagnostic frame buffers only.
- *Status*: **Compliant**

### Pillar 2: PAM Real-Time Latency & Concurrency
- **Zero Tokio in PAM**: `pam_soos.so` remains purely synchronous with blocking socket I/O and strict 250ms deadline.
- **Instant Direct Camera Startup**: Direct GUI camera capture initializes with `warmup_frames: 0` and `idle_timeout: Duration::ZERO`, eliminating camera wake delays and auto-suspension while GUI is active.
- **Output Isolation**: PAM module emits only sanitized `PAM_TEXT_INFO` via conversation callbacks without stdout/stderr pollution.
- *Status*: **Compliant**

### Pillar 3: Panic Safety & Fail-Closed Behavior
- **Zero Unwraps in Production**: All additions across `enrollment-cli`, `camera-v4l`, and `gui` use typed `Result` handling with `thiserror`, safe defaults, and `?`.
- **Fail-Closed Fallback**: In the GUI worker, if pipeline analysis fails, the fallback safely converts the frame to RGB for rendering without inventing spoof or biometric match verdicts (verdict remains locked / unauthenticated).
- *Status*: **Compliant**

### Pillar 4: Dependency Isolation & Banned Crates
- **Forbidden Crates**: Zero `opencv` or `nokhwa`. Camera capture remains backed exclusively by `v4l` and `MockCameraManager`.
- **Business Logic Safety**: `#![forbid(unsafe_code)]` remains strictly enforced in `protocol`, `policy`, `vision`, `enrollment-cli`, `admin-cli`, and `gui`.
- *Status*: **Compliant**

### Pillar 5: Data Confidentiality & Zeroization
- **Sensitive Data Isolation**: Passwords, raw embeddings, and key material are never logged or exposed.
- **Buffer Hygiene**: Ephemeral frame vectors and preview buffers are managed within memory limits and zeroized where appropriate.
- *Status*: **Compliant**

### Pillar 6: Test Integrity & TDD Contracts
- **Red-Green TDD Workflow**: Unit and integration tests for auto-device resolution, buffer slicing, GUI worker fallback, and packaging are authored BEFORE production code.
- **Zero Weakening**: All existing tests across the workspace remain untouched and valid.
- *Status*: **Compliant**

---

## 3. Implementation Plan Specification

### Component 1: `crates/enrollment-cli` (`service.rs`)
- In `resolve_camera_device_from_config(cli_device, config_path)`:
  - If `cli_device` is provided and is not `"auto"` / `"default"` / empty, return it.
  - Parse `pipeline.camera_device` and `pipeline.sensor_preference` from `daemon.toml`.
  - If `camera_device` is specified, starts with `/dev/`, and is not `"auto"` / `"default"`: return it.
  - Otherwise, parse `sensor_preference` (defaulting to `SensorPreference::PreferIr`).
  - Call `soos_camera_v4l::enumerate_capture_devices()`.
  - Call `soos_camera_v4l::select_camera_device(&candidates, sensor_pref)`.
  - If candidate found, return `candidate.path`.
  - Fall back to first deterministic entry in `/dev/v4l/by-id/`, then `/dev/video0`, then `DEFAULT_CAMERA_DEVICE`.

### Component 2: `crates/camera-v4l` (`v4l_impl.rs`)
- In `open_and_stream`:
  - When `stream.next()` returns `(buf, meta)`:
  - Slice `buf` to `&buf[..meta.bytesused as usize]` when `meta.bytesused > 0 && (meta.bytesused as usize) <= buf.len()`.
  - Ensures exact frame bounds without trailing MMAP padding.

### Component 3: `crates/gui` (`worker.rs`, `main.rs`, `ipc_camera.rs`)
- In `worker.rs`:
  - If `pipeline.analyze_frame(&frame)` returns `Err(err)`:
  - Log warning: `tracing::warn!("Pipeline frame analysis error: {err}; rendering raw frame fallback")`.
  - Convert frame to RGB via `soos_vision::convert_to_rgb`.
  - Push `LatestFrameData` with empty detections and 0ms model latency.
- In `main.rs`:
  - Initialize `OrtPadDetector` with `DEFAULT_MINIFASNET_LIVE_CLASS_INDEX` (1).
  - Configure direct V4L2 camera with `.warmup_frames(0)` and `.idle_timeout(Duration::ZERO)`.
- In `ipc_camera.rs`:
  - Set stream read timeout to `Duration::from_millis(2500)` and write timeout to `Duration::from_millis(1000)`.

### Component 4: `scripts/` (`install.sh`, `uninstall.sh`)
- In `scripts/install.sh`:
  - Find artifact `soos-gui` and install to `${TARGET_BIN_DIR}/soos-gui` (mode 0755).
- In `scripts/uninstall.sh`:
  - Remove `${TARGET_BIN_DIR}/soos-gui`.

---

## 4. Formal Verdict

**VALIDATION_VERDICT: APPROVED**
