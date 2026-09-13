# Walkthrough 20 — `camera-v4l` Crate: V4L2 MMAP Capture Manager

> **Date**: 2026-09-13  
> **Target**: Issue #5 (`camera-v4l` Crate — V4L2 Capture Manager / GitHub Issue #12)  
> **Branch**: `feat/camera-v4l`  
> **Verification Matrix**: C1, C2, C3, C4, C5  

---

## 1. Overview & Objectives

Issue #5 introduces `crates/camera-v4l/` (`soos-camera-v4l`), the warm camera streaming engine for `soos-daemon`. To achieve the project's <= 150ms authentication latency budget, the camera manager operates continuously in a dedicated background thread, updating an atomic `ArcSwapOption<Frame>` RAM snapshot so authentication workers never wait on camera sensor initialization.

### Architectural Invariants Enforced
1. **Daemon Exclusive Ownership**: Only the privileged root daemon accesses `/dev/video*`. The PAM module (`pam_soos.so`) never links this crate.
2. **Persistent Hardware Paths (C4)**: Uses `/dev/v4l/by-id/...` stable hardware identifiers rather than unstable integer indices (`/dev/video0`).
3. **Lock-Free RAM Snapshotting (C2)**: Frame lookup completes in nanoseconds (< 5ms criterion, p95 < 100 µs measured).
4. **Auto-Exposure Convergence (C5)**: Discards the first 15–30 frames upon startup before setting `is_ready(true)`.
5. **Panic Safety & Bounded Backoff (C3)**: Handles `ENODEV`, `EBUSY`, and `EIO` without crashing, applying bounded exponential backoff (100ms → 5,000ms).
6. **Hardware-Free Simulation (C1)**: Provides `MockCameraManager` for headless CI and developer workstations.

---

## 2. Multi-Agent Implementation Cycle

### Phase 1 — Architect Sub-Agent
- Scaffolds `crates/camera-v4l/` with `Cargo.toml`, configured with `v4l = "0.14"`, `arc-swap = "1"`, and `mock-camera` feature flag.
- Specifies core domain structures:
  - `PixelFormat`: `Yuyv`, `Rgb24`, `Grey`, `Mjpeg` with buffer size calculations.
  - `Frame`: raw data buffer, width, height, monotonic timestamp in ns, sequence counter, and age methods.
  - `CameraConfig` & `CameraConfigBuilder`: configurable paths, resolution, FPS, idle throttling, warmup frames, and backoff limits.
  - `CameraManager` trait: `latest_frame()`, `is_ready()`, `notify_activity()`, `stop()`.
  - `CameraError` (`thiserror`): comprehensive error mapping for capabilities, formats, MMAP queues, and I/O.

### Phase 1.5 — Plan Evaluator Sub-Agent
- Conducted exhaustive compliance audit across the 6 architectural pillars.
- Produced `AI/plan_evaluations/02_camera_v4l_plan_evaluation.md` with **`VALIDATION_VERDICT: APPROVED`**.

### Phase 2 — Tester Sub-Agent (TDD Red Phase)
- Authored 15 contractual unit, integration, and benchmark tests:
  - `tests/mock_camera_tests.rs` (C1): frame generation, readiness transitions, resolution override, starvation simulation, idle throttling and wake.
  - `tests/bench_latency_tests.rs` (C2): p95 latency benchmark across 10,000 iterations (< 5ms) and concurrent multi-thread read test.
  - `tests/error_recovery_tests.rs` (C3): fault injection of `ENODEV`, `EBUSY`, `EIO` without panic, automatic recovery, and exponential backoff progression math.
  - `tests/config_hardware_tests.rs` (C4): persistent `/dev/v4l/by-id/` validation, builder options, and OS error mapping.
  - `tests/warmup_tests.rs` (C5): warmup frame discard, readiness gating, and configurable frame counts (15–30).
- Confirmed initial TDD Red state (test failures against initial stubs).

### Phase 3 — Auditor Sub-Agent
- Verified zero `unwrap()` or `expect()` in production code.
- Confirmed zero stdout/stderr prints (`println!`, `dbg!`).
- Verified memory bounds and zero sensitive credential exposure.

### Phase 4 — Developer Sub-Agent (TDD Green Phase)
- Implemented `MockCameraManager` in `src/mock.rs` with synthetic frame generation, fault injection, and idle power throttling.
- Implemented `V4lCameraManager` in `src/v4l_impl.rs` with MMAP buffer streaming, auto-exposure warmup frame discard, and exponential backoff supervisor.
- All 15 unit/benchmark tests passed cleanly.
- Workspace-wide tests passed (70+ tests across monorepo).
- Clippy verified clean (`cargo clippy --all-targets --all-features -- -D warnings`).
- Code formatted with `cargo fmt`.

### Phase 5 — Candid Reviewer Sub-Agent
- Conducted impartial pre-push review via `./scripts/candid_review.sh`.
- Authored `AI/candid_review_report.md` with **`VERDICT: APPROVED`**.

---

## 3. Verification Evidence

| Matrix ID | Criterion | Test Evidence | Status |
|---|---|---|---|
| **C1** | `mock-camera` feature provides functional `MockCameraManager` | `mock_camera_tests::test_mock_camera_generates_frames_and_readiness` | ☑ Validated |
| **C2** | Fresh frame available in < 5ms via `ArcSwap` | `bench_latency_tests::test_arcswap_frame_retrieval_latency_under_5ms` | ☑ Validated |
| **C3** | Handles `ENODEV`, `EIO`, `EBUSY` without panic | `error_recovery_tests::test_error_recovery_enodev_without_panic` | ☑ Validated |
| **C4** | Hardware selection by `/dev/v4l/by-id/` rather than index | `config_hardware_tests::test_config_by_id_path_selection` | ☑ Validated |
| **C5** | Drops first 15–30 frames after startup for auto-exposure | `warmup_tests::test_warmup_frames_discard_before_ready` | ☑ Validated |

---

## 4. Deliverables
- Crate: `crates/camera-v4l/` (`Cargo.toml`, `src/lib.rs`, `src/error.rs`, `src/frame.rs`, `src/config.rs`, `src/manager.rs`, `src/v4l_impl.rs`, `src/mock.rs`)
- Test suites: `crates/camera-v4l/tests/` (`mock_camera_tests.rs`, `bench_latency_tests.rs`, `error_recovery_tests.rs`, `config_hardware_tests.rs`, `warmup_tests.rs`)
- Documentation: `Docs/CAMERA_V4L_CRATE.md`
- Plan evaluation: `AI/plan_evaluations/02_camera_v4l_plan_evaluation.md`
- Candid review: `AI/candid_review_report.md`
- Verification matrix: updated `AI/VERIFICATION_MATRIX.md`
