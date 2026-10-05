# Tester Contract — GitHub #331 (install follow-ups, physical procedure fixes, GDM double face verification)

- Spec: `AI/architect_spec_install_gdm_followups.md` (incl. Revision 1); plan evaluation: `AI/plan_evaluator_report.md`
  (APPROVED round 2; R2-1 applied: the screensaver §6 needle does not require "there is no backup").
- Branch: `fix/install-gdm-followups`. Matrix prefix: **IGF**.
- Red kinds: **A** = runs and fails on an assertion; **C** = compile error on the specified new API only
  (the error names are listed); **G** = guard, passes today and must stay green (it pins behaviour the change
  must not regress).

## Contract table

| IGF | Test (path::name) | Kind | Red evidence |
|---|---|---|---|
| IGF1 | `tests/invariants/src/install_gdm_followups_contract.rs::test_igf_build_refuses_read_only_file_in_target_dir` | A | no "holds a file … cannot write" line; the build runs and exit 2 only comes from missing artifacts |
| IGF1 | `tests/invariants/src/install_gdm_followups_contract.rs::test_igf_dry_run_build_reports_read_only_file_in_target_dir` | A | exits 0 instead of 2 |
| IGF2 | `tests/invariants/src/install_gdm_followups_contract.rs::test_igf_build_refuses_unreadable_dir_mode_0000_with_chmod_advice`, `…_mode_0300_…` (non-root runner; skipped as root), `tests/invariants/src/install_gdm_followups_contract.rs::test_igf_fake_root_build_refuses_unreadable_dir_with_chmod_advice` | A | prints "cannot be fully inspected … sudo chown -R" instead of "cannot read" + `chmod -R u+rwX` |
| IGF3 | `tests/invariants/src/install_gdm_followups_contract.rs::test_igf_readiness_documents_real_worst_case_bound` | A | header, `--help`, README, packaging doc lack "+ 7.5 s" / "37.5 s" |
| IGF3 | `tests/invariants/src/install_gdm_followups_contract.rs::test_igf_readiness_hanging_admin_respects_documented_bound` | G | `--timeout 1` with a hanging admin ends < 10.5 s (passes today) |
| IGF4 | `tests/invariants/src/install_gdm_followups_contract.rs::test_igf_readiness_timeout_reports_last_status_stderr`, `…::test_igf_readiness_stderr_tail_replaces_control_characters`, `…::test_igf_readiness_stderr_tail_is_bounded` | A | no "Last '…' error:" block |
| IGF5 | `tests/invariants/src/install_gdm_followups_contract.rs::test_igf_readiness_fails_fast_on_non_executable_admin`, `…::test_igf_readiness_fails_fast_on_missing_admin` | A | takes ≈ 30.4 s (full timeout) instead of < 3 s, no `exit 126`/`exit 127` message |
| IGF6 | `crates/daemon/tests/presence_stream_settle_tests.rs::test_igf_settle_window_*` (15 tests: `constant_is_1000_ms`, `zero_settle_is_no_bound`, `not_woken_without_stamp_is_no_bound`, `woken_without_stamp_keeps_the_329_bound`, `stream_age_equal_to_settle_is_no_bound`, `stream_age_999_ms_waits_1_ms`, `stream_age_zero_waits_the_whole_settle`, `stream_age_300_ms_waits_700_ms`, `rounds_the_wait_up`, `future_stream_stamp_is_clamped_to_start`, `zero_stamp_is_none`, `woken_with_old_stream_keeps_start_bound`, `woken_with_young_stream_takes_the_maximum`, `saturates_without_panic`, `properties_over_a_grid`) | C | E0432 unresolved `soos_daemon::presence::{presence_settle_window, PresenceSettle}` |
| IGF7 | `crates/daemon/tests/presence_stream_settle_tests.rs::test_igf_scan_settles_a_stream_started_shortly_before` (stream stamped 300 ms before the scan: first PAD ≥ 690 ms and < 1000 ms, k = 3, one unlock, 39 attempts left) | C | E0432 as above + E0407 `stream_started_mono_ns` is not a member of `CameraManager` (in `tests/common/mod.rs`) |
| IGF7 | `crates/daemon/tests/presence_stream_settle_tests.rs::test_igf_scan_of_a_long_running_stream_has_no_settle` (stream 5 s old: first PAD < 500 ms, unlock) | C | same |
| IGF8 | `crates/camera-v4l/src/v4l_impl/supervisor_tests.rs::test_igf_stream_start_stamp_precedes_the_first_frame` | C | E0599 no method `stream_started_mono_ns` on `V4lCameraManager` |
| IGF8 | `crates/camera-v4l/src/v4l_impl/supervisor_tests.rs::test_igf_stream_start_stamp_is_withdrawn_on_suspend_and_renewed_on_resume` | C | same |
| IGF8 | `crates/camera-v4l/src/v4l_impl/supervisor_tests.rs::test_igf_stream_start_stamp_is_none_before_any_stream` | C | same |
| IGF8 | `crates/camera-v4l/tests/stream_start_stamp_tests.rs::test_igf_default_stream_started_mono_ns_is_none` | C | E0599 no method `stream_started_mono_ns` on `MinimalCamera` |
| IGF8 | `crates/camera-v4l/tests/stream_start_stamp_tests.rs::test_igf_mock_stream_started_defaults_to_none` | C | E0599 on `MockCameraManager` |
| IGF8 | `crates/camera-v4l/tests/stream_start_stamp_tests.rs::test_igf_mock_stream_started_setter_round_trips_regardless_of_readiness` | C | E0599 no method `set_stream_started_mono_ns` on `MockCameraManager` |
| IGF9 | `tests/invariants/src/install_gdm_followups_contract.rs::test_igf_stream_start_settle_is_presence_only` | A | `worker.rs` does not call `stream_started_mono_ns()` / `presence_settle_window(` yet (dispatcher/consensus/warmup/`crates/pam` parts pass) |
| IGF10 | `tests/invariants/src/install_gdm_followups_contract.rs::test_igf_relative_cargo_target_dir_artifact_default_uses_checkout` | A | "Artifact directory not found" (relative dir resolved against the cwd) |
| IGF10 | `tests/invariants/src/install_gdm_followups_contract.rs::test_igf_relative_cargo_target_dir_named_resolved_in_messages` | A | error prints the unresolved `target/igf_rel_…` |
| IGF10 | `tests/invariants/src/install_gdm_followups_contract.rs::test_igf_relative_cargo_target_dir_ownership_check_uses_checkout` | A | no ownership refusal on `<checkout>/<rel>` |
| IGF10 | `tests/invariants/src/install_gdm_followups_contract.rs::test_igf_relative_cargo_target_dir_passed_to_the_build` | A | `CARGO_TARGET_DIR="${BUILD_TARGET_DIR}"` literal missing (non-root and runuser shim checks follow) |
| IGF11 | `crates/admin-cli/src/status.rs::systemctl_bound_tests::test_igf11_systemctl_show_timeout_is_one_second` | C | E0425 cannot find value `SYSTEMCTL_SHOW_TIMEOUT_MS` |
| IGF11 | `…::systemctl_bound_tests::test_igf11_hanging_systemctl_is_killed_and_reported_unknown` (`exec sleep 30`, 300 ms timeout, unknown triple within timeout + 1 s) | C | E0425 cannot find function `inspect_systemd_unit_with` |
| IGF11 | `…::systemctl_bound_tests::test_igf11_descendant_holding_stdout_does_not_block` (`sleep 30 &` keeps stdout) | C | same |
| IGF11 | `…::systemctl_bound_tests::test_igf11_normal_output_is_parsed` / `test_igf11_arguments_are_unchanged` | C | same |
| IGF11 | `…::systemctl_bound_tests::test_igf11_non_zero_exit_is_unknown` / `test_igf11_oversized_output_is_unknown` (> 4096 bytes) / `test_igf11_missing_program_is_unknown` | C | same |
| IGF12 | `tests/invariants/src/install_gdm_followups_contract.rs::test_igf_physical_rollback_edits_only_base_stacks` | A | §6 has no `--follow-symlinks` loop |
| IGF13 | `tests/invariants/src/install_gdm_followups_contract.rs::test_igf_physical_gdm_worker_pgrep_hint` | A | §3.3 Test Case 2 lacks the `pgrep -af` hint |
| IGF14 | `crates/admin-cli/tests/gdm_shared_rule_tests.rs::test_igf14_arch_packaged_system_auth_enable_adds_no_block` (packaged `packaging/pam/arch/system-auth` via `include_str!`) | A | `enable must accept …: the shared auth stack 'system-auth' runs the unclassified auth rule 'pam_soos.so' (control '[success=4 default=ignore]') …` |
| IGF14 | `…::test_igf14_debian_common_auth_done_enable_adds_no_block` / `test_igf14_debian_common_auth_rewritten_jump_enable_adds_no_block` | A | same refusal on `'common-auth'` (`[success=done …]` / `[success=2 …]`) |
| IGF14 | `…::test_igf14_fedora_soos_password_auth_enable_adds_no_block` (packaged authselect template rendered with `with-faillock`) | A | same refusal on `'password-auth'` |
| IGF14 | `…::test_igf14_hand_written_sufficient_rule_enable_adds_no_block`, `…::test_igf14_rules_after_the_shared_rule_are_not_inspected`, `…::test_igf14_second_enable_on_shared_stack_still_writes_nothing` | A | same refusal on `'shared-auth'` / `'system-auth'` |
| IGF14 | `crates/admin-cli/src/pam_stack.rs::tests::test_igf14_delegated_auth_reports_the_stack_of_the_shared_rule`, `…::test_igf14_delegated_auth_without_soos_rule_returns_gates` | C | E0425 `delegated_auth`, E0433 `DelegatedAuth` |
| IGF15 | `crates/admin-cli/tests/gdm_shared_rule_tests.rs::test_igf15_existing_block_is_removed_and_backup_left_identical` (content, mode, inode of the backup; no temp file) | A | `enable must remove the redundant managed block, not refuse: … unclassified auth rule 'pam_soos.so' …` |
| IGF15 | `…::test_igf15_restore_after_block_removal_succeeds_and_removes_backup`, `…::test_igf15_second_enable_after_block_removal_writes_nothing` (inode + mtime), `…::test_igf15_stale_backup_still_needs_force_after_block_removal` | A | `enable removes the redundant block: … unclassified …` |
| IGF15 | `…::test_igf15_legacy_and_bare_lines_removed_without_creating_a_backup` (`LEGACY_GDM_PAM_LINE` and bare `GDM_PAM_LINE`) | A | `enable must remove the redundant line "auth  sufficient  pam_soos.so timeout_ms=2500", not refuse: …` |
| IGF16 | `crates/admin-cli/tests/gdm_shared_status_tests.rs::test_igf16_arch_shared_rule_reports_installed_with_stack_name`, `test_igf16_debian_and_fedora_shared_rules_report_their_stack`, `test_igf16_direct_rule_keeps_shared_stack_none`, `test_igf16_stock_stack_without_soos_reports_not_installed`, `test_igf16_unreadable_or_refused_stacks_fail_closed`, `test_igf16_enable_returns_status_with_shared_stack`, `test_igf16_status_json_and_table_show_the_shared_stack` (binary: `  Shared soos Rule:  system-auth`, JSON `shared_stack`) | C | E0609 no field `shared_stack` on type `GdmStatus` (14×) |
| IGF17 | `crates/admin-cli/tests/gdm_shared_rule_tests.rs::test_igf17_qualifying_edge_forms_count_as_shared` (`SUFFICIENT`, `AUTH`, `[Success=DONE Default=Ignore]`, extra `ignore=ignore`, `success=1` + `timeout_ms`, path form, `-auth`) | A | `rule "auth  SUFFICIENT  pam_soos.so" must count as shared: … unclassified …` |
| IGF17 | `…::test_igf17_non_primary_soos_rules_keep_the_refusal` (all 14 §2.2 non-qualifying forms: unclassified refusal, file unchanged, no backup, flag kept) | G | passes today; must keep passing |
| IGF17 | `…::test_igf17_soos_rule_after_credential_module_still_inserts_block` | G | passes today |
| IGF17 | `crates/admin-cli/src/pam_stack.rs::tests::test_igf17_packaged_soos_rules_are_primary`, `test_igf17_qualifying_edge_forms_are_primary`, `test_igf17_non_primary_soos_rules_are_rejected` | C | E0599 no method `is_primary_soos_rule` on `PamLine` |
| IGF18 | `crates/admin-cli/tests/gdm_shared_rule_tests.rs::test_igf18_jump_crossing_with_shared_rule_is_accepted_without_edit` | A | `enable must accept …: a [...=N] jump before the insertion point would change target …` |
| IGF18 | `…::test_igf18_jump_crossing_without_shared_rule_keeps_jump_error`, `…::test_igf18_unclassified_rule_before_shared_rule_is_refused`, `…::test_igf18_unclassified_rule_in_gdm_file_before_delegation_is_refused`, `…::test_igf18_administrator_rule_in_gdm_file_is_untouched_with_shared_rule`, `…::test_igf18_missing_include_still_refused` | G | pass today (error priority / untouched administrator rule) |
| IGF18 | `crates/admin-cli/src/pam_stack.rs::tests::test_igf18_delegated_auth_credential_before_soos_rule_is_not_shared`, `test_igf18_delegated_auth_keeps_refusals`, `test_igf18_delegated_auth_keeps_the_depth_bound` | C | E0425 `delegated_auth`, E0433 `DelegatedAuth` |
| IGF19 | `tests/invariants/src/install_gdm_followups_contract.rs::test_igf_docs_and_adr_describe_the_changes` | A | first failing needle: `[2026-10-05] GDM Reuses a Shared Primary soos Rule` absent from `AI/DECISIONS.md`; also checks the amendment note, `AI/ARCHITECTURE.md` (`only when`, `shared_stack`, no bare sentence), deployment §2.1 (`Shared soos Rule`, QFU5 needles, `1000 ms`, Fedora label), screensaver §6 (`Shared soos Rule`, `may be no backup` per R2-1), project-facts rows, `Docs/CAMERA_V4L_CRATE.md` |

## Collateral compile breakage until the API exists (expected)

- `crates/daemon/tests/common/mod.rs` overrides `CameraManager::stream_started_mono_ns` on `SpyCamera`, so every daemon
  integration test that includes `mod common;` fails to compile with E0407 only.
- `crates/camera-v4l` lib unit tests (supervisor tests live in the lib `cfg(test)` build) fail with E0599 only.
- `crates/admin-cli` lib unit tests (`pam_stack.rs`, `status.rs` test modules) fail with E0425/E0433/E0599 on
  `delegated_auth`, `DelegatedAuth`, `is_primary_soos_rule`, `inspect_systemd_unit_with`, `SYSTEMCTL_SHOW_TIMEOUT_MS` only.

## Deviations from the spec

- IGF9 does not diff `consensus.rs` / `config.rs` against `b05477d` (would break after merge or in shallow CI); it checks
  instead that dispatcher and consensus never name the settle items, `DAEMON_DEFAULT_WARMUP_FRAMES` stays 0 and only
  `crates/daemon/src/presence/` names `stream_started_mono_ns`. The byte-identity check stays with the auditor.
- IGF16 lives in its own file (`gdm_shared_status_tests.rs`) so that its compile error on `GdmStatus::shared_stack` does
  not stop IGF14/15/17/18 from running red on assertions.
- IGF7 long-running-stream case asserts first PAD < 500 ms (stricter than IWP10's 1000 ms, to catch a partial wait).
- IGF19 also requires `stream_started_mono_ns` in `Docs/CAMERA_V4L_CRATE.md` and `1000 ms` in the deployment doc (§9.2);
  per R2-1 it requires "may be no backup", never "there is no backup".
- IGF10 tests create `<checkout>/target/igf_rel_*` (gitignored) and remove it on drop.

## Migrated existing tests

None. The only edit to existing test code is the owner-accepted setup-only `SpyCamera` field
`stream_started_ns: AtomicU64` (0 = `None`), its initialiser at the single construction site and the trait override
(`crates/daemon/tests/common/mod.rs`); plus `clippy::arithmetic_side_effects` added to the `allow` list of the
existing `pam_stack.rs` tests module (lint attribute only). No assertion was modified, weakened or deleted.

## Flakiness check

To run once green (timing-sensitive): `presence_stream_settle_tests::test_igf_scan_*`,
`status.rs::systemctl_bound_tests::test_igf11_hanging_*` / `…descendant_*`, and the shell-driven IGF3/IGF5 elapsed-time
checks, 10× each (`for i in $(seq 10); do cargo test --locked -p <pkg> --all-features <name> -q || break; done`).
