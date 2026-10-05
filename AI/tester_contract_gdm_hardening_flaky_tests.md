# Tester Contract — GitHub #333 (GDM hardening, enable/status agreement, flaky tests)

- **Spec**: `AI/architect_spec_gdm_hardening_flaky_tests.md` (incl. Revision 1, Owner Approvals OA-1/2/3)
- **Plan evaluation**: `AI/plan_evaluator_report.md` round 2 APPROVED; R2-1, R2-2, R2-4 covered by tests below
  (R2-3 is doc wording, checked at traceability).
- **Branch**: `fix/gdm-hardening-and-flaky-tests` (not committed by the tester)
- **Red commands** (all `--locked --all-features`):
  - `cargo test --locked -p soos-admin-cli --all-features --lib -- ghf1 ghf3 ghf5`
  - `cargo test --locked -p soos-admin-cli --all-features --test gdm_hardening_tests`
  - `cargo test --locked -p soos-daemon --all-features --test sd_notify_stamp_tests`
  - `cargo test --locked -p soos-invariants --all-features -- ghf systemd_acceptance_harness_asserts_ready`
- `cargo fmt --all -- --check` and `cargo clippy --locked -p soos-admin-cli -p soos-daemon -p soos-invariants
  --all-targets --all-features -- -D warnings` are clean. Every pre-existing test of `soos-admin-cli`,
  `soos-daemon` (`sd_notify_tests`, `systemd_readiness_tests`) and `soos-invariants` still passes, except the
  OA-2 invariant (red by design until `main.rs` logs the new shapes).

## Phase 2 scaffolding (production, to be replaced by the developer)

Added only so that every test compiles and fails on an **assertion** (spec API, wrong behaviour):

| Item | File | Stub behaviour |
|---|---|---|
| `ensure_gdm_pam_line_with(pam_file, &mut dyn FnMut())` | `crates/admin-cli/src/gdm.rs` | Former `ensure_gdm_pam_line` body; the hook is ignored (never called). `ensure_gdm_pam_line` delegates with a no-op |
| `pub const MAX_PAM_STACK_READS: usize = 32` (+ re-export in `gdm.rs`) | `crates/admin-cli/src/pam_stack.rs` | Spec value; not enforced yet |
| `PamLine::primary_success_jump()` | `crates/admin-cli/src/pam_stack.rs` | Returns `None`; carries `#[allow(dead_code, reason = …)]` the developer must remove once `scan_lines` uses it |
| `notify_to_stamped`, `notify_ready_stamped` | `crates/daemon/src/sd_notify.rs` | Delegate to `notify_to`, stamp always `None` |

## Contract table

| GHF | Test (path::name) | Red evidence (observed failure) |
|---|---|---|
| GHF1 | `crates/admin-cli/src/gdm.rs::enable_race_tests::test_ghf1_enable_aborts_when_file_is_edited_before_rename` | `assertion left == right failed: the hook runs exactly once, before the rename` (left 0, right 1) |
| GHF1 | `…::test_ghf1_enable_aborts_when_file_is_replaced_by_same_bytes` | hook calls 0 ≠ 1 |
| GHF1 | `…::test_ghf1_enable_aborts_when_file_is_deleted_before_rename` | hook calls 0 ≠ 1 |
| GHF1 (R2-1) | `…::test_ghf1_enable_aborts_on_chmod_before_rename` | hook calls 0 ≠ 1 (then: E1 expected, concurrent mode 0o600 kept) |
| GHF1 (R2-1) | `…::test_ghf1_enable_of_group_writable_file_passes_the_recheck` | `the re-check runs once before the rename` 0 ≠ 1 (then: unchanged 0o664 file must pass, written/backup mode 0o644) |
| GHF1 | `…::test_ghf1_enable_remove_redundant_aborts_and_keeps_backup` | hook calls 0 ≠ 1 |
| GHF1 (R2-4) | `…::test_ghf1_enable_abort_keeps_preexisting_backup` | hook calls 0 ≠ 1 (then: backup bytes + inode kept, exact E1 without E1b) |
| GHF1 (R2-4) | `…::test_ghf1_enable_never_follows_or_removes_a_backup_symlink` | `abort=true`: hook calls 0 ≠ 1 (abort and success cases: symlink and its target untouched, no temp file) |
| GHF1 (R2-4) | `…::test_ghf1_enable_abort_leaves_a_replaced_backup_and_reports_it` | hook calls 0 ≠ 1 (then: E1 + E1b suffix `… could not be removed: it was replaced`, replacement kept) |
| GHF1 (R2-2) | `…::test_ghf1_losing_enable_keeps_backup_when_file_holds_managed_block` | hook calls 0 ≠ 1 (then: E1, backup with pristine bytes kept) — see note R2-2 |
| GHF1 | `…::test_ghf1_enable_hook_runs_once_per_write_and_never_without_write` | `Insert: one re-check before the rename` 0 ≠ 1 |
| GHF1 | `…::test_ghf1_restore_message_is_unchanged` | Green guard (restore E1 text byte-identical) |
| GHF2 | `tests/invariants/src/gdm_hardening_flaky_tests_contract.rs::test_ghf2_enable_reads_the_gdm_file_through_one_descriptor` | `regular_file_metadata (path-based lstat before the read) must be gone` |
| GHF2 | `crates/admin-cli/tests/gdm_hardening_tests.rs::test_ghf2_enable_refuses_a_symlinked_gdm_file_with_kept_text` | Green guard (exact kept text) |
| GHF2 | `…::test_ghf2_enable_refuses_a_fifo_without_blocking` | Green guard (5 s bound, exact text) |
| GHF2 | `…::test_ghf2_enable_refuses_oversized_non_utf8_and_missing_files_with_kept_texts` | Green guard (exact texts) |
| GHF2 (R2-1) | `…::test_ghf2_enable_accepts_group_writable_file_and_writes_0644` | Green guard (fails if the snapshot stores the masked mode) |
| GHF3 | `crates/admin-cli/src/pam_stack.rs::ghf_tests::test_ghf3_max_pam_stack_reads_is_32` | Green (constant scaffolded at spec value) |
| GHF3 | `…::ghf_tests::test_ghf3_delegated_auth_accepts_32_stack_reads` | Green guard (32 accepted) |
| GHF3 | `…::ghf_tests::test_ghf3_delegated_auth_refuses_the_33rd_stack_read` | `33 stack reads must be refused: Gates([])` |
| GHF3 | `…::ghf_tests::test_ghf3_nested_reads_count_and_a_late_shared_rule_is_refused` | `the shared stack opened 33rd must be refused` |
| GHF3 | `crates/admin-cli/tests/gdm_hardening_tests.rs::test_ghf3_constant_is_reexported_with_value_32` | Green (re-export) |
| GHF3 | `…::test_ghf3_enable_refuses_33_stack_reads` | `gdm enable must refuse this stack` (returned `Ok`) |
| GHF3 | `…::test_ghf3_shared_rule_on_the_33rd_read_is_refused_and_not_installed` | `status must report installed: false` (reported `installed: true, shared_stack: Some("soos-auth")`) |
| GHF3 | `…::test_ghf3_shared_rule_on_the_32nd_read_is_installed` | Green guard |
| GHF4 | `…::test_ghf4_status_reports_not_installed_when_a_jump_skips_the_shared_stack` | Green guard (issue item 4: status test of `jump_skips_anchor`) |
| GHF4 | `…::test_ghf4_enable_refuses_a_jump_that_skips_the_shared_stack` | `gdm enable must refuse this stack` (returned `Ok`) |
| GHF4 | `…::test_ghf4_enable_ok_implies_installed_and_shared_stack_implies_ok` | `jump skips anchor: enable returned Ok but status says not installed` |
| GHF5 | `crates/admin-cli/src/pam_stack.rs::ghf_tests::test_ghf5_primary_success_jump_reads_the_decimal_jump` | `left: None, right: Some(4)` |
| GHF5 | `…::ghf_tests::test_ghf5_jump_past_the_end_of_the_file_is_refused` (original IGF14 fixture) | `a jump target outside the file must be refused (E5): SharedSoosRule { stack: "inner" }` |
| GHF5 | `…::ghf_tests::test_ghf5_target_must_be_the_rule_after_the_n_skipped_ones` | same E5 expectation (off-by-one guard) |
| GHF5 | `…::ghf_tests::test_ghf5_jump_over_a_delegation_is_refused` | same (include, substack, `@include`, `INCLUDE`) |
| GHF5 | `…::ghf_tests::test_ghf5_malformed_unknown_or_continued_line_in_span_is_refused` | same (malformed, unknown type `authx`/`foo`, continued line) |
| GHF5 | `…::ghf_tests::test_ghf5_counting_rules_of_the_jump_span` | same (non-auth lines must not count; `-auth` must count, comments/blank skipped) |
| GHF5 | `…::ghf_tests::test_ghf5_done_and_sufficient_need_no_target` | Green guard |
| GHF5 | `…::ghf_tests::test_ghf5_packaged_jump_targets_are_accepted` | Green guard (Arch packaged `[success=4]`, Debian `[success=2]`) |
| GHF5 | `crates/admin-cli/tests/gdm_hardening_tests.rs::test_ghf5_enable_refuses_a_shared_jump_past_the_end_of_its_file` | `gdm enable must refuse this stack` (returned `Ok`) |
| GHF5 | `…::test_ghf5_enable_refuses_a_shared_jump_over_an_include` | same |
| GHF6 | `tests/invariants/src/gdm_hardening_flaky_tests_contract.rs::test_ghf6_readiness_timeout_line_quotes_the_admin_path` | `the timeout line must print the %q form of the --admin path` (printed `'/tmp/…/soos admin$HOME status'`) |
| GHF6 | `…::test_ghf6_no_message_prints_a_raw_admin_path` | lists the line-228 `echo` with raw `${ADMIN_BIN}` |
| GHF7 | `crates/daemon/tests/sd_notify_stamp_tests.rs::test_ghf7_notify_to_stamped_returns_the_monotonic_send_time` | `a sent READY=1 must carry its CLOCK_MONOTONIC send time` |
| GHF7 | `…::test_ghf7_notify_to_stamped_without_supervisor_has_no_stamp`, `…::test_ghf7_notify_to_stamped_keeps_the_notify_to_validation`, `…::test_ghf7_notify_ready_stamped_without_supervisor_is_a_no_op` | Green guards |
| GHF7 | `…::test_ghf7_harness_parse_extracts_send_time_from_ansi_compact_line` (optional rendering test, spec §4.7.4) | Green: the exact harness `grep`/`sed` pipeline extracts the digits from an ANSI compact line and nothing from the `unknown` shape |
| GHF7 | `tests/invariants/src/gdm_hardening_flaky_tests_contract.rs::test_ghf7_daemon_logs_the_ready_send_time_in_the_message_text` | `crates/daemon/src/main.rs must report readiness through notify_ready_stamped` |
| GHF7 | `…::test_ghf7_harness_orders_ready_send_time_against_active_enter_timestamp` | Green after the OA-2 harness change (tester-owned) |
| GHF7 (OA-2) | `tests/invariants/src/systemd_unit_acceptance_contract.rs::test_systemd_acceptance_harness_asserts_ready_ordering_and_clean_stop` | `crates/daemon/src/main.rs must still log Reported readiness to systemd (ready_sent_monotonic_us={us})` |
| GHF8 (OA-3) | `crates/admin-cli/tests/cli_deadline_json_tests.rs::test_simulate_pam_auth_zero_timeout_is_clamped_to_pam_minimum` (helper `capture_deadline_with` / `capture_attempt`) | Test-only change, green (not a Red item: no production change, spec §4.8.2) |
| GHF8 | `tests/invariants/src/gdm_hardening_flaky_tests_contract.rs::test_ghf8_clamp_test_retry_is_bounded_and_assertion_unchanged` | Green guard (bound 20, Required = 1 attempt, assertion text unchanged) |
| GHF9 | `…::test_ghf9_docs_and_adr_describe_the_changes` | `AI/DECISIONS.md must carry the GitHub #333 ADR (MAX_PAM_STACK_READS)`; also checks §2.1 (`changed concurrently`, `O_NOFOLLOW`, `MAX_PAM_STACK_READS`, `bad jump`, `pam_deny.so`), ARCHITECTURE `#333`, DAEMON.md both message shapes, project-facts row, matrix rows GHF1–GHF9, SUA4 mentions `ActiveEnterTimestampMonotonic`, walkthrough 180 |

### Note R2-2 (spec does not decide)

The spec (§4.1 step 5) removes the backup whenever its `(dev, ino)` is the run's own; the plan evaluator's R2-2 left
"skip the removal when the re-read file holds a managed block" or "state it as a residual" open. As instructed, the
contract pins the **non-removal** option: `test_ghf1_losing_enable_keeps_backup_when_file_holds_managed_block`
requires that a losing run whose pre-rename re-read finds a soos managed block (`GDM_BLOCK_BEGIN … GDM_BLOCK_END`)
returns E1 and leaves the pristine backup in place. Whether an E1b suffix is appended in that case is not asserted.
The other abort tests (edit, same-bytes replacement, delete, chmod) re-read a file without a managed block and
require the run's own backup to be removed.

## Pre-existing test files changed (with justification)

| File | Change | Justification |
|---|---|---|
| `crates/admin-cli/src/pam_stack.rs` | `tests::test_igf14_delegated_auth_reports_the_stack_of_the_shared_rule`: the four auth lines of spec §4.5.4 appended to the `inner` fixture; assertion untouched. New `ghf_tests` module appended (no other existing test touched) | **OA-1** (owner-approved): the old fixture's `[success=4]` ran past the end of the chain (libpam bad jump), now lands on `required pam_env.so` |
| `tests/docker/systemd_unit_acceptance_test.sh` | `part2_notify_readiness`: `ready_line < started_line` replaced by the SGR strip + anchored parse of `ready_sent_monotonic_us`, the presence check, `(( listening_line < ready_line ))` and, after `active_us` is read, `(( ready_sent_us <= active_us ))`; success message per spec §4.7.4 | **OA-2** (owner-approved, timestamp variant) |
| `tests/invariants/src/systemd_unit_acceptance_contract.rs` | `test_systemd_acceptance_harness_asserts_ready_ordering_and_clean_stop`: needle `listening_line < ready_line && ready_line < started_line` → `listening_line < ready_line`, `ready_sent_us <= active_us`, the parse needle and the SGR-strip needle; `main.rs` must also contain both exact message shapes; doc comment updated | **OA-2** ("invariant needle updated accordingly"; the new needles are stricter) |
| `crates/admin-cli/tests/cli_deadline_json_tests.rs` | `capture_deadline_with` split into a bounded loop (`MAX_CLAMP_ATTEMPTS = 20`, tolerated mode only; `Required` = 1 attempt) and `capture_attempt`; the server sends `Option<u64>` and returns `None` only on `UnexpectedEof` (prefix or body) in tolerated mode; every other failure still panics; the test assertion is untouched | **OA-3** (owner-approved) |
| `tests/invariants/src/lib.rs` | One `mod gdm_hardening_flaky_tests_contract;` registration (no test changed) | Registration of the new contract module |
| `crates/admin-cli/src/gdm.rs` | New `enable_race_tests` module appended; `restore_race_tests` untouched | New tests only |

Matrix SUA4 wording (OA-2) and the PFU7 annotation (OA-3) are left to the traceability agent (GHF9 checks SUA4).

## Flakiness check

- `cli_deadline_json_tests` (incl. the OA-3 clamp test): 10/10 green in a row.
- `gdm_hardening_tests` `ghf2` (FIFO non-blocking bound): 10/10 green.
- `test_ghf6_readiness_timeout_line_quotes_the_admin_path`: 10/10 deterministic (red on the same assertion).

## Owner approval OA-4 (2026-10-05)

After Phase 4 found that `test_ghf7_daemon_logs_the_ready_send_time_in_the_message_text` forbade the `sd_notify::notify_ready()` call that the pre-existing `systemd_readiness_tests::test_daemon_reports_ready_only_after_socket_bind` and `presence_unlock_contract::test_pau_main_spawns_presence_after_ready_and_stops_it_at_shutdown` require): approved. `sd_notify::notify_ready()` itself becomes the stamped variant (returns `(NotifyOutcome, Option<u64>)`); `notify_to_stamped` stays the test seam and `notify_ready_stamped()` stays as an alias used by `sd_notify_stamp_tests`; `main.rs` calls `sd_notify::notify_ready()` and logs the two exact message shapes. Only the new GHF7 invariant is relaxed: its clauses requiring `notify_ready_stamped()` and forbidding `sd_notify::notify_ready()` in `main.rs` are removed; every other GHF7 assertion stays. The two pre-existing tests are untouched. §4.7.3 and C13 are superseded on the function name only.

Additional pre-existing-file change: `tests/invariants/src/gdm_hardening_flaky_tests_contract.rs::test_ghf7_daemon_logs_the_ready_send_time_in_the_message_text` loses its `notify_ready_stamped()` requirement and its `sd_notify::notify_ready()` prohibition (OA-4); the message-shape and `current_monotonic_nanos` assertions are unchanged.
