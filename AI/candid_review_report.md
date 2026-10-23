# Candid Review Report

- **Date**: 2026-10-23
- **Target Branch**: `fix/push-sender-system-service`
- **Base (merge-base)**: `c3ad0ee`
- **Reviewed-Diff-Fingerprint**: `69d90c05ab33836a43dba42a2ba6f33f77b87b132d1e42a1d56daeb5d32f8d1e`
- **Audited Files**: `.agents/skills/dev-workflow/references/project-facts.md`,
  `.github/workflows/ci.yml`, `AGENTS.md`, `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`,
  `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_push_sender_system_service.md`,
  `AI/auditor_constraints_push_sender_system.md`,
  `AI/walkthroughs/255_push_sender_system_service.md`, `Docs/CI_CD_AND_SECURITY.md`,
  `Docs/DISTRIBUTION_DEPLOYMENT.md`, `Docs/PACKAGING_AND_PROVISIONING.md`,
  `Docs/REMOTE_COMPANION.md`, `crates/gui/src/onboarding.rs`, `crates/push-protocol/src/lib.rs`,
  `crates/push-protocol/tests/system_socket_tests.rs`, `crates/push-sender/src/lib.rs`,
  `crates/push-sender/src/main.rs`, `crates/push-sender/tests/system_instance_tests.rs`,
  `crates/remote/src/main.rs`, `crates/remote/src/push.rs`,
  `crates/remote/tests/push_system_socket_tests.rs`, `packaging/arch/PKGBUILD`,
  `packaging/arch/soos.install`, `packaging/debian/prerm`, `packaging/rpm/soos.spec`,
  `packaging/soos-push-sender@.service`, `packaging/soos-push-sender@.socket`,
  `scripts/install.sh`, `scripts/install_remote.sh`, `scripts/setup.sh`, `scripts/uninstall.sh`,
  `tests/docker/push_sender_system_unit_test.sh`, `tests/docker/setup_root_preview_test.sh`,
  `tests/invariants/src/lib.rs`, `tests/invariants/src/push_sender_system_contract.rs`,
  `tests/invariants/src/setup_user_phase_contract.rs`

## 1. Executive Summary

The push sender moves from the user unit (which exits `226/NAMESPACE` on Ubuntu 23.10+) to a
socket-activated `DynamicUser` system template (`soos-push-sender@<uid>.socket/.service`), per
the owner decision of 2026-10-23. Second review round: round 1 raised two MAJOR findings
(setup reported the sender "done" for a non-`/usr` prefix whose unit ran a missing/stale
binary; `system_up_to_date` ignored the sender binary and units) plus two MINOR ones (rollback
leaves the socket enabled; packages did not disable instances on removal). All four are
resolved: an install.sh prefix drop-in resetting `ExecStart=` (validated prefix, journaled,
symlink-refusing, proven under real systemd with `/usr/local`), the sender binary and both units
in `system_up_to_date`, a documented and printed kept socket on companion failure, and
RPM/Arch/Debian removal hooks that only pass `soos-push-sender@<digits>.socket` names. Gates
re-run on this tree: `cargo fmt --check`, workspace clippy (all targets/features, `-D
warnings`), tests of push-protocol, push-sender, remote, gui and soos-invariants (96 suites, 0
failed), shellcheck (no finding beyond those already on `origin/main`), and
`tests/docker/push_sender_system_unit_test.sh` (all 7 cases pass, `systemd-analyze security`
exposure 1.4). Rebind after 28e2f98: three log-label strings of the branch-new PSS12 tests now use the harness's fixed role label `uid=1000` (the stub prologue always logs user calls as `uid=1000`; the previous `uid=<real uid>` failed on CI runners with uid 1001) and one unused `let uid` is removed; socket names still use `real_uid()`, matching setup's `TARGET_UID`. No assertion was dropped or loosened; soos-invariants re-run green (896 passed) with clippy clean. Remaining findings are MINOR only.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

Removed or changed pre-existing assertion lines in `target/candid_diff.patch` (all others are
additions):

- `tests/docker/setup_root_preview_test.sh` `companion_ran`: expected
  `[DONE] companion: soos-remote.service soos-push-sender.service running` becomes
  `[DONE] companion: soos-remote.service running`. Owner-approved migration (2026-10-23).
- `tests/invariants/src/setup_user_phase_contract.rs` SET24
  `test_set_u6_full_run_sets_up_the_default_on_companion`: the user-unit enable line becomes the
  root `systemctl enable --now soos-push-sender@<uid>.socket` line (plus an added ordering
  assertion); `user_unit(push-sender, active)` becomes `!user_unit(push-sender, enabled)`.
  Owner-approved.
- SET33 `test_set_u22_rollback_stops_a_unit_started_by_the_run`: the failure trigger moves from
  `STUB_USER_FAIL_UNIT=soos-push-sender.service` to `STUB_USER_DIE_UNIT=soos-remote.service`;
  the single deleted assertion is `user_unit("soos-push-sender.service", "enabled")`. Every
  soos-remote assertion (start/stop lines, inactive and enabled, restored file, exactly two
  restarts, active after rollback) is kept. Owner-approved.
- SET37 `test_set_u21_push_switched_on_later_enables_the_sender`: user-unit state assertions
  retargeted to the system socket enable line and the `<socket> is not enabled` message, with
  `STUB_SYSTEM_OFF_UNIT`. Owner-approved.
- Stub knobs `STUB_USER_DIE_UNIT` / `STUB_SYSTEM_OFF_UNIT` are inert when unset (the latter
  guarded by `-n`), so no other test can pass vacuously.
- 28e2f98 changes only branch-new PSS12 tests (`test_pss12_setup_enables_the_system_sender_not_the_user_unit`, `test_pss12_setup_disables_a_legacy_user_unit`): the same lines are still required, with the fixed `uid=1000` role label instead of the real uid; nothing is weakened.
- The round-2 commits (bca9e03..2247134) remove no test line at all; they add PSS16-PSS19
  invariants and Docker case 7. No `#[ignore]`, `should_panic`, tolerance or inline
  `mod tests` change in the patch.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Prefix `/usr/local` via setup.sh: install.sh writes
  `soos-push-sender@.service.d/prefix.conf` (`ExecStart=` reset to
  `<prefix>/libexec/soos/soos-push-sender --system-instance %i`) before the later
  `daemon-reload`; Docker case 7 proves the instance runs the `/usr/local` binary with the
  `/usr/libexec` one removed → PASS (round-1 finding 1 resolved).
- Prefix validation (`^(/[A-Za-z0-9._+-]+)+$`, no `.`/`..`, outside sandbox-hidden trees) runs
  only when the sender artifact exists and PREFIX ≠ `/usr`, as a preflight failure before any
  change. Existing harnesses that pass `/tmp/...` prefixes to setup.sh use a stub install.sh,
  AFC7 uses `/opt/soos`, packaging uses `/usr`: all existing tests still pass unchanged → PASS.
  A trailing-slash or `/home` prefix is now refused for install.sh (fail closed, clear message).
- Sender-only change or pre-unit install → `system_up_to_date` false → install.sh runs → PASS
  (round-1 finding 2 resolved).
- Client socket selection (system socket first, peer uid 0; then legacy; first existing only, no
  downgrade); explicit `push_socket_path` single candidate → PASS.

### PAM Concurrency & Deadlines
- No change under `crates/pam`. Setup systemctl calls bounded by `run_bounded` → PASS.

### Panic Safety & Fail-Closed
- `parse_run_mode`, `check_instance_identity`, `listener_from_fd`, `evaluate_system_sandbox`:
  no `unwrap/expect`, every refusal exit 78 (`RestartPreventExitStatus=78`); uid 0 instance
  exits 78 under real systemd → PASS.

### Test Integrity & Anti-Weakening
- Section 2: only the owner-approved migrations change pre-existing assertions → PASS.
- New tests were committed red first (bca9e03, f765c3c) and target plausible wrong
  implementations (missing drop-in, unsafe prefix, missing marker check in packages) → PASS.

### Memory, Bounds & Secrets
- Peer authentication: sender closes any peer whose `SO_PEERCRED` uid ≠ instance before reading;
  bob gets `EACCES` at connect, root gets no reply (Docker). Socket 0600 owned by the user in
  root-owned 0755 directories under root-only `/run`; PID 1 binds under its umask, so no
  creation/chown race. Other DynamicUser instances cannot see `/run` → PASS.
- Sandbox: strict superset of the old user unit (plus `DynamicUser`, `ProtectHome=yes`,
  `ProtectProc=invisible`, `ProcSubset=pid`, `RemoveIPC`, `KeyringMode=private`,
  `~@privileged`); DNS binds limited to the two resolved files, read-only → PASS.
- Drop-in: content built from the validated prefix only (no `%`, space, quote or newline
  possible), staged in the private `mktemp -d` backup directory, installed atomically by
  `tracked_install`, refused through a symlinked directory or file, stale-removal journaled and
  restored on rollback → PASS. Constant log strings, no secret logged → PASS.

### Supply Chain & Automation
- CI job `push-sender-unit`: SHA-pinned checkout, `persist-credentials: false`, no
  `github.event` interpolation, listed in `ci-success` needs (checked through
  `needs.*.result`). No dependency change. Package hooks only pass digit-validated instance
  names to `systemctl` → PASS.

### English-Only Policy
- All code, comments, docs and commit messages in English → PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `scripts/uninstall.sh:208-215` — removes `soos-push-sender@.service.d/prefix.conf`
  without checking the install.sh marker line, unlike the RPM/Arch/Debian hooks. The template
  unit is removed in the same run, so the effect is limited to an administrator file of that
  exact name; align with the marker check for consistency.
- **[MINOR]** `scripts/install.sh:1027-1035` — with `--prefix /usr` any existing `prefix.conf`
  is removed (journaled) regardless of the marker; same consistency remark.
- **[MINOR]** `scripts/setup.sh:1006-1015` — for a non-`/usr` prefix `system_up_to_date` does
  not compare the drop-in, so a hand-deleted drop-in is not reinstalled by a re-run (only by
  `--reinstall`).
- **[SUGGESTION]** install.sh now refuses a non-normalised `--prefix` (for example a trailing
  slash) whenever the sender is built; normalising a trailing slash in setup.sh would avoid a
  surprising refusal.

## 5. Final Verdict
**VERDICT: APPROVED**
