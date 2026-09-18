# Walkthrough 38: Automatic Format Negotiation and NV12 Support

## 1. Overview & Architectural Motivation

Backlog Issue #22 (GitHub Issue #61) resolves hardware diversity and sensor adaptability challenges across Linux laptops and USB webcams:
1. **NV12 Pixel Format**: Adds native support for the widely-used YUV 4:2:0 bi-planar format (`NV12`), commonly output by modern integrated webcams and hardware encoders.
2. **Automatic Format Negotiation (Criterion C6)**: Probes device capabilities via `VIDIOC_ENUM_FMT` and deterministically selects the optimal capture format adhering to the priority order:
   $$\text{RGB24} \longrightarrow \text{YUYV} \longrightarrow \text{NV12} \longrightarrow \text{MJPEG} \longrightarrow \text{Grey}$$
   with graceful fallback if a configured format is unsupported.
3. **Hot-Unplug Resilience (Criterion C7)**: Transparently handles device disconnection (`ENODEV`), entering bounded exponential backoff and reinitializing when the hardware node reappears.
4. **Dual-Sensor Device Discrimination (Criterion C8)**: Intelligently classifies and distinguishes standard RGB color cameras from Infrared (IR) sensors, defaulting to RGB while supporting configuration overrides.

---

## 2. Key Changes by Component

### `crates/camera-v4l`
- [`frame.rs`](file:///home/hadrien/Project/soos/crates/camera-v4l/src/frame.rs):
  - Added `PixelFormat::Nv12` variant.
  - Implemented FourCC mapping (`"NV12"`).
  - Implemented uncompressed buffer size calculation: `(width * height * 3) / 2`.
- [`error.rs`](file:///home/hadrien/Project/soos/crates/camera-v4l/src/error.rs):
  - Added `CameraError::NoSupportedFormats` and `CameraError::NoCompatibleFormat`.
- [`sensor.rs`](file:///home/hadrien/Project/soos/crates/camera-v4l/src/sensor.rs):
  - Added `SensorType` (`Rgb`, `Infrared`, `Unknown`) and `SensorPreference` (`PreferRgb`, `PreferIr`, `Any`).
  - Added `CameraDeviceInfo` metadata abstraction.
  - Implemented `classify_sensor(card_name, supported_formats)` and `select_camera_device(devices, preference)`.
- [`config.rs`](file:///home/hadrien/Project/soos/crates/camera-v4l/src/config.rs):
  - Added `auto_format: bool` (default: `false` preserving backward compatibility) and `sensor_preference: SensorPreference` (default: `PreferRgb`).
  - Added builder methods `.auto_format(bool)` and `.sensor_preference(SensorPreference)`.
- [`v4l_impl.rs`](file:///home/hadrien/Project/soos/crates/camera-v4l/src/v4l_impl.rs):
  - Exported `FORMAT_PRIORITY` and `negotiate_format()`.
  - Added FourCC mapping helpers `fourcc_to_pixel_format()` and `pixel_format_to_fourcc()`.
  - Updated `open_and_stream()` to query supported formats and set negotiated format on device.
  - Implemented `ENODEV` detection in `stream.next()` to fail closed and cleanly enter supervisor backoff.
- [`mock.rs`](file:///home/hadrien/Project/soos/crates/camera-v4l/src/mock.rs):
  - Added `warmup_remaining` reset on error injection to simulate hardware re-stabilization upon reconnection.

### `crates/vision`
- [`color.rs`](file:///home/hadrien/Project/soos/crates/vision/src/color.rs):
  - Added `PixelFormat::Nv12` branch to `convert_to_rgb()`.
  - Validated even dimensions (`!width.is_multiple_of(2) || !height.is_multiple_of(2)`) and buffer length `(width * height * 3) / 2`.
  - Implemented full-range fixed-point YUV420 bi-planar to RGB24 conversion matching the precision of YUYV.

---

## 3. Verification & Test Evidence

### Contractual Test Suites
1. **Format Negotiation (`crates/camera-v4l/tests/format_negotiation_tests.rs`)**:
   - `test_format_negotiation_prefers_rgb24`: Validates RGB24 preference over other formats.
   - `test_format_fallback_on_unsupported`: Validates graceful fallback to NV12 when RGB24/YUYV are unsupported.
   - `test_format_negotiation_all_priority_order`: Validates full priority cascade: `RGB24 -> YUYV -> NV12 -> MJPEG -> Grey`.
   - `test_format_negotiation_empty_fails`: Asserts fail-closed behavior on empty format list.
   - `test_format_negotiation_uses_preferred_when_supported`: Asserts configured format is used when supported.
2. **Hot-Unplug Recovery (`crates/camera-v4l/tests/hotunplug_tests.rs`)**:
   - `test_camera_hotunplug_recovery`: Validates `ENODEV` causes fail-closed `is_ready = false`, and camera automatically recovers when re-connected.
3. **Dual-Sensor Discrimination (`crates/camera-v4l/tests/dual_sensor_tests.rs`)**:
   - `test_dual_sensor_prefers_rgb`: Asserts RGB camera is selected over IR camera by default.
   - `test_dual_sensor_override_prefers_ir`: Asserts IR camera is selected when explicitly configured.
   - `test_sensor_classification_by_card_name`: Validates card name pattern matching.
   - `test_sensor_classification_by_formats`: Validates format-based sensor inference.
4. **NV12 Color Conversion (`crates/vision/tests/color_tests.rs`)**:
   - `test_nv12_to_rgb_conversion`: Validates 4x4 image conversion to 48-byte RGB24 buffer.
   - `test_nv12_known_reference_image`: Validates exact RGB channel values against known reference inputs.
   - `test_nv12_invalid_size_fails_closed`: Validates rejection of truncated or oversized buffers.
   - `test_nv12_odd_dimensions_rejected`: Validates rejection of odd width or height.

### Test Execution Summary
```text
running 26 tests in soos-camera-v4l ... ok (0 failed)
running 12 tests in soos-vision ... ok (0 failed)
running 12 tests in soos-invariants ... ok (0 failed)
```

All 100+ tests across the entire workspace monorepo pass cleanly with zero warnings (`cargo clippy -D warnings`).
