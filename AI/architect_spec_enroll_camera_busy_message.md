# Architect Spec — soos-enroll Camera-Busy Message (GitHub #337)

Branch: `fix/enroll-camera-busy-message` (GitHub-only, not registered in `scripts/sync_issue.py`).
Matrix prefix: `ECB` (ECB1–ECB3). Walkthrough: next free number.

## 1. Problem

`EnrollmentService::acquire_frame_after` (`crates/enrollment-cli/src/service.rs`) polls
`camera.latest_frame()` until a budget expires and then returns
`EnrollmentCliError::Camera(CameraError::Starved)` ("Camera error: Frame capture timed out or
starved"). When `soos-daemon` holds the device, the V4L supervisor of `soos-enroll` records
`CameraStatus::Error { kind: CameraErrorKind::DeviceBusy, .. }` (`CameraManager::status()`), but
that information is discarded.

## 2. Design

New variant in `crates/enrollment-cli/src/error.rs`:

```rust
#[error(
    "Camera is busy: another process holds the camera device. soos-daemon owns the camera \
     while it runs; enroll from soos-gui, or stop the daemon (sudo systemctl stop soos-daemon), \
     enroll, then start it again (sudo systemctl start soos-daemon)"
)]
CameraBusy,
```

`acquire_frame_after`, on budget expiry only: read `camera.status()` once; if it is
`CameraStatus::Error { kind: CameraErrorKind::DeviceBusy, .. }` return
`EnrollmentCliError::CameraBusy`, otherwise return the existing
`EnrollmentCliError::Camera(CameraError::Starved)` unchanged. A frame that arrives in time is
returned as today, whatever the status. No change to budgets, polling, the camera crate, the
GUI, the daemon or PAM.

Exit code: whatever the CLI maps `EnrollmentCliError` variants to today — check `main.rs`; the
new variant must map to the same exit code as `Camera(_)` (no new exit-code contract).

## 3. Acceptance Criteria

| ID | Criterion | Test |
|---|---|---|
| ECB1 | Mock camera with no frames and status `Error { kind: DeviceBusy, failures: n }` ⇒ the first-frame acquisition (enroll / verify path) returns `CameraBusy`; its `Display` contains `soos-daemon`, `soos-gui` and `sudo systemctl stop soos-daemon` | `crates/enrollment-cli/tests/` new file |
| ECB2 | Mock camera with no frames and any other status (`Starting`, `Suspended`, `Error` of another kind) ⇒ still `Camera(CameraError::Starved)` | same file |
| ECB3 | The CLI exit code for `CameraBusy` equals the one for `Camera(_)` | unit or integration test, per how exit codes are tested today |

Existing tests (e.g. `enroll_fresh_frames_tests.rs:254`, which pins `Starved` for a frameless
mock) must stay green unchanged. If the mock camera cannot report a `DeviceBusy` status today,
the tester may add a test-only setter on the mock (`soos-camera-v4l` `mock-camera` feature),
which is a production-crate addition, not a test change.

## 4. Invariants

No `unwrap`/`expect` in production; no `unsafe`; no new dependency; no frame or biometric data
in the message; English only; existing assertions unchanged.
