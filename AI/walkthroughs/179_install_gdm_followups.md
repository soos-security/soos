# Walkthrough 179 — GDM Shared soos Rule, Stream-Start Presence Settle and Install Follow-ups

- **Date**: 2026-10-05
- **Issue**: GitHub #331 (GitHub-only review follow-ups of PR #330, no backlog id; the squash commit carries
  `Closes #331`) — **Branch**: `fix/install-gdm-followups`
- **Matrix criteria**: IGF1–IGF19 (component `install-gdm-followups`)

## 1. Context & Objectives

PR #330 (walkthrough 178) left review follow-ups, and the owner's Arch host showed a double face
verification at the GDM unlock prompt. Issue #331 groups three themes:

1. **Install / readiness / status (§1)**: the `--build` preflight did not refuse read-only files and gave
   the wrong `sudo chown -R` advice for a user-owned unreadable subtree; `wait_daemon_ready.sh` claimed a
   30 s bound while its real worst case is about 37.5 s, dropped the stderr of failed `soos-admin status`
   attempts and waited the whole timeout for an admin binary that cannot run; a relative
   `CARGO_TARGET_DIR` was resolved against the current directory instead of the checkout (P-1);
   `soos-admin status` called `systemctl show` without a bound (P-2); the presence wake settle of #329
   only covered a camera woken by the scan itself, not one woken by a PAM request shortly before.
2. **Physical procedure (§2)**: the §6 rollback `sed` edited locker/sudo files that carry no soos rule and
   would replace authselect symlinks by regular files; §3.3 Test Case 2 gave no way to find the
   transient GDM worker.
3. **GDM double verification (§3)**: on Arch the managed block of `gdm-password` was inserted before
   `system-auth` received the packaged primary soos rule, so one GDM attempt made two daemon requests
   (and re-running `gdm enable` refused with `unclassified auth rule 'pam_soos.so'`).

## 2. Architect Design

Spec: `AI/architect_spec_install_gdm_followups.md` (incl. Revision 1).

- **GDM** (`crates/admin-cli/src/pam_stack.rs`, `gdm.rs`, `main.rs`): `PamLine::is_primary_soos_rule`
  (auth `pam_soos.so`, no `event=`/`service=`, control `sufficient` or a bracket whose `success` is `done`
  or a jump `N >= 1` with every other action `ignore`); `Scan::SharedSoos(stack)`; `DelegatedAuth
  { Gates(Vec<String>), SharedSoosRule { stack } }` returned by `delegated_auth`, which replaces
  `delegated_gates` (same refusals, same `MAX_PAM_INCLUDE_DEPTH` = 4); `EnablePlan { Insert,
  RemoveRedundant }`; `GdmStatus::shared_stack: Option<String>` (table line `Shared soos Rule:`, JSON
  `shared_stack`).
- **Camera** (`crates/camera-v4l`): defaulted trait method `CameraManager::stream_started_mono_ns()
  -> Option<u64>` (`None` by default), V4L stamp taken right after `start_stream` succeeds and cleared in
  `SupervisorShared::withdraw_frames`, mock hook `MockCameraManager::set_stream_started_mono_ns`.
- **Presence** (`crates/daemon/src/presence/`): `PresenceSettle { not_before_ns, wait_ms }` and
  `presence_settle_window(woke, start_ns, stream_started_ns, settle_ms)` =
  `max(woke ? start + settle : 0, stream + settle)` with saturating arithmetic; `PRESENCE_WAKE_SETTLE_MS`
  stays 1000.
- **Admin status**: `SYSTEMCTL_SHOW_TIMEOUT_MS` = 1000 (`crates/admin-cli/src/status.rs`),
  `MAX_SYSTEMCTL_OUTPUT_BYTES` = 4096, seam `inspect_systemd_unit_with`.
- **Scripts**: `resolve_cargo_target_dir` in `scripts/install.sh` (lexical join against the checkout),
  four ordered preflight checks; `ADMIN_STDERR_TAIL_MAX` = 1024 and exit 126/127 fail-fast in
  `scripts/wait_daemon_ready.sh`.
- **Invariants touched**: fail closed (every scan error still refuses, `gdm status` fails closed to
  `installed: false`); the GDM gate guarantee changes scope but not in practice (gates after the shared
  rule are governed by that stack, as for `sudo`/`login`, which was already the effective behaviour);
  PAM path untouched by the settle; `crates/pam`, `crates/protocol`, `crates/policy`, dispatcher,
  consensus, inference, daemon config, `packaging/pam/**`, `tests/docker/**`, `tests/distro/**` unchanged.

### Owner decisions

- **GDM adds no block on a shared stack**: when the delegated stack already reaches a primary soos rule,
  `gdm enable` writes nothing and removes a redundant block (or bare/legacy line) left by an earlier
  release; it never creates a backup and leaves an existing `.soos-backup` byte-identical so `gdm restore`
  and its stale-backup `--force` protection keep working. `gdm.disable` keeps working because the shared
  rule reads `PAM_SERVICE`.
- **1000 ms trade-off**: GDM through the shared rule uses that rule's deadline (module default
  `DEFAULT_TIMEOUT_MS` = 1000 ms, no packaged rule sets `timeout_ms=`) instead of `timeout_ms=2500`; a
  GDM unlock that needs a camera wake from auto-standby may fall back to the password. `GDM_PAM_LINE`
  keeps `timeout_ms=2500` for stacks without a shared rule.
- **SpyCamera setup field**: the owner accepted one setup-only harness extension in
  `crates/daemon/tests/common/mod.rs` (`SpyCamera::stream_started_ns: AtomicU64`, 0 = `None`, its
  initialiser and the trait override), because the spy restamps frames in the test-clock domain and the
  mock setter lives in the real clock domain. No assertion was touched.

ADR: `AI/DECISIONS.md` "[2026-10-05] GDM Reuses a Shared Primary soos Rule; Presence Settle Keyed on the
Camera Stream Start; Install Readiness Follow-ups", plus an amendment note on the 2026-09-30 "GDM PAM
Stack Placement" entry.

## 3. Plan Evaluation

`AI/plan_evaluator_report.md`: round 1 REVISION_REQUIRED (1 MAJOR: `AI/ARCHITECTURE.md` drift on the
GDM deadline and §5 paragraph; 6 MINOR: lazy `VIDIOC_STREAMON` wording, Fedora example label, gate
guarantee wording, undocumented `gdm status` false positives, P-2 read-after-exit blocking, test power).
Round 2 **APPROVED** after Revision 1; one non-blocking wording note (R2-1: the screensaver §6 bullet
must say "there may be no backup", since a kept backup still restores) was applied.

## 4. Tester Contract

`AI/tester_contract_install_gdm_followups.md` (Red kinds A = assertion, C = compile error on the new API
only, G = guard that must stay green).

| Rows | Tests |
|---|---|
| IGF1–IGF5, IGF9, IGF10, IGF12, IGF13, IGF19 | `tests/invariants/src/install_gdm_followups_contract.rs` (20 tests) |
| IGF6, IGF7 | `crates/daemon/tests/presence_stream_settle_tests.rs` (17 tests) |
| IGF8 | `crates/camera-v4l/src/v4l_impl/supervisor_tests.rs` (3), `crates/camera-v4l/tests/stream_start_stamp_tests.rs` (3) |
| IGF11 | `crates/admin-cli/src/status.rs::systemctl_bound_tests` (8) |
| IGF14, IGF15, IGF17, IGF18 | `crates/admin-cli/tests/gdm_shared_rule_tests.rs` (21), `crates/admin-cli/src/pam_stack.rs` tests (8) |
| IGF16 | `crates/admin-cli/tests/gdm_shared_status_tests.rs` (7) |

Red evidence: `gdm_shared_rule_tests` 14 failed on assertions and 7 guards passed (confirmed by the
auditor); the other suites failed on E0425/E0432/E0433/E0599/E0609 naming only the new API, or on
assertions for the shell-driven checks.

Contract deviations from the spec: IGF9 checks that dispatcher and consensus never name the settle items
instead of diffing them against `b05477d` (the byte-identity check stays with the auditor); IGF16 lives
in its own file so its compile error did not mask IGF14/15/17/18 assertions; IGF7's long-running-stream
case asserts the first PAD call < 500 ms (stricter than IWP10); IGF19 also requires
`stream_started_mono_ns` in `Docs/CAMERA_V4L_CRATE.md` and `1000 ms` in the deployment doc and, per R2-1,
"may be no backup".

Migrated tests: none. Only the owner-accepted `SpyCamera` setup field and a
`clippy::arithmetic_side_effects` entry in the existing `pam_stack.rs` tests `allow` list (lint
attribute only).

## 5. Auditor Constraints

`AI/auditor_constraints_install_gdm_followups.md`, verdict **CLEARED**, constraints C1–C23:

- C1–C4 (no panic paths, named reader thread with handled spawn error, child always reaped, 4096-byte
  bounded read with `recv_timeout`): met in `status.rs::inspect_systemd_unit_with`; IGF11.
- C5–C8 (conservative `is_primary_soos_rule`, classification order, stack name only from `valid_name`,
  `plan_gdm_enable` step order with one shared anchor-scan helper): met by `pre_credential_scan` /
  `AnchorScan`, shared by `plan_gdm_enable` and `shared_soos_stack`; IGF14–IGF18.
- C9–C10 (`RemoveRedundant` only through `write_atomic`, never touches the backup; `gdm status` fails
  closed): IGF15, IGF16.
- C11–C16 (frozen paths unchanged, defaulted trait method read only by presence, V4L stamp ordering and
  withdrawal without new `unsafe`, mock setter, worker ordering and budget, one `debug!` with
  `settle_ms`/`cause` only): IGF6–IGF9, IWP11, log audit.
- C17–C20 (single `CARGO_TARGET_DIR` resolution passed through the environment, grouped `find`
  alternation, in-memory capture and 126/127 fail-fast, documented bound): IGF1–IGF5, IGF10.
- C21–C23 (no new dependency, English docs and kept needles, tests immutable plus a 10× flakiness run of
  the timing-sensitive tests).

## 6. Implementation

- `crates/admin-cli/src/pam_stack.rs`: `is_primary_soos_rule`, `Scan::SharedSoos`, `DelegatedAuth`,
  `delegated_auth`.
- `crates/admin-cli/src/gdm.rs`: `refuse_continuations`, `pre_credential_scan` returning `AnchorScan`
  (single source of truth), `EnablePlan::{Insert, RemoveRedundant}`, `shared_soos_stack`,
  `include_dir_of`, `GdmStatus::shared_stack`; `main.rs` prints `  Shared soos Rule:  <stack>`.
- `crates/admin-cli/src/status.rs`: bounded `systemctl show` (poll `try_wait` every 10 ms, kill + wait on
  expiry, helper reader thread, unknown triple on any failure).
- `crates/camera-v4l/src/{manager,mock,v4l_impl}.rs`: trait default, mock `AtomicU64` hook, V4L
  `stream_started_ns: Arc<AtomicU64>`; `open_and_stream` now takes `&SupervisorShared` instead of five
  separate references (keeps clippy's argument limit).
- `crates/daemon/src/presence/{mod,worker}.rs`: `PresenceSettle`, `presence_settle_window`; the worker
  reads the stamp after the wake and `start_ns`, and logs `cause = "presence_wake" | "stream_start"`.
- `scripts/install.sh`: `resolve_cargo_target_dir`, `CARGO_TARGET_DIR` / `SOOS_CARGO_TARGET_DIR` passed
  to the build, four ordered preflight checks; `scripts/wait_daemon_ready.sh`: documented bound,
  last-stderr tail, 126/127 fail-fast.
- Docs: `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, `README.md`, `Docs/DISTRIBUTION_DEPLOYMENT.md` §2.1,
  `Docs/PACKAGING_AND_PROVISIONING.md` (readiness bound, preflight, and, added in this phase, the
  `SYSTEMCTL_SHOW_TIMEOUT_MS` bound of `soos-admin status`), `Docs/DAEMON.md`,
  `Docs/CAMERA_V4L_CRATE.md`, `tests/physical/screensaver_test.md` §3.3 and §6,
  `.agents/skills/dev-workflow/references/project-facts.md`.

## 7. Candid Review

Runs after this phase on the whole diff (including these docs); the fingerprint and verdict are
recorded in `AI/candid_review_report.md`.

## 8. Verification Results

- `cargo test --locked -p soos-admin-cli --all-features igf`: 16 lib unit tests (8 `pam_stack`, 8
  `systemctl_bound_tests`), 21 `gdm_shared_rule_tests`, 7 `gdm_shared_status_tests` passed.
- `cargo test --locked -p soos-daemon --all-features --test presence_stream_settle_tests`: 17 passed.
- `cargo test --locked -p soos-camera-v4l --all-features igf`: 3 supervisor unit tests, 3
  `stream_start_stamp_tests` passed.
- `cargo test -p soos-invariants --locked --all-features`: green, including
  `install_gdm_followups_contract` (20 tests) and the matrix citation invariants over the new IGF rows.
- `python3 scripts/sync_issue.py --check`: passes (the branch is GitHub-only and not registered).
- Full workspace test suite: green (developer gate).

## 9. Known Limitations / Follow-ups

- **Flaky pre-existing test**: `soos-admin-cli::cli_deadline_json_tests::test_simulate_pam_auth_zero_timeout_is_clamped_to_pam_minimum`
  occasionally fails under full-suite load (timing-sensitive; not touched by this change).
- **`gdm status` accuracy**: documented false negative (a stack `enable` refuses reports
  `installed: false`) and one fail-closed false positive (unchecked target of a shared `[success=N]`
  jump); a pre-anchor jump landing past the delegation now reports `installed: false` (candid review
  2026-10-05 follow-up); confirm on hardware with §3.3 of the screensaver procedure.
- **Candid review 2026-10-05 (MAJOR, fixed)**: removing a redundant managed block now runs the
  existing `[...=N]` jump-crossing check first. A crossing jump coexisting with a block was written
  with the block rules counted, so removing them would retarget it (possibly past the delegation,
  failing open); `enable` refuses with the jump error and writes nothing
  (`gdm_shared_rule_tests::test_igf18_block_removal_with_crossing_jump_is_refused`). The stderr of
  each `soos-admin status` attempt in `wait_daemon_ready.sh` is now bounded at the source
  (`tail -c 4096`) before the 1024-character tail.
- **Pre-existing auditor findings (not introduced here)**:
  1. `gdm enable` (`Insert`, and `RemoveRedundant`) does not re-check the PAM file before the rename,
     unlike `gdm restore` (GitHub #318): a concurrent administrator edit between read and rename is lost.
     Follow-up: reuse `write_atomic_checked` + `ensure_pam_file_unchanged`.
  2. `ensure_gdm_pam_line` checks `symlink_metadata` and then reads through `read_bounded_utf8`, which
     follows symlinks: a check/use window on root-owned `/etc/pam.d` (root-only threat).
  3. `scan_stack` bounds include depth (4) but not width: thousands of `include` lines can cause many
     bounded reads; root-controlled files only, now also reached by `gdm status`. Follow-up: a total-read
     budget.
  4. `wait_daemon_ready.sh` prints the `--admin` path unquoted in the existing timeout line (kept verbatim
     for IWP1).
- **Residual (presence)**: a stream restarted between the stamp read and the consensus is not re-read;
  frames of the restarted stream are withdrawn until ready and the next scan uses the new stamp.
