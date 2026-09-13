# Implementation Plan Evaluation Report

- **Evaluated Plan**: `AI/BACKLOG.md` (Issue #5: `camera-v4l` Crate — V4L2 Capture Manager) & Implementation Plan
- **Target Issue**: Issue #5 (`camera-v4l` Crate — V4L2 Capture Manager / GitHub Issue #12)
- **Date**: 2026-09-13

---

## Pillar Analysis

### 1. Architectural Alignment: [PASS]
- **Daemon Ownership Boundary**: In accordance with `AI/ARCHITECTURE.md` §6 Warm Camera Streaming and ADR [2026-09-12], hardware capture is strictly managed by `soos-daemon`. The unprivileged PAM module (`pam_soos.so`) never interacts with `/dev/video*` or video streaming libraries.
- **Hardware Addressing by Path (Criterion C4)**: Device addressing supports `/dev/v4l/by-id/...` persistent identifiers rather than ephemeral numerical indices (`/dev/video0`), avoiding device re-enumeration races across reboots.
- **Lock-Free RAM Snapshot Buffer (Criterion C2)**: The background capture thread continuously updates an `ArcSwapOption<Frame>`. Reading the latest frame takes zero locks and completes in nanoseconds (< 5ms), decoupling video hardware I/O from authentication request processing.

### 2. PAM Deadlines & Concurrency: [PASS]
- **Zero PAM Coupling**: `crates/camera-v4l` is an internal engine crate for the daemon. It is never linked or invoked inside the PAM `.so`.
- **Warm Camera Latency Margin**: By keeping the camera streaming in the background (warm capture), incoming PAM requests never endure device initialization delays (typically 100–300ms). The RAM snapshot is immediately available.
- **Warmup Auto-Exposure Stabilization (Criterion C5)**: Discards the first 15–30 frames upon camera startup to ensure exposure and white-balance stabilization before advertising `is_ready() == true`.
- **Zero Display Stream Pollution**: Production code contains zero `println!`, `eprintln!`, or `dbg!` macros, adhering to workspace lints.

### 3. Panic Safety & Fail-Closed: [PASS]
- **Zero Panics in Production**: The crate uses explicit `CameraError` (`thiserror`) and returns `Result` across all fallible paths. `unwrap()` and `expect()` are denied.
- **Error Recovery with Bounded Backoff (Criterion C3)**: Handles `ENODEV`, `EBUSY`, and `EIO` gracefully. The supervisor loop implements bounded exponential backoff (100ms → 200ms → 400ms → cap at 5,000ms) without aborting or hanging the host process.
- **Fail-Closed Availability**: When camera hardware is disconnected, uninitialized, or recovering from errors, `is_ready()` returns `false` and `latest_frame()` yields `None`. This maps cleanly to daemon `Verdict::Unavailable` and PAM `PAM_IGNORE`.

### 4. Dependency Isolation: [PASS]
- **Prohibited Crates**: Zero dependency on `opencv` or `nokhwa` (enforced by `tests/invariants/` and `deny.toml`).
- **Standardized V4L2 Stack**: Uses `v4l = "0.14"` and `arc-swap = "1"`.
- **Unsafe Code Policy**: Low-level MMAP handling is encapsulated by `v4l`. Any adapter unsafe code is strictly documented with `// SAFETY:` justifications under `#![deny(clippy::undocumented_unsafe_blocks)]`.

### 5. Memory & Secret Hygiene: [PASS]
- **Bounded Buffer Allocation**: Uses fixed-count V4L2 kernel MMAP buffers (e.g. 4 buffers) with immediate queue/dequeue rotation. Memory cannot grow unbounded.
- **Zero Credential Exposure**: Frame structures handle only raw pixel buffers (`Vec<u8>`) and monotonic timing metadata. Zero passwords or encryption keys are handled.

### 6. Test Integrity: [PASS]
- **Mock Camera Simulation (Criterion C1)**: Behind `mock-camera` feature, `MockCameraManager` generates synthetic frames and simulates `ENODEV`, `EBUSY`, `EIO`, and frame starvation for automated testing in headless environments.
- **Comprehensive Test Suite**: Covers C1 (MockCameraManager), C2 (RAM snapshot latency < 5ms), C3 (Error recovery and backoff), C4 (Persistent hardware path configuration), and C5 (Warmup frame discard).
- **Verification Matrix Traceability**: Maps directly to criteria `C1`, `C2`, `C3`, `C4`, `C5` in `AI/VERIFICATION_MATRIX.md`.

---

## Findings & Recommendations

1. **Monotonic Timing Source**:
   - *Observation*: Monotonic timestamps for frames should use `CLOCK_MONOTONIC` to ensure immunity to system clock steps (NTP).
   - *Recommendation*: Use `std::time::Instant` or `libc::clock_gettime(libc::CLOCK_MONOTONIC, ...)` for `timestamp_mono_ns`.
2. **Dynamic Idle Power Throttling**:
   - *Observation*: Dropping to 5 FPS after 60s idle saves CPU and camera thermal budget.
   - *Recommendation*: Expose `notify_activity()` on `CameraManager` so that incoming IPC ping/auth calls immediately restore 30 FPS before frame evaluation.

---

## Conclusion

**VALIDATION_VERDICT: APPROVED**
