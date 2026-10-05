# Candid Review Report

- **Date**: 2026-10-05
- **Target Branch**: `fix/enroll-camera-busy-message`
- **Base (merge-base)**: `a1612f1`
- **Reviewed-Diff-Fingerprint**: `42a2ff18386860b64ca937455a42c74d312382abe9ef9b05097dfddc8405df5e`
- **Audited Files**: `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_enroll_camera_busy_message.md`, `AI/auditor_constraints_enroll_camera_busy_message.md`, `AI/tester_contract_enroll_camera_busy_message.md`, `AI/walkthroughs/182_enroll_camera_busy_message.md`, `Docs/ENROLLMENT_CLI.md`, `crates/enrollment-cli/src/error.rs`, `crates/enrollment-cli/src/service.rs`, `crates/enrollment-cli/tests/enroll_camera_busy_tests.rs`

## 1. Executive Summary

GitHub #337: when `soos-daemon` holds the camera, `soos-enroll` reported the generic
`Camera error: Frame capture timed out or starved`. The diff adds a unit variant
`EnrollmentCliError::CameraBusy` with a static operator-guidance message, and makes
`EnrollmentService::acquire_frame_after` read `camera.status()` only after the capture budget has
expired: `Error { kind: DeviceBusy, .. }` yields `CameraBusy`, every other status keeps
`Camera(CameraError::Starved)`. Budgets, polling and the frame-acceptance path are unchanged. One
new integration test file (14 tests) covers ECB1, ECB2 and ECB-F. No existing test was modified.
All three acceptance criteria of #337 are met; ECB3 is confirmed below by review. No findings.

## 2. Test Changes

Mechanical listing on `target/candid_diff.patch`:

- Test files touched: `crates/enrollment-cli/tests/enroll_camera_busy_tests.rs` (new file only).
- Removed or changed assertions (`^-` with `assert`, `#[test]`, `proptest!`, `should_panic`): **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, `tolerance`, `epsilon`): **none**.
- Inline `mod tests` changes: **none**.

The existing Starved pin `enroll_fresh_frames_tests::test_enroll_frozen_camera_fails_with_bounded_wait`
uses a stub that keeps the default `CameraManager::status()` (`Starting`), so it still expects and
gets `Camera(Starved)`. Nothing needed migrating.

## 3. Deep Reasoning Audit

### Logic & Architecture
- *Status-first implementation (busy reported while frames flow)*: the status is read only inside
  the `start.elapsed() >= budget` branch, after a failed frame poll. ECB-F tests use a stub that
  always reports `DeviceBusy` while publishing frames, and expect `enroll`/`verify` to succeed. A
  status-first implementation would fail them. PASS.
- *Both budgets*: `acquire_frame` (verify, debug-vision) goes through `acquire_frame_after(None)`
  (2 s), and enrollment candidates use `acquire_frame_after(Some(seq))` (500 ms). Both share the
  changed branch, and the tests cover first-frame (enroll, verify) and fresh-frame (busy
  mid-enrollment) paths. PASS.
- *Real V4L path*: `V4lCameraManager::status()` passes `Error { .. }` through. The supervisor calls
  `record_error(err.kind())` on an open/STREAMON failure and keeps that status during backoff (it
  goes back to `Starting` only after a frame was published). `CameraError::kind()` maps `EBUSY`
  (open and ioctl, #150) to `DeviceBusy`. So a camera held by the daemon really reports
  `DeviceBusy` when the 2 s budget expires. PASS.
- *Wrong kind matched*: `test_ecb2_enroll_other_error_kinds_stay_starved` loops over
  `CameraErrorKind::ALL` minus `DeviceBusy`, so a broad `Error { .. }` match would fail it. PASS.
- *Exhaustive-match consumers*: no crate outside `enrollment-cli` names `EnrollmentCliError`, and no
  exhaustive match on it exists in the crate. Adding a variant breaks nothing. PASS.
- *Scope*: there is no change to the daemon, the camera crate, budgets or the CLI surface, so there
  is no scope creep.

### ECB3 — exit code
`git diff a1612f1 -- crates/enrollment-cli/src/main.rs` is empty (main.rs is not in the patch).
`main()` still does `if let Err(err) = run() { eprintln!("[ERROR] {err}"); std::process::exit(1); }`
(lines 294–296). `CameraBusy` propagates through `?` from `enroll`/`verify`/`debug_vision` like
`Camera(_)`, so it prints `[ERROR] <message>` and exits 1. The only other `exit(1)` calls (verify
non-Allow verdict, migrate failures) are unrelated. **CONFIRMED.** The matrix row ECB3 is still
marked pending. The traceability phase can now mark it verified by review.

### PAM Concurrency & Deadlines
- `crates/pam` is not touched. The change is in the root enrollment CLI. The one added call,
  `camera.status()`, is non-blocking: a lock/atomic read with no I/O. It runs once, after the
  budget has expired, so the bounded wait stays bounded. PASS.

### Panic Safety & Fail-Closed
- The new code has no `unwrap`/`expect`/indexing/`panic!`. `CameraBusy` is an error, and enrollment
  stores nothing (`store.get(..).is_none()` is asserted in ECB1). No error path leads to a success
  or a stored template. PASS.

### Test Integrity & Anti-Weakening
- The new tests would fail against plausible wrong implementations: no change at all (ECB1 fails),
  status-first (ECB-F fails), any-`Error` match (ECB2 kinds loop fails), a message missing guidance
  (needle checks fail).
- **Flakiness**: `cargo test -p soos-enrollment-cli --locked --all-features` ran **3 times, all
  green, 0 failures** (ECB suite: 14 passed, about 20.1 s each run). The mock worker race (one stale
  frame published just after `set_error`) is handled by `wait_until_no_cached_frame`. That helper
  is bounded (5 s deadline) and needs the slot to stay empty for 250 ms, then asserts. The stub
  camera is deterministic (atomic counter, fixed status). Timing assertions use only
  budget-expiry outcomes, never wall-clock upper bounds. PASS.
- `cargo fmt --check` and `cargo clippy -p soos-enrollment-cli --all-targets --all-features -D warnings` are clean.

### Memory, Bounds & Secrets
- The message is a static string: no device path, uid, username, frame or embedding data, which
  satisfies the #337 "no frame data or biometric material" criterion. No new allocation or logging
  was added. PASS.

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or hook changes. The test uses `libc`, which is
  already a regular dependency of the crate. PASS.

### English-Only Policy
- Code, comments, docs, matrix rows, spec/contract/auditor files and walkthrough 182 are English;
  a scan for non-ASCII and French tokens in the added lines found nothing beyond typographic symbols. PASS.

## 4. Detailed Findings & Action Items

- None (no CRITICAL, MAJOR or MINOR findings).
- **[SUGGESTION]** `AI/VERIFICATION_MATRIX.md` ECB3: during traceability, mark it verified by
  this candid review (main.rs unchanged, every error exits 1).

## 5. Final Verdict

**VERDICT: APPROVED**
