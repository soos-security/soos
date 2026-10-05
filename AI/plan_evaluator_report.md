# Plan Evaluation Report
- **Date**: 2026-10-05
- **Issue**: GitHub #337 — fix: soos-enroll reports a generic starvation error when soos-daemon holds the camera (GitHub-only, no backlog entry)
- **Branch**: `fix/enroll-camera-busy-message`
- **Base commit**: `a1612f1`
- **Spec**: `AI/architect_spec_enroll_camera_busy_message.md`

## 1. Coverage Matrix
| Acceptance line / TDD test | Spec element | Status |
|---|---|---|
| #337-1: no frame in time + status `Error { kind: DeviceBusy, .. }` ⇒ dedicated error naming another process, `soos-daemon`, `soos-gui`, `systemctl stop/start` | `EnrollmentCliError::CameraBusy` + status check on budget expiry; ECB1 | Covered |
| #337-2: every other timeout keeps `CameraError::Starved` | "otherwise return the existing `Camera(Starved)`"; ECB2; `enroll_fresh_frames_tests.rs:240` unchanged | Covered |
| #337-3: no frame/biometric data in message; English only | Static `#[error]` text without fields; Invariants section | Covered |
| ECB3 (spec addition): exit code of `CameraBusy` equals `Camera(_)` | "no new exit-code contract" | Covered by construction, test is weak (Finding 1) |

## 2. Facts Verified Against Code
| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| `acquire_frame_after` polls `latest_frame()` and returns `Camera(Starved)` on budget expiry | `crates/enrollment-cli/src/service.rs:690-716` | Yes; budgets `FIRST_FRAME_TIMEOUT_MS = 2000` (prev `None`), `ENROLL_FRESH_FRAME_TIMEOUT_MS = 500` (prev `Some`), poll 10 ms | Match |
| Callers | `service.rs:677` (`acquire_frame`), `:744` (enroll loop), `:939` (verify), `:1370` (debug-vision) | All go through `acquire_frame_after`, so one change covers enroll, verify and debug-vision | Match (spec names only enroll/verify; debug-vision benefits too) |
| `camera` is a `dyn CameraManager` exposing `status()` | `service.rs:639`; `crates/camera-v4l/src/manager.rs:85` | `Option<Arc<dyn CameraManager>>`; trait default `status()` derives `Ready`/`Starting` from `is_ready()` | Match |
| V4L supervisor records `DeviceBusy` on EBUSY | `crates/camera-v4l/src/v4l_impl.rs:509` `shared.status.record_error(err.kind())`; EBUSY at `VIDIOC_S_FMT`/stream ioctls mapped via `CameraError::from_ioctl_error` → `from_io_error` → `DeviceBusy` (`error.rs:143-165`); open() EBUSY via `from_io_error` (`v4l_impl.rs:785-788`) | Yes | Match |
| Status stays `Error{DeviceBusy}` across retries (so a single read at expiry is reliable) | `v4l_impl.rs` status writes: 448 (Stopped on exit), 477 (Suspended), 495 (Starting on resume), 505 (Starting only if a frame was published), 509 (record_error) | No reset to `Starting` at the start of each attempt; with a busy device the status remains `Error{DeviceBusy, failures: n}` during backoff (100 ms min, 5 s max) | Match |
| `V4lCameraManager::status()` passes errors through | `v4l_impl.rs:395-402` | `Ready` if ready; else cell value (`Ready`→`Starting`) | Match |
| Mock cannot report DeviceBusy today (spec allows a test-only setter) | `crates/camera-v4l/src/mock.rs:210` `set_error`, `:362` `status()` | `set_error(Some(CameraError::Simulated { code: libc::EBUSY, .. }))` (or `DeviceBusy { .. }`) already yields `Error { kind: DeviceBusy, failures: 1 }`, withdraws the frame, and both the worker loop and `notify_activity` stop publishing frames while an error is set | Mismatch: no new setter is needed (Finding 2) |
| Exit code mapping | `crates/enrollment-cli/src/main.rs:294-297` | Every `Err` from `run()` prints `[ERROR] {err}` and exits 1; no per-variant mapping exists | Match ("same as `Camera(_)`" is automatic) |
| GUI parses soos-enroll error text | `crates/gui/src/privileged.rs` | GUI only invokes `list`/`import`/`delete` via pkexec and parses JSON / own messages; never parses enroll errors. `crates/gui/src/camera_status.rs` "Starved" is the GUI's own `CameraErrorKind` presentation | No impact |
| Existing tests pinning the text/variant | `crates/enrollment-cli/tests/enroll_fresh_frames_tests.rs:240-256` | Uses a custom `SlowSwapCamera` with the default `status()` (never `DeviceBusy`), so stays `Starved`; no invariant in `tests/invariants` pins the starvation text | Match |
| `libc` / `MockCameraManager` available to enrollment-cli tests | `crates/enrollment-cli/Cargo.toml:24,28`; `service.rs:15` | `libc` is a dependency; `MockCameraManager` is imported un-gated | Match |
| Next walkthrough | `AI/walkthroughs/` | Highest is 181 ⇒ 182 | n/a |

## 3. Pillar Analysis
### Pillar 1 — Architecture & threat model
- Failure scenario considered: the message could push operators to run a second camera owner permanently, or tell non-root users to bypass the daemon. The text only recommends the GUI (which goes through the daemon preview proxy) or a stop/enroll/start cycle that `soos-enroll` already requires root for. No change to the root daemon's exclusive ownership, IPC or socket permissions.
- Result: PASS

### Pillar 2 — PAM deadline & concurrency
- Failure scenario considered: an extra blocking call in the wait loop could lengthen the budget. `status()` is a mutex read done once after the budget expires, outside PAM; `crates/pam` is untouched.
- Result: PASS

### Pillar 3 — Panic safety & fail-closed
- Failure scenario considered: a frame that arrives while status is still `Error{DeviceBusy}` (stale cell) would be rejected if the implementation checked status first. The spec explicitly checks status only on expiry and returns any in-time frame whatever the status. A poisoned status mutex is handled by `unwrap_or_else(into_inner)` in `CameraStatusCell::get`. Both outcomes are errors; no path turns into success.
- Result: PASS

### Pillar 4 — Dependencies
- Failure scenario considered: a new crate for message formatting. None is added; `thiserror` already in use.
- Result: PASS

### Pillar 5 — Data confidentiality
- Failure scenario considered: an implementation that embeds the `CameraError` (which carries the device path and `io::Error`) or frame metadata into the message. The spec's variant is field-less with a static text; the device path would not be sensitive anyway, and nothing biometric is in scope.
- Result: PASS

### Pillar 6 — Test integrity
- Failure scenario considered: a wrong implementation that maps *any* `Error { .. }` status (e.g. `Starved`, `DeviceNotFound`, `PermissionDenied`) to `CameraBusy` must fail ECB2; the spec lists `Error` of another kind, so ECB2 has that power if the tester includes at least one non-busy `Error` kind (e.g. `Simulated { code: libc::ENODEV }` and `set_starved(true)`). A wrong implementation that checks status before the budget (returning `CameraBusy` immediately) would still pass ECB1 unless the test or another test covers the "frame arrives while status is busy" case (Finding 3). The existing `Starved` test is kept unchanged.
- Result: FINDING (MINOR)

## 4. Findings
1. **[MINOR]** ECB3 cannot fail: `main.rs:294-297` maps every `EnrollmentCliError` to exit code 1 and there is no per-variant mapping, and a binary-level test with a hermetically busy V4L device is not feasible. Required change: restate ECB3 as "no new exit path; `main()` unchanged, every error exits 1" verified by review (or drop it); the tester should not invent a variant-to-exit-code function just to test it.
2. **[MINOR]** The spec's fallback ("the tester may add a test-only setter on the mock") is unnecessary: `MockCameraManager::set_error(Some(CameraError::Simulated { code: libc::EBUSY, message }))` (or `CameraError::DeviceBusy { path, source }`) already makes `status()` return `Error { kind: DeviceBusy, failures: 1 }` and stops frame publication (worker loop and `notify_activity`). Required change: point the tester to `set_error`; no change to `soos-camera-v4l` is needed or allowed by "no change to the camera crate".
3. **[MINOR]** ECB1/ECB2 do not pin "a frame that arrives in time is returned whatever the status". Recommended: add an ECB test with a custom `CameraManager` stub whose `status()` returns `Error { kind: DeviceBusy, .. }` while `latest_frame()` returns a frame, asserting the frame-acquiring operation does not fail with `CameraBusy` (e.g. `debug_vision`/`verify` proceeds past capture). This guards against a status-first implementation.
4. **[MINOR]** ECB2 lists `Suspended`, which `MockCameraManager::status()` never returns (its `Suspended` mention is only a comment in the worker). The tester needs a small custom `CameraManager` stub overriding `status()` (the pattern already used by `SlowSwapCamera` in `enroll_fresh_frames_tests.rs`) for `Suspended`/`Starting`; mock `set_error(ENODEV)` and `set_starved(true)` cover the "other `Error` kind" case.
5. **[MINOR]** The spec names the enroll and verify paths only; `debug_vision` (`service.rs:1370`) and every enroll candidate after the first (500 ms fresh-frame budget, `service.rs:744`) also flow through `acquire_frame_after` and will get the same behavior. This is desirable; the spec/walkthrough should say so explicitly so the fresh-frame path is a known, intended change (mid-enrollment EBUSY is unlikely because the supervisor resets to `Starting` after a streamed attempt, `v4l_impl.rs:504-506`).
6. **[MINOR]** ECB1 lists `soos-daemon`, `soos-gui` and `sudo systemctl stop soos-daemon` but not the restart step the issue also requires. Required change: ECB1 should also assert `sudo systemctl start soos-daemon` and a phrase saying another process holds the camera.

## 5. Verdict
No CRITICAL or MAJOR finding. The design matches the code: the V4L supervisor does record and keep `Error { kind: DeviceBusy }` on EBUSY across retries, a single status read at budget expiry is sound, exit code is unchanged by construction, and no GUI or invariant depends on the starvation text. Minor findings 1-6 should be folded into the tester brief.

VALIDATION_VERDICT: APPROVED
