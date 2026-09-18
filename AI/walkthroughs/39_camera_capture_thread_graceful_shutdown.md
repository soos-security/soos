# Walkthrough 39: Graceful Capture Thread Shutdown and Memory Ordering

## 1. Overview & Architectural Motivation

Backlog Issue #23 (GitHub Issue #62) eliminates indefinite blocking during camera capture thread shutdown and hardens cross-thread memory visibility on weakly-ordered architectures:
1. **Graceful Capture Thread Shutdown (Criterion C9 / Sub-issue #23.1)**:
   - Previously, `v4l::io::mmap::Stream::next()` executed an unbounded blocking poll (`poll(..., -1)`) under the hood when dequeuing frames (`VIDIOC_DQBUF`).
   - If the camera stopped producing frames (e.g. device idle, low frame rate, or hardware stall), calling `drop()` or `stop()` on `V4lCameraManager` could hang indefinitely on `worker_handle.join()`.
   - Issue #23 introduces frame-rate adaptive stream timeouts via `stream.set_timeout(Duration)`, ensuring that `stream.next()` unblocks within at most 150–250ms.
   - On unblocking with `io::ErrorKind::TimedOut`, if shutdown was signaled (`!running.load(Ordering::Acquire)`), the capture loop immediately exits cleanly (`Ok(())`), allowing `Drop` to complete in strictly less than 500ms even when the camera is completely idle.
   - Replaced monolithic `thread::sleep` intervals during idle mode and supervisor backoff with responsive sliced sleep loops polling `running.load(Ordering::Acquire)` every 10–20ms.
2. **Acquire/Release Memory Ordering (Sub-issue #23.2)**:
   - Upgraded `is_ready`, `running`, and `starved` atomic flags from `Ordering::Relaxed` to paired `Acquire`/`Release` semantics.
   - Sequenced `latest_frame.store(...)` before `is_ready.store(true, Ordering::Release)` to guarantee that whenever a reader observes `is_ready() == true` with `Ordering::Acquire`, `latest_frame.load_full()` is guaranteed to observe the published frame without stale reads.
   - Enforced monotonic sequence tracking in `MockCameraManager` via `store_frame_monotonic` and `Arc<AtomicU64>`, preventing race conditions between background frame generators and injected synthetic frames.

---

## 2. Key Changes by Component

### `crates/camera-v4l`
- [`v4l_impl.rs`](file:///home/hadrien/Project/soos/crates/camera-v4l/src/v4l_impl.rs):
  - Configured adaptive timeout on `v4l::io::mmap::Stream`:
    ```rust
    let frame_interval_ms = 1_000u64.checked_div(config.fps as u64).unwrap_or(33);
    let stream_timeout_ms = frame_interval_ms.saturating_mul(3).clamp(150, 250);
    stream.set_timeout(Duration::from_millis(stream_timeout_ms));
    ```
  - Intercepted `io::ErrorKind::TimedOut` in `open_and_stream()`:
    - If `!running.load(Ordering::Acquire)`: returns `Ok(())` for immediate graceful termination.
    - If `running.load(Ordering::Acquire)`: returns `Err(CameraError::BufferDequeue { ... })` to enter supervisor backoff.
  - Sliced idle sleep into 20ms polling increments checking `running.load(Ordering::Acquire)`.
  - Upgraded `is_ready(&self)` to `self.is_ready.load(Ordering::Acquire)`.
  - Sequenced `latest_frame.store(...)` prior to `is_ready.store(true, Ordering::Release)`.
  - Upgraded `running` stores in `stop()` and `drop()` to `Ordering::Release`.
- [`mock.rs`](file:///home/hadrien/Project/soos/crates/camera-v4l/src/mock.rs):
  - Sliced frame interval sleep into 10ms polling increments checking `running_clone.load(Ordering::Acquire)` and `starved_clone.load(Ordering::Acquire)`.
  - Added `sequence: Arc<AtomicU64>` and `store_frame_monotonic` to guarantee monotonically non-decreasing frame sequence visibility across concurrent producers.
  - Fast-path fail-closed evaluation in `is_ready(&self)` checking `self.starved.load(Ordering::Acquire)`.
  - Upgraded all `is_ready`, `running`, and `starved` loads and stores to `Acquire`/`Release` semantics.
- [`shutdown_tests.rs`](file:///home/hadrien/Project/soos/crates/camera-v4l/tests/shutdown_tests.rs):
  - Created contractual test suite for Criterion C9 and sub-issues #23.1 and #23.2.

### `scripts/sync_issue.py`
- Registered `"fix/camera-thread-shutdown": 23` in `BRANCH_TO_ISSUE` for automated issue tracking.

### `AI/` Documentation
- Updated [`BACKLOG.md`](file:///home/hadrien/Project/soos/AI/BACKLOG.md) checking off sub-issues #23.1 and #23.2 and marking Criterion C9 as Verified.
- Updated [`VERIFICATION_MATRIX.md`](file:///home/hadrien/Project/soos/AI/VERIFICATION_MATRIX.md) adding Criterion C9 acceptance criteria and test evidence.

---

## 3. Verification & Test Evidence

### Contractual Test Suite (`crates/camera-v4l/tests/shutdown_tests.rs`)
1. `test_camera_drop_completes_within_timeout`:
   - Configures a camera with `idle_fps: 1` (1000ms frame interval) and `idle_timeout: 10ms`.
   - Asserts that `drop(camera)` completes within strictly less than 500ms (completes in ~10–20ms).
2. `test_camera_stop_signals_graceful_shutdown`:
   - Asserts calling `camera.stop()` flips `is_ready()` to false immediately and subsequent `drop()` completes in < 100ms.
3. `test_v4l_camera_drop_completes_within_timeout`:
   - Spawns `V4lCameraManager` with non-existent device path and 2000ms max backoff.
   - Asserts that `drop()` completes within < 500ms during backoff.
4. `test_is_ready_memory_visibility_acquire_release`:
   - Multi-threaded stress test with 4 concurrent reader threads continuously querying `is_ready()` and `latest_frame()` while a writer thread injects 500 frames.
   - Asserts that whenever `is_ready()` returns true, `latest_frame()` is never `None`, and frame sequence numbers never go backwards.

### Test Execution Results
```text
running 4 tests
test test_v4l_camera_drop_completes_within_timeout ... ok
test test_camera_stop_signals_graceful_shutdown ... ok
test test_camera_drop_completes_within_timeout ... ok
test test_is_ready_memory_visibility_acquire_release ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 13.96s
```

All 28 unit, integration, and benchmark tests in `camera-v4l` pass cleanly, and the entire workspace test suite is 100% green with zero Clippy warnings.
