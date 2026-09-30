# `soos-camera-v4l` Crate — V4L2 MMAP Capture Manager

## Overview

The `soos-camera-v4l` crate provides warm camera streaming and snapshot capabilities for `soos-daemon`. In facial recognition authentication, camera initialization latency (100–300ms) would violate the 150ms real-time authentication budget. To circumvent this, the camera manager continuously captures frames in a dedicated background OS thread, updating an atomic `ArcSwapOption<Frame>` snapshot in RAM.

When an authentication request arrives, consumer vision pipelines query `latest_frame()`, retrieving an $O(1)$ lock-free snapshot in under 100 microseconds without touching camera hardware or kernel MMAP buffers directly.

---

## Architectural Principles

1. **Daemon Exclusive Ownership**: Only `soos-daemon` opens `/dev/video*`. The PAM module (`pam_soos.so`) never links this crate or accesses video devices.
2. **Persistent Hardware Addressing (Criterion C4)**: Defaults to `/dev/v4l/by-id/...` stable paths, avoiding numerical device index races (`/dev/video0`) upon system restart.
3. **Lock-Free RAM Snapshotting (Criterion C2)**: Background capture publishes frames using `arc_swap::ArcSwapOption`, guaranteeing < 5ms p95 retrieval latency.
4. **Auto-Exposure Warmup Discard (Criterion C5)**: Discards the first 15–30 frames (default 20) during startup to let sensor auto-exposure and white balance stabilize before advertising `is_ready() == true`.
5. **Idle Power Management**: Automatically drops capture rate to 5 FPS after 60 seconds of inactivity to conserve CPU and camera thermals. Restores full capture rate immediately upon `notify_activity()`.
6. **Error Recovery with Bounded Backoff (Criterion C3 & C7)**: Handles `ENODEV`, `EBUSY`, and `EIO` without crashing. Uses exponential backoff (100ms → 200ms → 400ms → cap at 5,000ms) with fail-closed availability reporting. Seamlessly re-initializes and warms up when a hot-unplugged device is reconnected.
7. **Hardware-Free Mocking (Criterion C1)**: Provides `MockCameraManager` behind the `mock-camera` feature flag for testing in headless CI and Docker environments.
8. **Automatic Format Negotiation (Criterion C6)**: Discovers device capabilities via `VIDIOC_ENUM_FMT` and automatically negotiates capture format across preference priority `RGB24 -> YUYV -> NV12 -> MJPEG -> Grey` with graceful fallback.
9. **Dual-Sensor Device Discrimination (Criterion C8)**: Distinguishes RGB color sensors from Infrared sensors (e.g. on ThinkPad dual-camera laptops) and selects RGB by default, while supporting explicit configuration overrides.
10. **Device Re-Resolution (CSH1–CSH4, GitHub #151)**: `V4lCameraManager::spawn_with_resolver` takes a `DevicePathResolver` (any `Fn() -> Option<PathBuf> + Send + Sync`) that the supervisor consults once per backoff period after `DeviceNotFound` / `UnsupportedCapability`, so a camera re-enumerated under a new `/dev/videoN` (suspend, replug, boot race) is reopened without a restart. `V4lCameraManager::spawn` never substitutes the configured path. `stable_device_path` maps an enumerated node to its `/dev/v4l/by-id/...` link.
11. **Truthful Lifecycle Health (CSH5–CSH6, GitHub #153)**: `CameraManager::health()` returns a `CameraHealth` (`Starting`, `Streaming`, `Standby`, `Recovering`, `Dead`). Unlike `is_ready()`, it distinguishes an idle auto-standby (healthy, `is_operational() == true`) from a missing/busy device. The supervisor runs inside `catch_unwind`; a panic marks the camera `Dead`, withdraws frames and is never restarted (fail-closed).

---

## Public API

### `CameraManager` Trait
```rust
pub trait CameraManager: Send + Sync {
    /// Lock-free lookup of the latest stabilized video frame.
    fn latest_frame(&self) -> Option<Arc<Frame>>;

    /// Returns true if the camera is healthy and warmup has finished.
    fn is_ready(&self) -> bool;

    /// Wakes the camera from idle power saving (5 FPS) to full FPS.
    fn notify_activity(&self);

    /// Requests shutdown of background capture threads.
    fn stop(&self);

    /// Lifecycle state for health reporting (default derived from `is_ready()`).
    fn health(&self) -> CameraHealth;
}
```

### Daemon Health Integration
`soos-daemon` attaches its camera manager to `HealthState` (`HealthState::attach_camera`);
`camera_ready` in every `StatusResponse` / `soos-admin status` is then
`camera.health().is_operational()`, evaluated at snapshot time. The daemon also logs every camera
state transition (500 ms poll). In auto-selection mode (`device_path` left at the
`/dev/v4l/by-id/default-camera` sentinel) the supervisor re-enumerates on device loss; an explicit
`device_path` is retried as-is, so a by-id link that udev creates late is picked up without the
daemon switching to another camera.

### `CameraConfig` & `CameraConfigBuilder`
Configures:
- `device_path`: Path to video device (`/dev/v4l/by-id/...`)
- `width` & `height`: Frame resolution (default: 640×480)
- `format`: Pixel format (`PixelFormat::Yuyv`, `Rgb24`, `Grey`, `Mjpeg`, `Nv12`)
- `auto_format`: Automatic priority-based format negotiation (default: false)
- `sensor_preference`: Dual-sensor device preference (`SensorPreference::PreferRgb`, `PreferIr`, `Any`)
- `fps`: Full streaming frame rate (default: 30)
- `idle_fps`: Throttled power-saving rate (default: 5)
- `idle_timeout`: Duration of inactivity before throttling (default: 60s)
- `warmup_frames`: Discarded startup frames (default: 20)
- `min_backoff` & `max_backoff`: Error backoff limits (default: 100ms to 5s)

---

## Verification Matrix Mapping

| Matrix ID | Criterion | Verification Method | Status |
|---|---|---|---|
| **C1** | `mock-camera` feature provides functional `MockCameraManager` | `mock_camera_tests::test_mock_camera_generates_frames_and_readiness` | Validated |
| **C2** | Fresh frame available in < 5ms via `ArcSwap` | `bench_latency_tests::test_arcswap_frame_retrieval_latency_under_5ms` | Validated |
| **C3** | Handles `ENODEV`, `EIO`, `EBUSY` without panic | `error_recovery_tests::test_error_recovery_enodev_without_panic` | Validated |
| **C4** | Hardware selection by `/dev/v4l/by-id/` rather than index | `config_hardware_tests::test_config_by_id_path_selection` | Validated |
| **C5** | Drops first 15–30 frames after startup for auto-exposure | `warmup_tests::test_warmup_frames_discard_before_ready` | Validated |
| **C6** | Priority format negotiation (`RGB24 -> YUYV -> NV12 -> MJPEG -> Grey`) | `format_negotiation_tests::test_format_negotiation_prefers_rgb24` | Validated |
| **C7** | Graceful hot-unplug recovery on `ENODEV` | `hotunplug_tests::test_camera_hotunplug_recovery` | Validated |
| **C8** | Dual-sensor discrimination (RGB vs IR preference) | `dual_sensor_tests::test_dual_sensor_prefers_rgb` | Validated |
| **CSH1–CSH4** | Re-resolution after `ENODEV`, bounded pacing, no silent substitution, by-id addressing | `supervision_tests::test_supervisor_reresolves_device_after_enodev` (and siblings) | ✅ Verified |
| **CSH5–CSH6** | Truthful `CameraHealth`, standby vs failure, panic → `Dead` | `supervision_tests::test_supervisor_panic_marks_camera_dead` (and siblings) | ✅ Verified |
