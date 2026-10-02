# Walkthrough 172 — Screen Locker PAM Behaviour (Documentation Correction)

- **Date**: 2026-10-02
- **Issue**: none (documentation-only correction found during hardware validation on the
  owner's Arch Linux laptop). The branch is not registered in `BRANCH_TO_ISSUE`.
- **Branch**: `chore/swaylock-pam-doc`
- **Base commit**: `be1dd95`
- **Matrix criteria**: none (no code, test or invariant changes)
- **ADR**: none; cites ADR 2026-09-30 "Local Session Binding for Facial `Auth`"

---

## 1. Context

`Docs/DISTRIBUTION_DEPLOYMENT.md` §5.3 promised that on a face match `pam_soos.so` returns
`PAM_SUCCESS` "within `< 150ms`, unlocking the screen locker without requiring Enter or
keyboard input". Hardware validation showed this is wrong for `swaylock`:

- With `ignore-empty-password` set, locking the screen and pressing Enter never reached the
  daemon: the journal showed no `Auth` request from the locker at all.
- `swaylock` (1.8.x, `password.c` `submit_password`) only calls `pam_authenticate` when a
  password is submitted, and returns early on an empty buffer when `ignore_empty` is set.
- A face match measured from camera auto-standby takes about 0.3–0.5 s, not < 150 ms
  (journal: camera resume → `Face verification consensus reached` with three consecutive live
  captures).

## 2. Change

§5.3 now documents:

1. How the locker service reaches the soos line (Arch: `swaylock` → `login` →
   `system-local-login` → `system-login` → `system-auth`).
2. That `swaylock` verifies only on submit, shows nothing on screen, and never scans in the
   background.
3. That Enter on an empty field runs face verification, and that a failed face then sends the
   empty password to `pam_unix.so`, which counts one `pam_faillock` failure.
4. That `ignore-empty-password` disables face verification for empty submissions.
5. The measured latency, the governing `timeout_ms` deadline, and the same-UID session rule
   that applies because `swaylock` runs as the locked user.
6. That `hyprlock` is not validated on hardware.

The false "Instant Unlock" guarantee is removed; the "Graceful Fallback" and "Zero Lockup"
guarantees are kept.

## 3. Out of Scope

The owner's personal lock screen (a patched `swaylock-plugin` with a face-only PAM service and
continuous scanning) is local desktop configuration, not part of soos, and is not shipped or
documented here.

## 4. Validation

- Documentation only: no Rust, test, packaging or CI file changes.
- `./save.sh` quality pipeline (fmt, clippy, tests, deny, candid review) run before the push.
