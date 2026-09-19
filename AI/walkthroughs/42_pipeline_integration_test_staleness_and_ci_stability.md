# Walkthrough 42 — Pipeline Integration Test Staleness & CI Stability

## Context & Objectives

- **Mission**: Resolve intermittent CI failure on `main` where `pipeline_integration_tests::test_12_5_rate_limit_exceeded_returns_protocol_error_rate_limited` panicked with `assertion left == right failed (left: Unavailable, right: Allow)` on GitHub Actions runner VMs.
- **Root Cause Analysis**:
  1. On constrained 2-vCPU CI runners executing all test suites concurrently, the OS worker thread in `MockCameraManager` was occasionally descheduled or delayed in its sleep intervals.
  2. Between back-to-back authentication requests in `test_12_5`, the latest frame timestamp in `MockCameraManager` could exceed the 150ms freshness threshold (`MAX_FRAME_AGE_NS`), causing `ConnectionDispatcher` to return `Verdict::Unavailable` (`ReasonClass::StaleFrame`) instead of `Verdict::Allow`.
  3. Furthermore, `TestPipelineFixture` lacked an explicit `Drop` implementation to call `camera.stop()`, causing background OS worker threads to linger as zombies across subsequent tests.
  4. Server listener tasks in `test_12_5` and `test_12_6` were never aborted, holding references to the dispatcher and camera.

---

## 1. Architecture & Mock Improvements (Phase 1 & Phase 4)

- **Immediate Activity Frame Refresh in `MockCameraManager`**:
  - In `crates/camera-v4l/src/mock.rs`:
    Updated `CameraManager::notify_activity` to immediately generate and store a fresh frame when the camera is ready, running, and not starved/errored/frozen.
    This satisfies the trait contract ("immediately restoring full FPS") and guarantees that incoming authentication requests receive a fresh frame with `mono_ns` timestamp matching request arrival, eliminating CI scheduler jitter.
- **Deterministic Sensor Freeze Simulation**:
  - Added `frozen: Arc<AtomicBool>` and `pub fn set_frozen(&self, frozen: bool)` to `MockCameraManager`.
  - When frozen, frame generation and `notify_activity` timestamp updates are suspended while readiness is preserved, enabling deterministic testing of the 150ms frame freshness invariant (`MAX_FRAME_AGE_NS`).
- **Deterministic Static Frame Generation**:
  - Replaced sequence-dependent pattern offsets with a deterministic static gradient pattern, ensuring 100% stable embedding generation across multiple authentication attempts.

---

## 2. Test Contract & Resource Cleanup (Phase 2 & Phase 4)

- **Fixture Resource Cleanup via RAII `Drop`**:
  - Implemented `Drop for TestPipelineFixture` in `crates/daemon/tests/pipeline_integration_tests.rs`:
    ```rust
    impl Drop for TestPipelineFixture {
        fn drop(&mut self) {
            self.camera.stop();
        }
    }
    ```
  - Aborted background listener tasks in `test_12_5` and `test_12_6`.
- **Contractual Stale Frame Verification**:
  - Authored `test_12_7_frozen_camera_returns_unavailable_stale_frame`:
    Contractually asserts that freezing the camera and allowing 160ms (> `MAX_FRAME_AGE_NS`) to elapse deterministically returns `Verdict::Unavailable` with `ReasonClass::StaleFrame`.

---

## 3. Auditing & Candid Review (Phase 3 & Phase 5)

- Panic safety verified: zero `unwrap()` or `expect()` introduced.
- Strict `#![forbid(unsafe_code)]` preserved across business crates.
- `scripts/candid_review.sh` executed: all 7 architectural checks passed.
- Plan Evaluator and Candid Review reports updated with `APPROVED` verdicts.

---

## 4. Verification Results

- `cargo test -p soos-daemon --test pipeline_integration_tests` passed 10 consecutive runs with zero failures.
- Full workspace test suite (`cargo test --all-targets`): 100% green.
- Quality gates: `cargo fmt --check` and `cargo clippy --all-targets --all-features -- -D warnings` passed cleanly.
