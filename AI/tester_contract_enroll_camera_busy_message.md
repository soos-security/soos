# Tester Contract — GitHub #337 (soos-enroll camera-busy message)

Branch: `fix/enroll-camera-busy-message`. Spec: `AI/architect_spec_enroll_camera_busy_message.md`.
Plan evaluation: `AI/plan_evaluator_report.md` (APPROVED; minor findings 1-6 applied below).

Test file: `crates/enrollment-cli/tests/enroll_camera_busy_tests.rs` (public `EnrollmentService`
API only; mock camera = `MockCameraManager::set_error`/`set_starved`, plus a local
`StatusStubCamera` implementing `CameraManager` with a fixed `status()` and a configurable
number of frames before going silent).

Red command: `cargo test -p soos-enrollment-cli --locked --all-features --test enroll_camera_busy_tests`
→ 8 passed, 6 failed (all failures on assertions, no compile error).

## Production stub added for compilation

| Item | Location | Note |
|---|---|---|
| `EnrollmentCliError::CameraBusy` (field-less) with placeholder text `"Camera is busy"` | `crates/enrollment-cli/src/error.rs` | Developer must replace the text with the spec message and wire it into `acquire_frame_after`. |

No change to `soos-camera-v4l` (Finding 2: `set_error(Simulated { code: EBUSY })` already reports
`Error { kind: DeviceBusy, failures: 1 }`).

## Contract

| Test (enroll_camera_busy_tests.rs::) | Matrix ID | Red evidence |
|---|---|---|
| `test_ecb1_enroll_busy_camera_returns_camera_busy` | ECB1 (enroll, first-frame budget, mock EBUSY) | `got Err(Camera(Starved))` |
| `test_ecb1_verify_busy_camera_returns_camera_busy` | ECB1 (verify path) | `got Err(Camera(Starved))` |
| `test_ecb1_enroll_busy_custom_camera_with_failure_count_returns_camera_busy` | ECB1 (any `failures` count) | `got Err(Camera(Starved))` |
| `test_ecb1_enroll_camera_goes_busy_mid_enrollment_returns_camera_busy` | ECB1 (fresh-frame 500 ms budget, Finding 5) | `got Err(Camera(Starved))` |
| `test_ecb1_camera_busy_message_guides_operator` | ECB1 (Display: "another process holds", `soos-daemon`, `soos-gui`, `sudo systemctl stop soos-daemon`, `sudo systemctl start soos-daemon`; Finding 6) | `got "Camera is busy"` |
| `test_ecb1_camera_busy_from_enroll_displays_guidance` | ECB1 (end-to-end text, not the starvation text) | `got "Camera error: Frame capture timed out or starved"` |
| `test_ecb2_enroll_device_not_found_stays_starved` | ECB2 (mock ENODEV) | green (regression guard) |
| `test_ecb2_enroll_starved_mock_stays_starved` | ECB2 (mock `set_starved`) | green (regression guard) |
| `test_ecb2_enroll_suspended_status_stays_starved` | ECB2 (`Suspended`, stub; Finding 4) | green (regression guard) |
| `test_ecb2_enroll_starting_status_stays_starved` | ECB2 (`Starting`, stub) | green (regression guard) |
| `test_ecb2_enroll_other_error_kinds_stay_starved` | ECB2 (every `CameraErrorKind::ALL` except `DeviceBusy`, verify path) | green (regression guard; kills an "any `Error` ⇒ busy" implementation) |
| `test_ecb2_verify_device_not_found_stays_starved` | ECB2 (verify, mock ENODEV) | green (regression guard) |
| `test_ecb_frame_arriving_while_status_busy_is_used_for_enroll` | ECB-F (Finding 3: in-time frame used whatever the status) | green (regression guard; kills a status-first implementation) |
| `test_ecb_frame_arriving_while_status_busy_is_used_for_verify` | ECB-F (verify path) | green (regression guard) |

### ECB3 (restated, Finding 1)

ECB3 is a review check, not a test: `crates/enrollment-cli/src/main.rs` maps every `Err` from
`run()` to `[ERROR] {err}` and exit code 1; the change must not touch `main()` or add an exit
path. The candid reviewer verifies this on the diff.

### Migrated existing tests

None. `enroll_fresh_frames_tests.rs::test_enroll_frozen_camera_fails_with_bounded_wait` keeps
pinning `Camera(Starved)` (its camera uses the default `status()`, never `DeviceBusy`).

### Contract Migration (setup only, candid review finding)

`AI/candid_review_report.md` saw two tests fail when the whole crate ran in parallel:
`test_ecb1_verify_busy_camera_returns_camera_busy` and `test_ecb2_verify_device_not_found_stays_starved`
returned `Ok(Allow)`. Cause: a mock worker iteration that read "no error" just before
`set_error` can still publish one frame after the slot was cleared, so verify picked up a cached
frame. Fix: the new setup helper `wait_until_no_cached_frame` waits up to 5 s until
`latest_frame()` has stayed `None` for 250 ms (about 7 frame intervals), then asserts that no
frame is cached. It runs after `set_error` in `mock_camera_with_error`, which covers every
busy/ENODEV fixture, and after `set_starved` in `test_ecb2_enroll_starved_mock_stays_starved`.
No assertion was changed. Tests using the stub camera have no worker and no race.

### Flakiness check

`test_ecb_frame_*` run 10× in a row: 10/10 green. The other tests use deterministic stubs or a
mock with an injected error; their duration is bounded by the existing 2 s / 500 ms budgets.
