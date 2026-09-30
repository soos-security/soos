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
8. **Automatic Format Negotiation (Criterion C6)**: Discovers device capabilities via `VIDIOC_ENUM_FMT` and automatically negotiates capture format across preference priority `RGB24 -> YUYV -> NV12 -> MJPEG -> Grey` with graceful fallback. `plan_capture` classifies the opened node with `classify_sensor`; on an `Infrared` node auto negotiation prefers `Grey`, and every captured `Frame` is stamped with the node's `SensorType` (`Frame::sensor_type`, GitHub #169) so the IR PAD policy applies even when an IR node streams a colour format.
9. **Dual-Sensor Device Discrimination (Criterion C8)**: Distinguishes RGB color sensors from Infrared sensors (e.g. on ThinkPad dual-camera laptops) and selects according to `sensor_preference`. The code default is `SensorPreference::PreferIr` (`CameraConfig::default`, also the `soos-enroll` default); `PreferRgb` and `Any` are explicit overrides. Grey frames and every frame tagged `SensorType::Infrared` are subject to the format-aware PAD policy of `soos-vision` (GitHub #169, see "IR Sensors and Emitter Requirements" below).
10. **Device Re-Resolution (CSH1–CSH4, GitHub #151)**: `V4lCameraManager::spawn_with_resolver` takes a `DevicePathResolver` (any `Fn() -> Option<PathBuf> + Send + Sync`) that the supervisor consults once per backoff period after `DeviceNotFound` / `UnsupportedCapability`, so a camera re-enumerated under a new `/dev/videoN` (suspend, replug, boot race) is reopened without a restart. `V4lCameraManager::spawn` never substitutes the configured path. `stable_device_path` maps an enumerated node to its `/dev/v4l/by-id/...` link by delegating to the resolver's single bounded by-id scanner (`SystemCameraEnumerator::by_id_aliases`, `MAX_BY_ID_ENTRIES`).
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

    /// User-presentable lifecycle state (default: derived from `is_ready()`).
    fn status(&self) -> CameraStatus { /* Ready or Starting */ }
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

### `CameraStatus`, `CameraErrorKind` & `CameraStatusCell` (GitHub #155, CAM-07)
- `CameraError::kind()` classifies every error into a stable, detail-free `CameraErrorKind`:
  `DeviceNotFound` (`ENOENT`/`ENODEV`/`ENXIO`), `DeviceBusy` (`EBUSY`), `PermissionDenied`
  (`EACCES`/`EPERM`, including when wrapped in the generic `Io` variant), `UnsupportedDevice`
  (capabilities or format negotiation), `Starved`, `Io`, plus `Source*` kinds for remote frame
  sources such as the `soos-daemon` preview proxy (`SourceUnreachable`, `SourceUnauthorized`,
  `SourceRateLimited`, `SourceUnavailable`, `SourceProtocol`).
- `CameraStatus` is `Starting | Ready | Suspended | Stopped | Error { kind, failures }`, where
  `failures` counts consecutive failed attempts of the same kind (saturating) and restarts after
  a successful streaming session.
- `V4lCameraManager` records every supervisor failure in a `CameraStatusCell` (the existing
  `warn!` log is kept), reports `Suspended` during auto-standby and `Stopped` after shutdown.
  `MockCameraManager::status()` reports injected faults (`set_error`, `set_starved`).
- The status carries no frame data, device contents or biometric material.

### `CameraConfig` & `CameraConfigBuilder`
Configures:
- `device_path`: Path to video device (`/dev/v4l/by-id/...`)
- `width` & `height`: Frame resolution (default: 640×480)
- `format`: Pixel format (`PixelFormat::Yuyv`, `Rgb24`, `Grey`, `Mjpeg`, `Nv12`) (default: `Yuyv`)
- `auto_format`: Automatic priority-based format negotiation (default: true)
- `sensor_preference`: Dual-sensor device preference (`SensorPreference::PreferRgb`, `PreferIr`, `Any`) (default: `PreferIr`)
- `fps`: Full streaming frame rate (default: 30), requested from the driver with `VIDIOC_S_PARM`
  (GitHub #193) through `capture::apply_frame_rate`, which takes the ioctl as a closure; the
  granted interval is logged, the poll timeout follows the granted rate, and a driver without
  frame-interval control keeps its default rate (`FrameRateOutcome::Refused`, a warning, never
  fatal)
- `idle_fps`: Publication rate once more than half of `idle_timeout` has elapsed (default: 5).
  It throttles frame publication only (`CameraConfig::publish_fps`, identical in the mock and the
  V4L2 manager); the hardware keeps streaming at `fps` until auto-standby
- `idle_timeout`: Inactivity before auto-standby releases the device (default: 10s);
  `Duration::ZERO` disables both auto-standby and the idle throttle
- `warmup_frames`: Discarded startup frames (default: 20). This is the library default: `soos-daemon` does not use this default: it runs with `DAEMON_DEFAULT_WARMUP_FRAMES` (0, `crates/daemon/src/config.rs`) whether or not `/etc/soos/daemon.toml` exists, unless `[pipeline] warmup_frames` is set (GitHub #205, `Docs/DAEMON.md` §1.4)
- `min_backoff` & `max_backoff`: Error backoff limits (default: 100ms to 5s)

These are the `CameraConfig::default()` / `CameraConfigBuilder` values. `soos-daemon` builds its
camera configuration from `/etc/soos/daemon.toml`: a `[pipeline]` section without `warmup_frames`
discards **0** frames (instant wake, matrix CLP2) and `idle_timeout_secs` overrides `idle_timeout`.
The page is checked against the code by `soos-invariants::camera_docs_contract` (GitHub #197).

### Shared Camera Resolver (`resolver.rs`, GitHub #152)

`soos-daemon`, `soos-enroll` and `soos-gui` (direct mode) all resolve the device through
`resolve_camera_device(explicit, preference, &dyn CameraEnumerator) -> CameraResolution`, so that
enrollment and authentication always use the same sensor:

1. An explicit path (anything but the sentinels `""`, `auto`, `default` and
   `AUTO_CAMERA_DEVICE` = `/dev/v4l/by-id/default-camera`, see `is_auto_camera_device`) is returned
   verbatim (`CameraResolutionSource::Explicit`). Callers order their explicit sources
   (CLI flag, then `[pipeline] camera_device`).
2. Otherwise the enumerated V4L2 capture nodes are ranked by `select_camera_device` with the
   `SensorPreference` (default `PreferIr`; `parse_sensor_preference` accepts
   `prefer_ir`/`ir`, `prefer_rgb`/`rgb`, `any`). The selected node is reported through its
   `/dev/v4l/by-id/` alias when one exists (Criterion C4). Aliases are only matched against capture
   nodes, so a metadata node (`...-video-index1`) is never selected (`AutoDetected`).
3. Without any capture node the sentinel is returned (`Fallback`).

`SystemCameraEnumerator` reads `/sys/class/video4linux` and at most `MAX_BY_ID_ENTRIES` (64)
by-id aliases (dangling aliases skipped); tests inject a hermetic `CameraEnumerator`.
`CameraConfig::explicit_device()` returns `None` for a sentinel `device_path`.

### Capture-Path Validation (`capture.rs`, GitHub #192, #193, #194)

- **Driver-returned format.** After `VIDIOC_S_FMT`, `validate_negotiated_format` reads the
  returned fourcc, width, height and `bytesperline`. A substituted fourcc is adopted (warning, and
  frames are labelled with it) only when its layout is known: `RGB3`, `YUYV`, `NV12`, `MJPG`,
  `GREY`/`Y800`/`Y8  `. `BGR3`, `RGB4` or any other fourcc fails as `CameraError::SetFormat`
  (`UnsupportedDevice`). A stride below one row, zero dimensions and odd NV12 dimensions are
  rejected; `bytesperline == 0` means tight rows.
- **Captured buffers.** `validate_captured_buffer` skips `V4L2_BUF_FLAG_ERROR` buffers, rejects a
  `bytesused` larger than the mapping, rejects uncompressed buffers shorter than
  `(rows - 1) * stride + row_bytes` and removes row padding, so every published frame has exactly
  `PixelFormat::expected_buffer_size` bytes; an MJPEG frame is its `bytesused` prefix and an empty
  one is rejected. `MAX_CONSECUTIVE_REJECTED_FRAMES` (30) consecutive rejections end the stream
  with `BufferDequeue`, so the supervisor backs off and reopens the device instead of silently
  dropping frames.
- **Bounded DQBUF wait (Criterion C9).** `dqbuf_poll_timeout(fps)` is three frame intervals
  clamped to `MIN_DQBUF_POLL_TIMEOUT`..`MAX_DQBUF_POLL_TIMEOUT` (150-250 ms), computed from the
  granted rate. The capture thread polls the device with it and re-checks its shutdown flag after
  every poll, so `Drop` waits for at most one poll (< `C9_SHUTDOWN_BUDGET`, 500 ms). Consecutive
  timeouts covering `MAX_STREAM_STALL` (2 s, the previous single-poll tolerance) report
  `CameraError::Starved`. After a timeout or a signal interruption (`EINTR`) inside `v4l`'s
  `next()` (whose re-queued buffer is still owned by the driver) the loop dequeues and discards one
  buffer before calling `next()` again, so no buffer is queued twice and the device is not
  reopened. Only a timeout spends the stall budget: `EINTR` (in `next()`, `wait_ready` or the
  resync) never counts toward `Starved` (GitHub #285).
- **Deep-greyscale buffers (GitHub #195).** A driver-returned `Y8I`/`Y10`/`Y12`/`Y16` format is
  validated by `validate_deep_grey_format` (2 bytes per pixel, wire stride kept) and every buffer by
  `validate_deep_grey_buffer`, which applies the same error-flag, `bytesused` and short-buffer
  checks before `DeepGreyFormat::to_grey8` produces the packed `Grey` payload.

### Hermetic Node Enumeration (`sensor.rs`, GitHub #198)

`enumerate_capture_devices()` is a thin wrapper over
`enumerate_capture_devices_with(sysfs_dir, dev_dir, &dyn V4lNodeProbe)`:

- only `video<decimal>` entries of `/sys/class/video4linux` (`SYSFS_VIDEO4LINUX_DIR`) are
  considered, in numeric order (`video2` before `video10`); sub-devices, radio and VBI nodes are
  never opened;
- the scan is bounded: at most `MAX_SYSFS_ENTRIES` (256) entries are read and at most
  `MAX_VIDEO_NODES` (64) nodes are probed;
- a node is listed only if the probe reports `V4L2_CAP_VIDEO_CAPTURE` **and** at least one
  decodable pixel format (deep-greyscale IR fourccs count as `Grey`, GitHub #195), so uvcvideo metadata nodes, codec nodes and nodes that cannot be opened
  (`EACCES`, `ENODEV`) are skipped;
- `SystemV4lNodeProbe` issues the real `VIDIOC_QUERYCAP` / `VIDIOC_ENUM_FMT` ioctls; tests inject
  a fixture-table probe (`enumeration_tests.rs`).

When the auto-selected node has no `/dev/v4l/by-id/` alias, the resolver returns the
`/dev/videoN` node itself (ADR 2026-09-30 "Hermetic V4L2 Enumeration and `/dev/videoN`
Auto-Selection").

### Opt-in Hardware Smoke Tests

`tests/hardware_smoke_tests.rs` scans, resolves and streams one frame from the real camera. The
tests are `#[ignore]`d and also return early unless `SOOS_HW_TESTS=1`:

```bash
SOOS_HW_TESTS=1 cargo test -p soos-camera-v4l --test hardware_smoke_tests -- --ignored
```

### Busy-Device Classification (GitHub #150)

A uvcvideo node streamed by another process opens successfully and only fails at
`VIDIOC_S_FMT` / `VIDIOC_REQBUFS` / `VIDIOC_DQBUF` with `EBUSY`. `CameraError::from_ioctl_error`
classifies `EBUSY` as `DeviceBusy` and `ENODEV`/`ENOENT` as `DeviceNotFound` (raw OS error kept);
the supervisor logs a dedicated "held by another process" warning (`CameraError::is_device_busy`).

### Sensor Classification Hints and Deep-Greyscale IR Formats (GitHub #195, CAM-13)

The V4L2 card name is capped at 31 bytes, and the RGB and IR nodes of one laptop camera often
report the **same** truncated name (for example `Integrated Camera: Integrated C`). IR modules that
also advertise YUYV used to be classified RGB, and IR modules exposing only deep-greyscale
fourccs were dropped from enumeration. Classification now uses an ordered scorer,
`classify_sensor_with_hints(card_name, formats, &SensorHints)` (strongest signal first):

1. a whole `IR` token (or `infrared`) in the `/dev/v4l/by-id/` link name
   (`SensorHints::by_id_name`), which udev builds from the untruncated USB product string;
2. an IR marker in the card name (the historic markers, the truncated `" I"` suffix, or an `IR`
   token such as `Integrated_IR_Camera`);
3. a non-empty format list without any colour format (greyscale only);
4. the IR frame-size signature `is_ir_frame_size_signature`: every enumerated size is at most
   `IR_SIGNATURE_MAX_WIDTH` x `IR_SIGNATURE_MAX_HEIGHT` (640x400, e.g. 340x340, 400x400,
   640x360); any VGA or larger size rules it out;
5. otherwise any colour format means `Rgb`, and nothing at all means `Unknown`.

`classify_sensor(card, formats)` is the same scorer with empty hints (unchanged results).
`resolve_camera_device` builds the hints of every node (by-id alias name, and
`CameraEnumerator::frame_sizes`, a provided trait method that returns no sizes by default;
`SystemCameraEnumerator` enumerates `VIDIOC_ENUM_FRAMESIZES`, bounded by `MAX_FRAME_SIZE_HINTS`
= 64, stepwise ranges contribute their maximum only) and selects with
`select_camera_device_with`. The capture supervisor classifies the opened node with
`plan_capture_with_hints`, but its hints are narrower than the resolver's: the by-id name is
taken only when the configured `device_path` is itself a `/dev/v4l/by-id/` link (always the case
for an auto-resolved node that udev aliased), and the frame sizes are enumerated from the opened
node. When the configured path is a `/dev/videoN` node, no by-id alias is looked up, so the
classification rests on the card name, the formats and the frame sizes only; an IR node that
streams a colour format under an ordinary card name can then be stamped `SensorType::Rgb`. Use the
by-id path (or leave `camera_device` unset) for such modules.

Deep-greyscale fourccs (`deep_grey.rs`): `Y8I` (interleaved stereo, left sensor kept), `Y10`,
`Y12` and `Y16` (little-endian 16-bit containers) are mapped by `delivered_formats` to
`PixelFormat::Grey`, so such a node is enumerated (`capture_device_from_probe`) and classified
Infrared. When `Grey` is negotiated and the node has no native 8-bit greyscale fourcc,
`select_wire_format` requests the deep format (priority `Y16 > Y12 > Y10 > Y8I`) and every buffer
is normalised to packed 8-bit greyscale by `DeepGreyFormat::to_grey8` (honours `bytesperline`,
drops a truncated buffer) before a `Frame` is published. `PixelFormat` itself is unchanged.

Not covered hermetically: `bus_info`, `driver` and `device_caps` are not used, because both nodes
of one USB camera share them; real-hardware validation of the frame-size signature is a follow-up.

---

## IR Sensors and Emitter Requirements (GitHub #169)

`soos` does **not** drive the IR emitter: it only reads frames from the V4L2 node. On an IR
sensor the capture is usable only if the near-infrared (typically 850 or 940 nm) emitter is lit
while frames are captured.

- **Emitter must be active during capture.** Many UVC IR cameras only enable their emitter through a
  vendor UVC extension-unit control that the stock `uvcvideo` driver does not set. Configure it
  outside `soos` (for example with the community tool `linux-enable-ir-emitter`) and verify with
  `soos-gui` or a V4L2 viewer that the face is visibly lit in the IR stream.
- **Unlit crops are rejected, never accepted.** Without IR illumination the PAD crop is dark or
  flat; the IR gate of `soos-vision` rejects it (`IrLivenessGateFailed`: `Underexposed`,
  `LowContrast` or `LowTexture`) and the daemon treats it as a spoof capture, so the request ends
  in `Deny` / `PadFailed` and PAM falls back to the password. A missing emitter therefore degrades
  availability, not security.
- **Strobing emitters.** Emitters that light only alternate frames can produce unlit captures in
  which a face is still detected; such a capture vetoes the whole request (fail-closed). Prefer a
  steady emitter mode, or select the RGB sensor (`sensor_preference = "rgb"` in the daemon
  `[pipeline]` section) until the device is characterized.
- **Liveness on IR is uncalibrated.** MiniFASNetV2 is RGB-trained; on Grey frames the pipeline
  applies a stricter, conservative threshold (`DEFAULT_IR_PAD_THRESHOLD = 0.95`) whose value is not
  derived from measurements. Calibration on captured IR frames, or running PAD on the RGB sibling
  sensor / an IR-trained model, is a hardware follow-up (GitHub #172 / PAD-06).

## Verification Matrix Mapping

| Matrix ID | Criterion | Verification Method | Status |
|---|---|---|---|
| **C1** | `mock-camera` feature provides functional `MockCameraManager` | `mock_camera_tests::test_mock_camera_generates_frames_and_readiness` | Validated |
| **C2** | Fresh frame available in < 5ms via `ArcSwap` | `bench_latency_tests::test_arcswap_frame_retrieval_latency_under_5ms` | Validated |
| **C3** | Handles `ENODEV`, `EIO`, `EBUSY` without panic | `error_recovery_tests::test_error_recovery_enodev_without_panic` | Validated |
| **C4** | Hardware selection by `/dev/v4l/by-id/` rather than index | `config_hardware_tests::test_config_by_id_path_selection` | Validated |
| **C5** | Drops the first `warmup_frames` frames after startup for auto-exposure (20 in `CameraConfig::default()`; a daemon.toml `[pipeline]` section without the key sets 0, matrix CLP2) | `warmup_tests::test_warmup_frames_discard_before_ready` | Validated |
| **C6** | Priority format negotiation (`RGB24 -> YUYV -> NV12 -> MJPEG -> Grey`) | `format_negotiation_tests::test_format_negotiation_prefers_rgb24` | Validated |
| **C7** | Graceful hot-unplug recovery on `ENODEV` | `hotunplug_tests::test_camera_hotunplug_recovery` | Validated |
| **C8** | Dual-sensor discrimination (RGB vs IR preference) | `dual_sensor_tests::test_dual_sensor_prefers_rgb` | Validated |
| **CSR1–CSR2** | Single shared camera resolver (GitHub #152) | `resolver_tests::*` | Verified |
| **CCP1–CCP4** | Ordered sensor scorer and deep-greyscale IR formats (GitHub #195) | `sensor_hint_classification_tests::*` | Verified |
| **CSR5** | `EBUSY`/`ENODEV` from capture ioctls classified (GitHub #150) | `error_recovery_tests::test_set_format_ebusy_maps_to_device_busy` | Verified |
| **CSH1–CSH4** | Re-resolution after `ENODEV`, bounded pacing, no silent substitution, by-id addressing | `supervision_tests::test_supervisor_reresolves_device_after_enodev` (and siblings) | ✅ Verified |
| **CSH5–CSH6** | Truthful `CameraHealth`, standby vs failure, panic → `Dead` | `supervision_tests::test_supervisor_panic_marks_camera_dead` (and siblings) | ✅ Verified |
| **GRE5** | `CameraError::kind()` classification and `CameraManager::status()` reporting | `camera_status_tests::*` | ✅ Verified |
| **CHT1–CHT3** | Hermetic enumeration: capture-only, non-empty formats, numeric order, bounded scan, IR alias or `/dev/videoN` (GitHub #198) | `enumeration_tests::test_enumerate_filters_empty_format_nodes` (and siblings) | ✅ Verified |
