# Walkthrough 173 — Hermetic PAM None-Handle Test and Swaylock Physical Procedure

- **Date**: 2026-10-02
- **Issue**: none (follow-ups listed in PR #320). The branch is not registered in
  `BRANCH_TO_ISSUE`.
- **Branch**: `test/pam-hermetic-and-screensaver`
- **Base commit**: `4d56a01`
- **Matrix criteria**: none added (no production behaviour change)
- **ADR**: none; follows walkthrough 172 and `Docs/DISTRIBUTION_DEPLOYMENT.md` §5.3

---

## 1. Context

PR #320 left two follow-ups:

1. `crates/pam/tests/config_tests.rs::test_authenticate_with_none_handle_returns_ignore_cleanly`
   built its configuration with `PamConfig::default()`, so it used the default socket
   `/run/soos/daemon.sock`. On a developer host running `soos-daemon`, with an enrolled user in
   front of the camera, the daemon answered `Allow` (journal: `verdict=Allow
   reason=FaceMatch` at the time of the test run) and the test saw `PAM_SUCCESS`. The local
   `./save.sh` pipeline failed for a reason unrelated to the change under test; CI, which has
   no daemon, passed. Every other PAM test that can reach the daemon already points at a
   private or nonexistent socket (the fault-injection tests that keep the default socket panic
   before any socket activity).
2. `tests/physical/screensaver_test.md` §3.1 still expected `swaylock` to unlock "instantly
   (< 150ms)" after "any key", with a PAM file that included `system-auth` and added a second
   `event=password-failed` line, which the corrected §5.3 of the deployment guide refutes.

## 2. Changes

### 2.1 Test setup (assertion unchanged)

The test now sets `socket_path` to an absent socket inside a fresh `tempfile::tempdir()`, the
pattern used by the other PAM tests. The assertion (`PAM_IGNORE` from
`authenticate_with_config(None, ..)`) and its message are unchanged: the contract is the same,
only the environment it runs in is now controlled. This is a setup-only migration; no check was
removed or relaxed.

### 2.2 Physical procedure §3.1 and the behaviour matrix

- The PAM block is the distribution file (`auth include login` on Arch) with a pointer to §5.3;
  the instruction not to add a second `pam_soos.so` line replaces the duplicated event line.
- A "Behaviour to Keep in Mind" paragraph states that `swaylock` verifies only on submit and
  that `-e` drops empty submissions.
- Test Case 1 presses `Enter` on the empty field and expects an unlock within the `timeout_ms`
  budget (about 0.3–0.5 s measured), with the journal line to check.
- Test Case 2 records the faillock cost of a failed empty submission and warns against
  reaching `deny`.
- Every test case locks the screen explicitly with `swaylock -C /dev/null`, so no swaylock
  config file can set `ignore-empty-password` behind the operator's back.
- Test Case 2 also records the `PasswordFailed` event sent by the base-stack event line and
  names its journal line.
- Test Case 3 types the password with the daemon stopped, then restarts the daemon.
- New Test Case 4 (daemon running, no config file) checks that `-e` never contacts the daemon
  for an empty `Enter`, while a typed password still runs face verification first.
- The "Nominal Unlock" matrix row no longer promises "<= 150ms".

The hyprlock, GDM and other sections are unchanged (not revalidated on hardware here).

## 3. Validation

- `cargo test -p soos-pam --test config_tests`: 25 passed, with `soos-daemon` running and the
  enrolled user in front of the camera.
- `cargo test -p soos-invariants`: 378 passed (`pam_deadline_contract.rs` and `lib.rs` read
  `tests/physical/screensaver_test.md`).
- `./save.sh` and the candid review gate before the push.
