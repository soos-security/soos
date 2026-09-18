# Plan Evaluation Report: Issue #22 — feat(camera-v4l): Automatic format negotiation and NV12 support

- **Evaluator**: Independent Plan Evaluator Sub-Agent (dev-workflow / plan-evaluator)
- **Target Issue**: Backlog Issue #22 (GitHub Issue #61)
- **Target Branch**: `feat/camera-format-negotiation`
- **Date**: 2026-09-18
- **Evaluated Scope**:
  - `crates/camera-v4l`: `frame.rs`, `error.rs`, `config.rs`, `sensor.rs`, `v4l_impl.rs`, `mock.rs`, `lib.rs`
  - `crates/vision`: `color.rs`
  - `tests`: `format_negotiation_tests.rs`, `hotunplug_tests.rs`, `dual_sensor_tests.rs`, `color_tests.rs`

---

## Evaluation Across 6 Core Architectural Pillars

### Pillar 1: Architectural Alignment & Threat Model
- **Evaluation**: The proposed design aligns with §6 ("Warm Camera Streaming & Low Latency") of `AI/ARCHITECTURE.md`.
- Exclusive hardware ownership remains confined to `soos-daemon` and `camera-v4l`.
- Format negotiation operates entirely within the isolated camera streaming thread; no IPC protocol changes are required, preserving binary message bounds and serialization integrity.
- Sensor classification ensures deterministic device selection on multi-camera hardware (such as laptops with RGB + IR modules).
- **Verdict**: PASS

### Pillar 2: PAM Real-Time Latency & Concurrency
- **Evaluation**: The changes do not touch the synchronous PAM module (`pam_soos.so`).
- The camera streaming thread operates independently in background MMAP capture mode, serving lock-free frame snapshots to PAM requests via `ArcSwapOption<Frame>` in < 5ms (Criterion C2).
- Zero asynchronous runtime (Tokio) is introduced in client pathways; standard thread supervisor and atomics are preserved.
- **Verdict**: PASS

### Pillar 3: Panic Safety & Fail-Closed Behavior
- **Evaluation**: All error conditions in format negotiation (`NoCompatibleFormat`, `NoSupportedFormats`), device capabilities querying, and frame buffer dequeuing return explicit `CameraError` or `VisionError` variants.
- Hot-unplug `ENODEV` detection gracefully flags `is_ready = false` and clears `latest_frame`, failing closed and preventing any unhandled panic or stale frame serving.
- Zero `unwrap()` or `expect()` introduced in production code.
- **Verdict**: PASS

### Pillar 4: Dependency Isolation & Banned Crates
- **Evaluation**: Zero unapproved or banned dependencies.
- Neither `opencv` nor `nokhwa` is used.
- Uses standard workspace dependencies: `v4l 0.14`, `arc-swap`, `zeroize`, `thiserror`, `tracing`, `libc`.
- `#![forbid(unsafe_code)]` remains strictly enforced in `crates/vision` and is preserved.
- **Verdict**: PASS

### Pillar 5: Data Confidentiality & Zeroization
- **Evaluation**: Frame buffers in `Frame` implement `Zeroize` and are bounded by explicit uncompressed dimensions.
- NV12 buffer allocation is bounded and verified against expected dimension formulas (`(width * height * 3) / 2`).
- No raw frames or biometric embeddings are logged in tracing or exposed in errors.
- **Verdict**: PASS

### Pillar 6: Test Integrity & TDD Contracts
- **Evaluation**: Pre-existing test contracts are completely preserved with zero test weakening.
- Comprehensive test contracts are designed for all 4 sub-issues:
  - Sub-issue #22.1: `test_nv12_to_rgb_conversion`, `test_nv12_known_reference_image`, `test_nv12_invalid_size_fails_closed`, `test_nv12_odd_dimensions_rejected`
  - Sub-issue #22.2: `test_format_negotiation_prefers_rgb24`, `test_format_fallback_on_unsupported`, `test_format_negotiation_all_priority_order`, `test_format_negotiation_empty_fails`
  - Sub-issue #22.3: `test_camera_hotunplug_recovery`
  - Sub-issue #22.4: `test_dual_sensor_prefers_rgb`, `test_dual_sensor_override_prefers_ir`, `test_sensor_classification_by_card_name`, `test_sensor_classification_by_formats`
- All tests will be authored in Phase 2 (Tester Agent) BEFORE production code and verified to fail (TDD Red).
- **Verdict**: PASS

---

## Conclusion & Formal Validation Verdict

All criteria have been rigorously evaluated and conform to `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, and `Docs/SECURITY_AND_QUALITY_GUIDELINES.md`.

**VALIDATION_VERDICT: APPROVED**
