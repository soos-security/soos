# Walkthrough 107 — Bounded Polling Instead of Fixed Sleeps in Timing-Sensitive Tests

- **Date**: 2026-09-30
- **Issue**: GitHub #280 ([TCI-FLAKY] timing-based tests with fixed sleeps fail under host load)
- **Branch**: `test/deflake-timing-tests`
- **Matrix criteria**: TFL1–TFL4 (new)
- **Scope**: test files and test helpers of `soos-camera-v4l` and `soos-gui`, plus docs. No
  production code, no assertion, threshold, tolerance or `#[ignore]` change.

---

## 1. Context

Several tests waited a fixed `thread::sleep` (or polled with a tight bound) for a background
thread, then asserted its state. On a loaded host the background thread had not run yet, and the
test failed although the code was correct. This blocked `save.sh` repeatedly during the
2026-09-30 correction pass.

## 2. Reproduction (red evidence)

The load was 64 CPU burners on a 16-core host (`timeout <s> sh -c 'while :; do :; done'`, each
stopped by `timeout`). Four runs of
`cargo test --locked -p soos-camera-v4l -p soos-gui --all-targets --all-features --no-fail-fast`
before the change failed at these setup-dependent lines, among others:

| Test | Line of failure (before) | Cause |
|---|---|---|
| `error_recovery_tests::test_error_recovery_{enodev,ebusy,eio}_without_panic` | `assert!(camera.is_ready())` after a 50 ms sleep | warmup not finished yet |
| `hotunplug_tests::test_camera_hotunplug_recovery` | `assert!(camera.is_ready(), ...)` after a 100 ms sleep | warmup not finished yet |
| `mock_camera_tests::test_mock_camera_idle_throttling_and_wake` | `assert!(camera.is_ready())` after a 50 ms sleep | warmup not finished yet |
| `shutdown_tests::test_camera_stop_signals_graceful_shutdown` | `assert!(camera.is_ready(), ...)` after a 60 ms sleep | warmup not finished yet |
| `bench_latency_tests::test_lock_free_concurrent_access` | `assert!(camera.is_ready())` after a 100 ms sleep | warmup not finished yet |
| `warmup_tests::test_warmup_frames_configurable_bounds` | `assert!(camera.is_ready(), ...)` after `warmup_count * 10 + 100` ms | warmup not finished yet |
| `layout_tests::test_gui_worker_fallback_renders_raw_rgb_on_pipeline_error` | `assert!(frame.is_some(), ...)` after a 600 ms poll | worker had not published yet |
| `layout_tests::test_ipc_camera_manager_receives_persistent_frames` | `assert!(manager.is_ready(), ...)` after a 1000 ms poll | manager had not connected yet |

The same load also failed tests whose assertion is itself a wall-clock bound (section 5).

## 3. Change

A per-crate test helper, `crates/camera-v4l/tests/common/mod.rs` and
`crates/gui/tests/common/mod.rs`:

- `SETTLE_TIMEOUT` = 5 s, a generous upper bound;
- `wait_until(timeout, condition) -> bool` evaluates `condition` every 2 ms until it holds or the
  bound elapses (it returns the last evaluation).

Every migrated test keeps its original assertion right after the wait. A condition that never
becomes true within 5 s still fails that assertion.

Negative assertions (NOT ready and no frame after an injected error or starvation) keep their
original 50 ms dwell, so that a worker ignoring the fault would have republished a frame, and then
wait for the expected state with the same helper. A faulty worker keeps publishing, the wait times
out, and the unchanged assertion fails. A correct worker preempted between its error check and its
frame store no longer fails the test.

## 4. Tests Touched (setup lines only)

Every assertion, message, threshold and configuration value is unchanged. "Poll" below means
`wait_until(SETTLE_TIMEOUT, || <condition>);`.

### `soos-camera-v4l`

| Test | Setup lines changed |
|---|---|
| `error_recovery_tests::test_error_recovery_enodev_without_panic` | initial `sleep(50ms)` → poll `camera.is_ready()`; after `set_error(Some(..))`: `sleep(50ms)` kept as `NEGATIVE_DWELL` + poll `!camera.is_ready() && camera.latest_frame().is_none()`; after `set_error(None)`: `sleep(150ms)` → poll `camera.is_ready() && camera.latest_frame().is_some()` |
| `error_recovery_tests::test_error_recovery_ebusy_without_panic` | initial `sleep(50ms)` → poll `camera.is_ready()`; after `set_error(Some(..))`: `sleep(50ms)` kept as `NEGATIVE_DWELL` + poll `!camera.is_ready() && camera.latest_frame().is_none()`; after `set_error(None)`: `sleep(150ms)` → poll `camera.is_ready()` |
| `error_recovery_tests::test_error_recovery_eio_without_panic` | initial `sleep(50ms)` → poll `camera.is_ready()`; after `set_error(Some(..))`: `sleep(50ms)` kept as `NEGATIVE_DWELL` + poll `!camera.is_ready()`; after `set_error(None)`: `sleep(150ms)` → poll `camera.is_ready()` |
| `hotunplug_tests::test_camera_hotunplug_recovery` | `sleep(100ms)` → poll `camera.is_ready() && camera.latest_frame().is_some()`; after unplug: `sleep(50ms)` kept + poll `!camera.is_ready() && camera.latest_frame().is_none()`; after replug: `sleep(200ms)` → poll `camera.is_ready() && camera.latest_frame().is_some()` |
| `mock_camera_tests::test_mock_camera_generates_frames_and_readiness` | `sleep(250ms)` → poll `camera.is_ready()` (the later 50 ms sleep before the monotonic timestamp check is kept) |
| `mock_camera_tests::test_mock_camera_respects_custom_resolution` | `sleep(100ms)` → poll `camera.is_ready() && camera.latest_frame().is_some()` |
| `mock_camera_tests::test_mock_camera_simulates_starvation` | `sleep(100ms)` → poll `camera.is_ready()`; after `set_starved(true)`: `sleep(50ms)` kept + poll `!camera.is_ready() && camera.latest_frame().is_none()`; after `set_starved(false)`: `sleep(100ms)` → poll `camera.is_ready() && camera.latest_frame().is_some()` |
| `mock_camera_tests::test_mock_camera_idle_throttling_and_wake` | `sleep(50ms)` → poll `camera.is_ready()`; the 150 ms idle-expiry sleep (a lower bound) is kept and followed by poll `camera.latest_frame().is_none()` (the worker has actually entered the suspended state, so `notify_activity()` never races a late suspension) |
| `mock_camera_tests::test_mock_camera_auto_suspend_and_resume_lifecycle` | two hand-written `while !ready && elapsed < 500ms { sleep(10ms) }` loops → poll `camera.is_ready()` |
| `mock_camera_tests::test_mock_camera_idle_timeout_zero_disables_auto_standby` | hand-written 500 ms loop → poll `camera.is_ready()` (the 100 ms "no activity" sleep is kept) |
| `shutdown_tests::test_camera_stop_signals_graceful_shutdown` | `sleep(60ms)` → poll `camera.is_ready()` |
| `bench_latency_tests::test_arcswap_frame_retrieval_latency_under_5ms` | `sleep(100ms)` → poll `camera.is_ready()` |
| `bench_latency_tests::test_lock_free_concurrent_access` | `sleep(100ms)` → poll `camera.is_ready()` |
| `warmup_tests::test_warmup_frames_discard_before_ready` | final `sleep(300ms)` → poll `camera.is_ready() && camera.latest_frame().is_some()` (the 150 ms mid-warmup sleep and its negative assertion are kept) |
| `warmup_tests::test_warmup_frames_configurable_bounds` | `sleep(warmup_count * 10 + 100 ms)` → poll `camera.is_ready()` |

### `soos-gui`

| Test | Setup lines changed |
|---|---|
| `layout_tests::test_ipc_camera_manager_receives_persistent_frames` | `while !ready && elapsed < 1000ms { sleep(15ms) }` → poll `manager.is_ready()` |
| `layout_tests::test_gui_worker_fallback_renders_raw_rgb_on_pipeline_error` | `while slot.is_none() && elapsed < 600ms { sleep(20ms) }` → poll `latest_frame_slot.load().is_some()` |

## 5. Not Changed: Wall-Clock Assertions (proposals awaiting user approval)

These failures come from the assertion itself (a latency bound or a short time window). Polling
cannot fix them, and any threshold, tolerance or gating change needs explicit user approval.

| Test | Observed under load | Proposed options (not applied) |
|---|---|---|
| `soos-vision` `bench_tests::test_pipeline_latency_budget_under_150ms_p95` | p95 652.8 ms (budget 150 ms); it already runs one warm-up iteration outside the measurement | (a) gate behind an env var such as `SOOS_LATENCY_BENCH=1` and run it in a dedicated CI step with `--test-threads=1`; (b) mark it `#[ignore]` and run it with `--ignored` in that CI step; (c) measure per-thread CPU time (`CLOCK_THREAD_CPUTIME_ID`) instead of wall time, which changes what V5 measures |
| `soos-camera-v4l` `shutdown_tests::test_camera_drop_completes_within_timeout` (< 500 ms) and `test_camera_stop_signals_graceful_shutdown` (stopped drop < 100 ms) | 0.57–1.35 s and 236–568 ms | env gate / dedicated single-threaded CI step, as above; or assert that the worker observed the stop signal (join result) rather than a wall-clock duration |
| `soos-camera-v4l` `mock_camera_tests::test_mock_camera_auto_suspend_and_resume_lifecycle` (`idle_timeout` 60 ms) and `test_mock_camera_idle_throttling_and_wake` (100 ms) | the ready window is shorter than a scheduler delay: the worker itself can reach the idle expiry before it publishes the first frame | scale `idle_timeout` and the idle-expiry sleeps (for example to 1 s / 1.5 s), which changes test parameters |
| `soos-camera-v4l` `warmup_tests::test_warmup_frames_discard_before_ready` (mid-warmup negative at 150 ms of a ~330 ms warmup) | not observed failing, but the margin depends on scheduling | lower the fps or raise the warmup count so that the mid-warmup check has more margin |
| `soos-daemon` `pipeline_integration_tests` Allow cases (`test_12_5_*`, `test_12_6_*`, `test_48_*`, `test_147_*`, `test_169_grey_frames_above_ir_threshold_allow`) and `template_model_binding_tests::test_template_with_matching_model_id_is_evaluated` | `Unavailable` / `Timeout` instead of `Allow`: the fixture `connection_timeout` of 500 ms (450 ms after the write margin) bounds the whole multi-frame consensus | raise the fixture `connection_timeout` (for example to 5 s; the budget then becomes `DECISION_BUDGET_MS` = 900 ms, and `test_147_k_consecutive_live_frames_allow_within_budget` keeps its explicit 900 ms assertion), or send an explicit client deadline further away; both relax a timing bound |
| `soos-daemon` `preview_authorization_tests::test_preview_frame_rate_limited_per_peer_uid` | not observed failing; three requests must fit in the 1 s window | inject a clock into the preview rate limiter (production change) |

## 6. Verification

- Under the same load (64 burners), three further camera/GUI runs never failed on the
  setup-dependent lines of section 2. The only remaining failures were the wall-clock assertions of
  section 5 (stopped-camera drop 236.1–567.8 ms against 100 ms, idle drop 572.7 ms against
  500 ms, and the 60 ms idle window of `test_mock_camera_auto_suspend_and_resume_lifecycle`).
- Full workspace run under 32 burners: the only failures were
  `test_camera_stop_signals_graceful_shutdown` (drop 100.6 ms against 100 ms),
  `test_12_5_rate_limit_exceeded_returns_protocol_error_rate_limited` (`Timeout`, fixture
  `connection_timeout`) and `test_pipeline_latency_budget_under_150ms_p95` (p95 491.4 ms), all
  listed in section 5.
- Full gate without load, all green: `cargo fmt --all -- --check`, `cargo clippy --locked
  --workspace --all-targets --all-features -- -D warnings`, `cargo test --locked --workspace
  --all-targets --all-features --no-fail-fast` (three runs, 149 test binaries `ok` each),
  `./scripts/candid_review.sh`.

## 7. Documentation

- `Docs/DEVELOPMENT_WORKFLOW.md` §4.1: no fixed sleep before an assertion; positive and negative
  polling patterns; wall-clock assertions need approval.
- `AI/VERIFICATION_MATRIX.md`: component `timing-test-polling`, criteria TFL1–TFL4.
