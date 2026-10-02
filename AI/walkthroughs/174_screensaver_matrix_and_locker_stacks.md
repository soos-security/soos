# Walkthrough 174 — Physical Procedure: Behaviour Matrix, Locker and Login Stacks

- **Date**: 2026-10-02
- **Issue**: none (follow-ups listed in PR #321). The branch is not registered in
  `BRANCH_TO_ISSUE`.
- **Branch**: `test/screensaver-matrix-and-locker-stacks`
- **Base commit**: `b4c9556`
- **Matrix criteria**: none added (no production behaviour change; BACKLOG #31.4 / matrix PH4
  only require the document to cover these services)
- **ADR**: cites 2026-09-30 "Local Session Binding for Facial `Auth`" and "GDM PAM Stack
  Placement"

---

## 1. Context

After PR #321 fixed the `swaylock` section, the rest of `tests/physical/screensaver_test.md`
still disagreed with the code, the ADRs and the deployment guide:

- §3.2 (`hyprlock`), §3.4 (`login`) and §3.5 (`sudo`) told the operator to add an explicit
  `pam_soos.so` line on top of a base stack that already includes one, so a face that does
  not match would be tried twice. §3.3 (GDM) showed a hand-written block instead of the block
  `soos-admin gdm enable` manages (gates, `timeout_ms=2500`).
- §3.3 Test Case 1 and §3.4 Test Case 1 expected a password-less **initial** greeter or TTY
  login. ADR 2026-09-30 "Local Session Binding for Facial `Auth`" refuses it by design (the
  caller is in no session of the target user); on the owner's laptop the daemon journal shows
  `Local-session policy refused auth request ... reason="caller_session_unresolved"` for the
  first GDM login after a reboot.
- The §4 matrix still said "Prompts for password" (wrong for `swaylock`, which only shows a
  failure state) and fixed figures (`<= 5ms`, `<= 2ms`, `> 250ms`) that ignore the 1000 ms
  default `timeout_ms`; §3.5 expected the `sudo` prompt "within 250ms".

## 2. Changes

- **§3.2 hyprlock**: keep the packaged PAM file, no extra soos line. Marked not validated on
  hardware; Test Case 1 now records when face verification runs (lock start or `Enter`) from
  the `Rendered authentication response` journal line, and a password fallback case is added.
- **§3.3 GDM**: the PAM block is managed by `soos-admin gdm enable` (pointer to deployment
  guide §2.1 and `gdm status`). Test Case 1 expects the initial greeter login to ask for the
  password, with the `caller_session_unresolved` journal line. Test Case 2 is the screen unlock
  (`Super+L`), which depends on the reauthentication worker running in the user's session
  scope. The procedure asks the operator to record whether one GDM attempt produces one or two
  daemon requests when the base stack also carries a soos line (Arch).
- **§3.4 login**: keep the distribution file; Test Case 1 expects the password prompt and the
  same policy refusal; Test Case 2 is a plain password login.
- **§3.5 sudo**: keep the distribution file; run from a local desktop terminal after
  `sudo -k`; the fallback prompt appears within the `timeout_ms` budget.
- **§4 matrix**: rows say "Falls back to the password" and express timing through
  `timeout_ms` (the daemon-stopped row keeps "at once": the connection is refused).
- **§6 rollback**: adds `soos-admin gdm disable` / `gdm restore` for GDM.

## 3. Open Question Recorded (not changed here)

On Arch, `soos-admin gdm enable` puts a soos line in `gdm-password` before
`include system-local-login`, and `system-auth` carries the primary soos line too. A face that
does not match at the managed block is then verified a second time by the base-stack line.
Whether that second attempt is intended (it also consumes a second rate-limit attempt) is a
design question for a separate issue; the procedure only asks to record it.

## 4. Validation

- Documentation only; no Rust, test code, packaging or CI change.
- `cargo test -p soos-invariants`: 378 passed (`pam_deadline_contract.rs` and `lib.rs` read this
  file).
- `./save.sh` and the candid review gate before the push.
