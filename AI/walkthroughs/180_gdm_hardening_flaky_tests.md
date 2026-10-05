# Walkthrough 180 — GDM PAM File Hardening, Enable/Status Agreement and Deterministic Flaky Tests

- **Date**: 2026-10-05
- **Issue**: GitHub #333 (GitHub-only follow-ups of #331 / PR #332, no backlog id; the squash commit carries
  `Closes #333`) — **Branch**: `fix/gdm-hardening-and-flaky-tests`
- **Matrix criteria**: GHF1–GHF9 (component `gdm-hardening-flaky-tests`); SUA4 reworded (OA-2), PFU7
  annotated (OA-3)

## 1. Context & Objectives

The review of PR #332 (walkthrough 179) left eight follow-ups:

1. `gdm enable` did not re-check `gdm-password` before its atomic rename, unlike `gdm restore` (#318).
2. `gdm enable` checked the file type by path (`lstat`) and then read it again by name.
3. The delegated-stack analysis was bounded in depth (4) but not in width: many sibling includes could
   make one `gdm enable` / `gdm status` read an unbounded number of files.
4. `status` reported `installed: false` for a pre-anchor jump past the delegation while `enable`
   succeeded without writing.
5. The target of a shared `[success=N]` soos rule was never checked (a jump past the end of the chain is a
   libpam "bad jump", `PAM_PERM_DENIED`).
6. `wait_daemon_ready.sh` printed the raw `--admin` path in its timeout line.
7. The systemd acceptance harness ordered the daemon's stdout line against PID 1's `Started` line (two
   journald inputs, logged after `READY=1`): flaky under load.
8. The 0 ms clamp test of `soos-admin test-pam` failed when the client correctly timed out before writing.

## 2. Architect Design

Spec: `AI/architect_spec_gdm_hardening_flaky_tests.md` (incl. Revision 1, owner approvals OA-1/2/3).

- `ensure_gdm_pam_line_with(pam_file, &mut dyn FnMut())` seam; `PamFileSnapshot { dev, ino, bytes, mode,
  uid, gid }`; `GdmOperation { Enable, Restore }` selects the compared fields and the message wording.
- `MAX_PAM_STACK_READS` = 32 (`ScanBudget` threaded through `scan_lines` / `scan_stack`).
- E4 in `plan_gdm_enable` from the same `AnchorScan::jump_skips_anchor` predicate `status` uses.
- `PamLine::primary_success_jump` and `jump_lands_in_stack(rest, n)`.
- `sd_notify::notify_to_stamped` (test seam); `sd_notify::notify_ready()` returns the send stamp
  (owner approval OA-4; `notify_ready_stamped()` kept as an alias); readiness message
  `Reported readiness to systemd (ready_sent_monotonic_us=<us>|unknown)`.

## 3. Plan Evaluation

`AI/plan_evaluator_report.md` round 2: APPROVED with R2-1 (snapshot keeps the unmasked mode), R2-2 (losing
concurrent enable), R2-3 (ADR agreement wording), R2-4 (backup test hooks).

## 4. Tester Contract

`AI/tester_contract_gdm_hardening_flaky_tests.md`: 12 `enable_race_tests`, `gdm_hardening_tests` (GHF2–GHF5),
`pam_stack::ghf_tests` (GHF3, GHF5), `sd_notify_stamp_tests` (GHF7), `gdm_hardening_flaky_tests_contract`
(GHF2, GHF6–GHF9). Pre-existing test changes: OA-1 (IGF14 fixture), OA-2 (harness part 2 and its invariant
needle), OA-3 (bounded retry in `capture_deadline_with`). OA-4 (owner, during Phase 4): the new GHF7
invariant no longer forbids `sd_notify::notify_ready()` in `main.rs` (required by the pre-existing
`systemd_readiness_tests` and `presence_unlock_contract`); `notify_ready()` itself became the stamped variant.
R2-2 pinned to "keep the backup when the re-read
file holds a managed block".

## 5. Auditor Constraints

`AI/auditor_constraints_gdm_hardening_flaky_tests.md`: CLEARED, C1–C17. Recommended (optional) `O_NOCTTY`
on `pam_stack::read_bounded_utf8`: applied.

## 6. Implementation

- **`crates/admin-cli/src/gdm.rs`**: the enable path opens `gdm-password` once with
  `O_NOFOLLOW | O_NOCTTY | O_NONBLOCK | O_CLOEXEC` (`read_pam_file_snapshot(.., Enable)`), plans from those
  bytes, and passes `before_rename` + `ensure_pam_file_unchanged(.., Enable)` to `write_atomic_checked`.
  `publish_backup` writes an exclusive temporary file and publishes it with `fs::hard_link` (`EEXIST` ⇒ not
  this run's backup); `discard_created_backup` removes only the run's `(dev, ino)`, keeps it when the
  re-read file holds a managed block or cannot be re-read, and appends the E1b note for a replaced backup or
  a removal error. `regular_file_metadata` and the unused `write_atomic` wrapper are gone. E4 refuses a
  pre-anchor jump past the delegation on the shared branch (after the crossing check, before `Ok(None)`).
  The restore path compares identity and bytes only and keeps its message byte-identical.
- **`crates/admin-cli/src/pam_stack.rs`**: budget decrement with `checked_sub` (E3), `scan_lines` over
  `&[&str]`, E5 when a shared `[success=N]` jump does not land in the same file; `jump_lands_in_stack`
  compares `skipped == n` before a `checked_add` (no overflow for `n = usize::MAX`).
- **`crates/daemon/src/sd_notify.rs`**: one private `send_notification(socket, message, clock)`; the clock
  (`pipeline::current_monotonic_nanos`) is read after validation and socket set-up, right before
  `send_to_addr`; a clock error only drops the stamp. `notify_ready()` returns
  `(NotifyOutcome, Option<u64>)` (OA-4) and **`main.rs`** calls `sd_notify::notify_ready()` and logs the two
  exact message shapes.
- **`scripts/wait_daemon_ready.sh`**: the timeout line prints `${q_admin}`.
- Docs: ADR in `AI/DECISIONS.md`, `Docs/DISTRIBUTION_DEPLOYMENT.md` §2.1, `AI/ARCHITECTURE.md`,
  `Docs/DAEMON.md`, project facts, matrix rows GHF1–GHF9, SUA4 and PFU7.

## 7. Candid Review

Runs after this phase on the whole diff; the fingerprint and verdict are recorded in
`AI/candid_review_report.md`.

## 8. Verification Results

- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked --all-features -- -D warnings`:
  clean.
- Full workspace `cargo test --workspace --locked --all-features`: green (run twice after OA-4).
- `tests/docker/systemd_unit_acceptance_test.sh --models auto` (host models): exit 0; part 2 logged
  `bind -> READY=1 (sent 25898629223 us) -> active/Started (25898629319 us)`.
- Every GHF contract test, the OA-2 invariant
  (`systemd_unit_acceptance_contract::test_systemd_acceptance_harness_asserts_ready_ordering_and_clean_stop`)
  and the pre-existing `gdm_*`, `restore_race_tests`, `sd_notify_tests` suites pass.
- Timing-sensitive tests (`cli_deadline_json_tests`, `gdm_hardening_tests` FIFO bound) repeated 10 times:
  green.

## 9. Known Limitations / Follow-ups

- Residual windows (root-only, inherent without locking): a change made between the pre-rename re-check
  and `rename(2)` is replaced; a backup replaced between its identity check and the unlink is not detected.
- `gdm status` false positive (availability only): a shared jump landing in its file on a rule that refuses
  (`pam_deny.so`, `pam_faillock.so authfail`) still reports `installed: true`.
- A symlinked `gdm-password` is followed by `status` (like libpam) and refused by `enable`.
