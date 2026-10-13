# `soos-frame` Crate — Pure Camera Frame Types

## 1. Purpose

`soos-frame` (`crates/frame`) holds the camera frame types shared by the capture backends and the vision pipeline:
`Frame`, `PixelFormat` and `SensorType`. They were moved verbatim out of `soos-camera-v4l` (GitHub #367, ADR
"[2026-10-13] macOS Port in a Separate Repository") so that `soos-vision` and the other portable crates no longer
depend on `v4l` and can build on macOS for the separate `soos-macos` repository (`Docs/MACOS_PORT.md`).

The crate is a business crate: `#![forbid(unsafe_code)]`, no I/O, no logging, a single normal dependency (`zeroize`).

## 2. Public API

| Item | Behaviour (unchanged by the move) |
|---|---|
| `PixelFormat` (`Yuyv`, `Rgb24`, `Grey`, `Mjpeg`, `Nv12`) | `fourcc_str()` (`YUYV`, `RGB3`, `GREY`, `MJPG`, `NV12`); `expected_buffer_size(w, h)` saturating, `None` for MJPEG |
| `SensorType` (`Rgb`, `Infrared`, `Unknown`) | sensor tag carried by every frame; the IR PAD policy of `soos-vision` keys on it |
| `Frame { data, width, height, timestamp_mono_ns, format, sequence, sensor_type }` | `new` (sensor `Unknown`), `with_sensor_type`, `age_ms` (saturating), `is_stale` (strict `>`) |
| `impl Zeroize for Frame` | wipes `data` (the vector is cleared, its whole capacity zeroed) |
| `impl Drop for Frame` | wipes `data` on drop |

`Frame` derives `Debug`, which prints the buffer bytes: never format a whole `Frame` in a log (no current log site
does; a redacted `Debug` is a possible follow-up).

## 3. Compatibility

`soos-camera-v4l` re-exports the three types (`pub use soos_frame::{Frame, PixelFormat};` in `src/frame.rs`,
`pub use soos_frame::SensorType;` in `src/sensor.rs`), so `soos_camera_v4l::Frame`, `soos_camera_v4l::frame::*`,
`soos_camera_v4l::sensor::SensorType` and the crate-internal `crate::frame::*` paths name the same types. The daemon,
GUI and CLIs are unchanged. `soos-vision` and `soos-test-fixtures` depend on `soos-frame` directly.

IR-only types added later (IR anti-spoofing, #353) go behind a non-default `ir` cargo feature, so an RGB-only
consumer never compiles them. No feature exists today.

## 4. Tests

- `crates/frame/tests/frame_contract_tests.rs`: FourCC strings, buffer sizes and saturation, constructor defaults,
  age and staleness, zeroize semantics, derives.
- `crates/camera-v4l/tests/frame_drop_wipe_tests.rs`: a test-only allocator probe proves that dropping a `Frame` wipes
  its whole buffer capacity, with a non-vacuity twin, and that the camera types are the `soos-frame` types.
- `tests/invariants/src/portable_crates_contract.rs`: purity, single definition, dependency guard.

## 5. Verification Matrix Mapping

`MAC2` (extraction and wipe on drop), `MAC3`/`MAC4` (dependency guard).
