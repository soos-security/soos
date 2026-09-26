# Walkthrough 71: Biometric Reliability, Camera Power Lifecycle, IR Prioritization, and Daemon-Proxied GUI Preview

## Objective

Resolve four critical integration and architectural problems identified during physical hardware validation on dual-sensor laptops:
1. **Biometric Reliability & Separability**: Fix false acceptance (FAR) where another person was accepted as the enrolled user.
2. **Hardware Infrared (IR) Sensor Prioritization**: Prioritize IR capture when available, physically neutralizing 2D screen/phone replay attacks and handling `PixelFormat::Grey`.
3. **Camera Power Management & On-Demand Lifecycle**: Implement an auto-standby state machine (`Active` -> `Idle` -> `Suspended`) to drop the V4L2 device file descriptor and extinguish the hardware privacy LED after 10s of inactivity.
4. **Daemon Video Proxy for GUI (Eliminate V4L2 EBUSY Conflict)**: Implement a streaming preview proxy via Unix domain socket in `soos-daemon` and `soos-protocol`, allowing `soos-gui` to stream frames without stopping `soos-daemon` or prompting for Polkit elevation.

---

## Root Causes Identified

1. **ArcFace Input Inversion**: The InsightFace ONNX model (`arcface_w600k_mbf.onnx`) is trained on BGR channel order. `soos-inference-ort` was writing RGB channels, degrading embedding distance separability between distinct human faces.
2. **Permissive Default Match Threshold**: `VisionPipelineConfig` defaulted to `0.45` match threshold, while the security policy baseline in `soos_policy::ThresholdConfig` required `0.70`.
3. **Continuous Background Capture**: `soos-daemon` opened the camera device at startup and kept the V4L2 MMAP stream active continuously, keeping the camera hardware privacy LED lit and draining battery.
4. **V4L2 Device Conflict**: When `soos-gui` was launched, it attempted to open `/dev/video0` directly. Because Linux V4L2 devices do not support concurrent MMAP streams from multiple processes, `soos-gui` required `pkexec systemctl stop soos-daemon` to free the camera, creating friction and leaving the background PAM authentication service dead.

---

## Architectural Changes & Key Implementations

### 1. `soos-inference-ort` (`crates/inference-ort/src/embedding.rs`)
- Fixed channel ordering in `OrtEmbeddingExtractor::prepare_input()`: channel 0 is Blue, channel 1 is Green, channel 2 is Red:
  ```rust
  let b = rgb[src_idx + 2] as f32;
  let g = rgb[src_idx + 1] as f32;
  let r = rgb[src_idx] as f32;
  ```
- Normalization remains `(pixel - 127.5) / 127.5` mapping cleanly to `[-1.0, 1.0]`.

### 2. `soos-vision` (`crates/vision/src/pipeline.rs`, `crates/vision/src/crop.rs`)
- Synchronized `VisionPipelineConfig::default()` to match `soos_policy::ThresholdConfig`:
  - `match_threshold`: `0.70` (was `0.45`)
  - `pad_threshold`: `0.85` (was `0.80`)
- Hardened `expand_bbox_for_pad` in `crop.rs` to clamp bounds strictly within image dimensions while preserving scale.

### 3. `soos-camera-v4l` (`crates/camera-v4l/src/v4l_impl.rs`, `mock.rs`, `config.rs`)
- Added `idle_timeout: Duration` (default 10 seconds) and `idle_fps: u32` (default 5 FPS) to `CameraConfig`.
- Implemented `SupervisorAction::Suspend` in `run_v4l_supervisor`: drops the `v4l::Device` handle and MMAP stream when idle for more than `idle_timeout`.
- Implemented responsive wake-up upon `notify_activity()`: immediately attempts device reconnection.
- Updated `MockCameraManager` to mirror the full power lifecycle and maintain Acquire-Release memory visibility guarantees for `latest_frame()`.

### 4. `soos-protocol` (`crates/protocol/src/types.rs`, `codec.rs`)
- Added `RequestKind::PreviewFrame` to `RequestKind`.
- Added `PreviewResponse { width: u32, height: u32, format: PixelFormat, data: Vec<u8> }`.
- Defined `MAX_PREVIEW_MESSAGE_SIZE = 2 * 1024 * 1024` (2 MiB) with dedicated bounded helpers `encode_preview()` and `decode_preview()`.
- Added comprehensive unit tests in `crates/protocol/tests/preview_tests.rs`.

### 5. `soos-pam` (`crates/pam/src/config.rs`)
- Updated `DEFAULT_TIMEOUT_MS` from 200ms to 1000ms to accommodate camera cold-start and sensor initialization when waking from auto-standby.

### 6. `soos-daemon` (`crates/daemon/src/config.rs`, `pipeline.rs`, `dispatcher.rs`)
- Added `sensor_preference` (`prefer_ir`, `prefer_rgb`, `device_path`) and `idle_timeout_secs` to `PipelineConfigFile`.
- Updated `init_camera()` in `pipeline.rs` to auto-select candidate devices matching `sensor_preference` using `select_camera_device()`.
- Implemented `RequestKind::PreviewFrame` in `dispatcher.rs`:
  - Calls `camera.notify_activity()` to restore full frame acquisition.
  - Retrieves latest frame via lock-free `ArcSwapOption` and returns `encode_preview()`.
  - In `handle_auth()`, calls `camera.notify_activity()` and waits up to 600ms for camera warm-up if the camera was suspended.

### 7. `soos-gui` (`crates/gui/src/ipc_camera.rs`, `main.rs`, `app.rs`)
- Implemented `IpcCameraManager` querying `/run/soos/daemon.sock` via `soos_protocol` without root privileges.
- Completely removed `pkexec systemctl stop soos-daemon` and `pkexec systemctl start soos-daemon`.
- Updated `main.rs` to detect if `/run/soos/daemon.sock` is active and connect via `IpcCameraManager`, only falling back to direct `V4lCameraManager` if the daemon is stopped.
- Synchronized `match_threshold` in `SoosApp` to `0.70f32`.

---

## Verification & Test Results

- `cargo fmt --all -- --check`: passed (100% compliant).
- `cargo clippy --workspace --all-targets -- -D warnings`: passed with 0 warnings.
- `./scripts/candid_review.sh`: passed with 0 invariant violations across all 7 checks.
- Full workspace test suite (`cargo test --workspace`):
  - `soos-inference-ort`: all tests passed (including ArcFace BGR channel ordering).
  - `soos-vision`: all tests passed (including threshold calibration and pad tests).
  - `soos-protocol`: all tests passed (including preview codec and fuzz tests).
  - `soos-camera-v4l`: all tests passed (including auto-suspend, wake, and Acquire-Release tests).
  - `soos-pam`: all tests passed (including C ABI panic safety and 1000ms timeout).
  - `soos-daemon`: all tests passed (including preview dispatcher and config parsing).
  - `soos-gui`: all tests passed.
