# Walkthrough 182 — Camera-Busy Message for `soos-enroll`

- **Date**: 2026-10-05
- **Issue**: GitHub #337 (GitHub-only, no backlog id; not registered in `scripts/sync_issue.py`, the
  squash commit carries `Closes #337`) — **Branch**: `fix/enroll-camera-busy-message`
- **Matrix criteria**: ECB1, ECB2, ECB-F, ECB3 (component `enroll-camera-busy-message`)

## 1. Context & Objectives

`soos-daemon` owns the camera while it runs. Running `sudo soos-enroll enroll` next to it failed after
the 2 s first-frame budget with `Camera error: Frame capture timed out or starved`, which does not tell
the operator that the device is held by another process or how to proceed. The camera worker already
classifies the open failure as `CameraStatus::Error { kind: CameraErrorKind::DeviceBusy, .. }`, but the
enrollment service ignored the status. Objective: report a busy camera with an actionable message
(enroll from `soos-gui`, or stop the daemon, enroll, start it again) without changing budgets, polling
or any other error path.

## 2. Architect Design

Spec: `AI/architect_spec_enroll_camera_busy_message.md`.

- New field-less variant `EnrollmentCliError::CameraBusy` in `crates/enrollment-cli/src/error.rs` with
  a static message naming `soos-daemon`, `soos-gui`, `sudo systemctl stop soos-daemon` and
  `sudo systemctl start soos-daemon`.
- `EnrollmentService::acquire_frame_after`: on budget expiry only, read `camera.status()` once; map
  `Error { kind: DeviceBusy, .. }` to `CameraBusy`, everything else to the unchanged
  `Camera(CameraError::Starved)`.
- No change to `FIRST_FRAME_TIMEOUT_MS`, `ENROLL_FRESH_FRAME_TIMEOUT_MS`, `FRAME_POLL_INTERVAL_MS`, the
  camera crate, the GUI, the daemon, PAM or `main.rs`.

## 3. Plan Evaluation

`AI/plan_evaluator_report.md`: VALIDATION_VERDICT APPROVED with six minor findings, all applied:
ECB3 restated as a review check (every error already exits 1); no new mock setter (`set_error` with
`EBUSY` already reports `DeviceBusy`); an ECB-F guard against a status-first implementation; a status
stub for `Suspended`/`Starting`; the fresh-frame budget and `debug-vision` path documented as intended
beneficiaries; ECB1 also asserts the restart command and the "another process holds" phrase.

## 4. Tester Contract

`AI/tester_contract_enroll_camera_busy_message.md`, file
`crates/enrollment-cli/tests/enroll_camera_busy_tests.rs` (14 tests): six ECB1 tests (enroll, verify,
custom failure count, camera going busy mid-enrollment, `Display` text, end-to-end text), six ECB2
regression guards (device not found on enroll and verify, starved mock, `Suspended`, `Starting`, every
other `CameraErrorKind`), two ECB-F guards (frame in time with a busy status, enroll and verify).
Red evidence: 8 passed, 6 failed on assertions (`got Err(Camera(Starved))`, `got "Camera is busy"`).
Migrated tests: none; `enroll_fresh_frames_tests::test_enroll_frozen_camera_fails_with_bounded_wait`
still pins `Camera(Starved)`.

## 5. Auditor Constraints

`AI/auditor_constraints_enroll_camera_busy_message.md`: CLEARED, 12 constraints. How they were met:

1. No `unwrap`/`expect`/panic macro: the check is a single `matches!`.
2. `status()` is read once, only after the last `latest_frame()` poll failed and the budget expired.
3. Constants, loop shape and sleeps unchanged; `status()` is non-blocking.
4. Only `CameraErrorKind::DeviceBusy` maps to `CameraBusy` (no wildcard `Error { .. }` arm).
5. The mapping sits at the single expiry return, so both budgets share it.
6. `CameraBusy` is a unit variant with the exact spec text; the "TDD Red stub" doc line is removed.
7. The message is static text, English, with accurate commands.
8. `main.rs` is untouched (ECB3).
9. `CameraBusy` is an `Err`; no template is stored; only `error.rs`, `service.rs` and docs changed.
10. No `unsafe`, no dependency, `Cargo.lock` unchanged.
11. No test modified.
12. No exhaustive `match` on `EnrollmentCliError` elsewhere; the workspace builds.

## 6. Implementation

- `crates/enrollment-cli/src/error.rs`: final `#[error(...)]` text for `CameraBusy`.
- `crates/enrollment-cli/src/service.rs`: imports `CameraErrorKind` and `CameraStatus`; on budget
  expiry `acquire_frame_after` returns `CameraBusy` when the status is
  `Error { kind: DeviceBusy, .. }`, otherwise `Camera(Starved)`; the `# Errors` doc lists both.
- `Docs/ENROLLMENT_CLI.md`: new section 5 "Troubleshooting" describing the message and both remedies.
- `AI/VERIFICATION_MATRIX.md`: component `enroll-camera-busy-message`, rows ECB1, ECB2, ECB-F
  (`✅ Verified`) and ECB3 (`⬜ Pending (review check, no automated test)`: the citation invariant
  `test_matrix_claimed_rows_cite_only_existing_evidence` rejects production source as evidence).

## 7. Candid Review

Pending at the time of writing; run by the orchestrator after this phase (`./scripts/candid_review.sh`
and the candid-reviewer sub-agent writing `AI/candid_review_report.md`).

## 8. Verification Results

- `cargo test -p soos-enrollment-cli --locked --all-features --test enroll_camera_busy_tests`:
  14 passed, 0 failed.
- `cargo fmt --all --check`: clean.
- `cargo clippy --workspace --all-targets --locked --all-features -- -D warnings`: clean.
- `cargo test --workspace --locked --all-features`: all green.
- `python3 scripts/sync_issue.py --check`: OK.

## 9. Known Limitations / Follow-ups

- ECB3 is a review check, not a test: `main()` maps every error to exit code 1 and a binary-level test
  with a hermetically busy V4L device is not feasible.
- A camera held by another process that the V4L worker does not classify as `EBUSY` still reports the
  generic starved-camera error.
