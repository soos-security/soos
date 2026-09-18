# Candid Review Report

- **Date**: 2026-09-18
- **Target Branch**: `feat/camera-format-negotiation`
- **Base Reference**: `origin/main`
- **Audited Files**:
  - `crates/camera-v4l/src/config.rs`
  - `crates/camera-v4l/src/error.rs`
  - `crates/camera-v4l/src/frame.rs`
  - `crates/camera-v4l/src/lib.rs`
  - `crates/camera-v4l/src/mock.rs`
  - `crates/camera-v4l/src/sensor.rs`
  - `crates/camera-v4l/src/v4l_impl.rs`
  - `crates/camera-v4l/tests/dual_sensor_tests.rs`
  - `crates/camera-v4l/tests/format_negotiation_tests.rs`
  - `crates/camera-v4l/tests/hotunplug_tests.rs`
  - `crates/vision/src/color.rs`
  - `crates/vision/tests/color_tests.rs`
  - `scripts/sync_issue.py`

## 1. Executive Summary
Audit of proposed changes for Backlog Issue #22 (Automatic format negotiation and NV12 support).
The changes implement the `NV12` pixel format with fixed-point RGB24 conversion, priority-based format negotiation (`RGB24 -> YUYV -> NV12 -> MJPEG -> Grey`), hot-unplug recovery on `ENODEV`, and dual-sensor device classification (RGB vs IR preference).
All changes conform strictly to architectural invariants, maintain zero unsafe code in business crates, introduce zero unwrap/expect in production pathways, and preserve 100% test contract integrity.

## 2. Deep Reasoning Audit

### Logic & Architecture
- [PASS]: State transitions and protocol boundaries are sound.
- Format negotiation follows strict priority order `[PixelFormat::Rgb24, PixelFormat::Yuyv, PixelFormat::Nv12, PixelFormat::Mjpeg, PixelFormat::Grey]`.
- Empty supported format list correctly fails closed with `CameraError::NoSupportedFormats`.
- Sensor classification deterministically prioritizes RGB sensors while allowing configuration override to IR.
- NV12 conversion properly checks for even dimensions and bounds check for buffer size `(width * height * 3) / 2`.

### PAM Concurrency & Deadlines
- [PASS]: The changes operate within `camera-v4l` background capture thread and `vision` pure conversion.
- Zero asynchronous runtime (Tokio) is introduced in PAM pathways.
- The PAM module consumes lock-free RAM snapshots via `ArcSwapOption<Frame>` with zero contention.
- Zero standard output or error stream pollution (`println!`, `eprintln!`, `dbg!`).

### Panic Safety & Fallback
- [PASS]: Zero `unwrap()`, `expect()`, or `panic!()` in production code.
- Hot-unplug `ENODEV` error from `stream.next()` is trapped and mapped to `CameraError::DeviceNotFound`, resetting `is_ready` to `false` and clearing `latest_frame`.
- Background supervisor initiates bounded exponential backoff and transparently reinitializes when the device reappears.

### Test Integrity & Anti-Weakening
- [PASS]: Pre-existing test contracts are completely preserved with zero test weakening.
- Comprehensive new test contracts covering all 4 sub-issues:
  - `format_negotiation_tests.rs` (5 tests)
  - `hotunplug_tests.rs` (1 test)
  - `dual_sensor_tests.rs` (4 tests)
  - `color_tests.rs` (NV12 conversion, known reference, invalid buffer size, odd dimensions)
- All tests pass cleanly across the workspace.

### Memory & Secret Bounds
- [PASS]: Memory allocations are bounded by uncompressed frame dimensions.
- Sensitive frame buffers implement `Zeroize`.
- Zero credentials, embeddings, or raw frame dumps in log messages.

## 3. Detailed Findings & Action Items
- Zero blocking issues or invariant violations found.

## 4. Final Verdict
**VERDICT: APPROVED**
