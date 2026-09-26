# Walkthrough 74 — Camera Latency, Auto-Standby Disable, Lockscreen Persistence & GUI Preview Streaming

> **Issue**: #49 (GitHub #138)  
> **Branch**: `feat/camera-latency-lockscreen-gui-preview`  
> **Status**: Completed  
> **Verification Matrix**: CLP1, CLP2, CLP3, CLP4  

---

## 1. Overview & Root Cause Analysis

During physical verification on Linux lockscreens (GDM) and in `soos-gui`, several camera-related issues were identified:
1. **Broken `idle_timeout_secs = 0` (Auto-Standby Disable)**:
   In `crates/camera-v4l/src/v4l_impl.rs` and `mock.rs`, the idle check evaluated `elapsed > config.idle_timeout`. When `idle_timeout` was `Duration::ZERO`, any elapsed time was immediately greater than zero, causing the camera to immediately suspend on every single frame! Setting `idle_timeout_secs = 0` in `/etc/soos/daemon.toml` intended to keep the camera streaming permanently, but instead caused perpetual suspension.
2. **Warmup Frames Latency & Timeout**:
   When waking from standby, `PipelineConfig` defaulted to `warmup_frames = 20` (~700–1300ms at 15–30 FPS). During auth requests on the lock screen, the camera manager was discarding 20 frames while the authentication wake loop timed out, repeatedly returning `CameraUnavailable`.
3. **Broken Pipe in GUI Preview Streaming**:
   `ConnectionDispatcher::handle_connection` processed exactly one request per connection and closed the socket stream. When `soos-gui` connected over `/run/soos/daemon.sock` via `IpcCameraManager`, subsequent requests resulted in `Broken pipe` (code 32). This caused `is_ready` to flip to `false` and triggered a 100ms backoff loop, breaking live preview.
4. **GUI Fallback Conflict**:
   If the GUI's initial `probe()` timed out waiting for a frame while the camera was waking, `soos-gui` fell back to opening the direct V4L2 device (`/dev/video2`), which failed with `EBUSY` or permission errors because `soos-daemon` held exclusive ownership of the camera.

---

## 2. Changes Made

### 2.1 Auto-Standby Disable in `crates/camera-v4l` (CLP1)
- In `crates/camera-v4l/src/v4l_impl.rs` and `crates/camera-v4l/src/mock.rs`:
  - Added `!self.config.idle_timeout.is_zero()` checks before evaluating `is_ready()` and in the supervisor loop (`open_and_stream()`).
  - When `idle_timeout` is `Duration::ZERO`, auto-standby is completely disabled: `is_ready()` unconditionally returns `true` once initialized, and the supervisor never yields `SupervisorAction::Suspend`.

### 2.2 Warmup Frames and Lockscreen Refresh in `crates/daemon` (CLP2, CLP4)
- In `crates/daemon/src/config.rs`:
  - When parsing `daemon.toml` (`from_toml_str`), `config.pipeline.camera.warmup_frames` defaults to `0` (`pipe.warmup_frames.unwrap_or(0)`), eliminating the 20-frame discard delay for instant wake.
  - Criterion C5 (default of 20 frames) is preserved in `DaemonConfig::default()`.
  - Added support for `idle_timeout_secs = 0` mapping cleanly to `Duration::ZERO`.
- In `crates/daemon/src/dispatcher.rs`:
  - Increased `PreviewFrame` wake loop timeout to dynamic budget (up to 1000ms), giving the camera sufficient time to initialize on initial wake.
  - For display manager / lockscreen services (`req.service.contains("gdm") || req.service.contains("lock") || req.service.contains("screen")`), added `pipe.camera.notify_activity()` to keep the camera warm during screen interaction.

### 2.3 Persistent Connection Streaming in `soos-daemon` (CLP3)
- In `crates/daemon/src/dispatcher.rs`:
  - Converted `handle_connection` into a persistent loop processing sequential requests on the same stream until clean client disconnect (`UnexpectedEof`) or connection timeout.
  - Used `requests_processed = requests_processed.saturating_add(1)` to satisfy clippy arithmetic bounds while tracking connection state.
  - Clients streaming preview frames (e.g. `soos-gui`) can now reuse a single connection indefinitely without connection churn or `Broken pipe` errors.

### 2.4 GUI IPC Camera Manager & Socket Probe (CLP4)
- In `crates/gui/src/ipc_camera.rs`:
  - Streamlined `IpcCameraManager::probe` to instantly verify socket connectivity (`UnixStream::connect(socket_path).is_ok()`) without introducing frame acquisition delay.
  - Streaming worker continuously requests frames at ~30 FPS over the persistent connection.
- In `crates/gui/src/main.rs`:
  - Ensured that whenever `/run/soos/daemon.sock` is present, `soos-gui` connects exclusively through `IpcCameraManager`, avoiding V4L2 device collisions with the daemon.

---

## 3. Verification & Test Evidence

### 3.1 Unit & Contractual Tests
- `crates/camera-v4l/tests/mock_camera_tests.rs`:
  - `test_mock_camera_idle_timeout_zero_disables_auto_standby` (CLP1) — PASS
- `crates/daemon/tests/config_tests.rs`:
  - `test_daemon_toml_default_warmup_frames_is_zero` (CLP2) — PASS
  - `test_pipeline_config_idle_timeout_zero_from_toml` (CLP1) — PASS
  - `test_pipeline_default_sensor_preference_is_prefer_ir` — PASS
- `crates/daemon/tests/dispatcher_tests.rs`:
  - `test_dispatcher_persistent_stream_multiple_requests` (CLP3) — PASS
  - `test_dispatcher_preview_frame_roundtrip` (CLP4) — PASS
  - `test_dispatcher_timeout_on_idle_connection` — PASS
- `crates/gui/tests/layout_tests.rs`:
  - `test_ipc_camera_manager_receives_persistent_frames` (CLP3, CLP4) — PASS

### 3.2 Full Test Suite & Linters
- `cargo fmt --all -- --check`: Clean (0 diffs)
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: Clean (0 warnings)
- `cargo test --workspace`: 100% PASS (all unit, integration, and property tests across all workspace crates)
- `./scripts/candid_review.sh`: PASSED (all 7 audits green)

---

## 4. Acceptance Summary

| Criterion | Description | Status |
|-----------|-------------|--------|
| **CLP1** | `idle_timeout_secs = 0` disables camera auto-standby and suspension | ✅ Verified |
| **CLP2** | `daemon.toml` defaults `warmup_frames = 0` for instant wake | ✅ Verified |
| **CLP3** | Daemon dispatcher supports persistent stream request loops | ✅ Verified |
| **CLP4** | `soos-gui` receives continuous live preview frames without dropping stream | ✅ Verified |
