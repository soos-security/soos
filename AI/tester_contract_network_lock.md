# Tester Contract — GitHub #365: opt-in polkit Wi-Fi lock and `soos-admin network-lock`

- **Date**: 2026-10-13
- **Spec**: `AI/architect_spec_network_lock.md`
- **Branch**: `feat/network-lock` (GitHub-only issue, commit carries `Closes #365`)
- **Red run**: `cargo test --offline -p soos-admin-cli --all-features --no-fail-fast --test network_lock_status_tests --test network_lock_install_tests --test network_lock_shell_tests` and `cargo test --offline -p soos-invariants --all-features network_lock_contract`, against a compiling stub of the specified API (`crates/admin-cli/src/network_lock.rs` returning `Missing`/`Unknown`/`Err`, `main.rs` ignoring `network-lock`).

These tests are immutable contracts for the developer phase.

## 1. Contract table

| Test (path::name) | Matrix | Red evidence |
|---|---|---|
| `soos-invariants::network_lock_contract::test_nlk_rules_file_lists_exactly_the_documented_action_ids` | NLK1 | FAILED: `read Docs/NETWORK_LOCK.md: No such file or directory` |
| `soos-invariants::network_lock_contract::test_nlk_rules_file_only_returns_auth_admin` | NLK1 | passes on the Phase 1 rule file; mutation `AUTH_ADMIN_KEEP`, `else { return polkit.Result.NO; }` and a `subject.local` guard each make it FAIL |
| `soos-invariants::network_lock_contract::test_nlk_rules_file_falls_through_for_other_actions` | NLK1 | passes on the Phase 1 rule file; mutation `else { return polkit.Result.NO; }` makes it FAIL |
| `soos-invariants::network_lock_contract::test_nlk_rfkill_rule_only_removes_the_uaccess_tag` | NLK9 | passes on the Phase 1 udev file; mutation `TAG+=` makes it FAIL |
| `soos-invariants::network_lock_contract::test_nlk_rule_contents_single_sourced_from_packaging` | NLK2 | FAILED: stub has no `Sha256` / `O_NOFOLLOW` |
| `soos-invariants::network_lock_contract::test_nlk_lock_is_opt_in_and_removed_on_uninstall` | NLK2, NLK3 | FAILED: `scripts/uninstall.sh must remove polkit-1/rules.d/49-soos-network-lock.rules` |
| `soos-invariants::network_lock_contract::test_nlk_docker_pkcheck_job_wired_into_ci` | NLK10 | FAILED: `read tests/docker/network_lock_pkcheck_test.sh` (before the script was added), then `CI job network-lock-polkit` |
| `soos-invariants::network_lock_contract::test_nlk_docs_adr_and_walkthrough` | NLK11 | FAILED: `read Docs/NETWORK_LOCK.md` |
| `crates/admin-cli/tests/network_lock_status_tests.rs::test_nlk_status_installed_when_byte_identical_and_secure` | NLK4 | FAILED: `left: Missing right: Installed` |
| `...::test_nlk_status_installed_in_a_0750_polkitd_style_directory` | NLK4 | FAILED |
| `...::test_nlk_status_missing_when_absent` | NLK4 | FAILED: empty table lacks `missing` |
| `...::test_nlk_status_missing_when_polkit_rules_dir_absent` | NLK4 | passes on the stub (stub answers `Missing`); paired with the installed tests |
| `...::test_nlk_status_modified_when_one_byte_differs`, `..._trailing_byte_added_or_truncated`, `..._when_empty`, `..._over_the_read_cap` | NLK4 | FAILED: `left: Missing right: Modified` |
| `...::test_nlk_status_insecure_when_group_or_world_writable`, `..._owner_is_not_the_required_owner`, `..._rule_file_is_a_symlink`, `..._rules_dir_is_a_symlink`, `..._rules_dir_is_group_or_world_writable`, `..._insecure_takes_precedence_over_unsupported_backend` | NLK5 | FAILED: `left: Missing right: InsecurePermissions` |
| `...::test_nlk_status_unreadable_when_rules_dir_cannot_be_searched`, `..._rule_file_cannot_be_read` | NLK5 | FAILED: `left: Missing right: Unreadable` (return early as root) |
| `...::test_nlk_status_unsupported_backend_on_iwd_only_host`, `..._networkd_only_host`, `..._modified_takes_precedence_over_unsupported_backend`, `test_nlk_backend_precedence`, `..._unknown_backend_keeps_the_file_verdict`, `..._uses_the_probe_result_for_the_backend` | NLK6 | FAILED |
| `...::test_nlk_proc_probe_matches_exact_comm_names` | NLK6 | FAILED |
| `...::test_nlk_proc_probe_ignores_non_numeric_entries_and_near_names`, `..._ignores_oversize_comm_and_missing_proc` | NLK6 | pass on the stub (all-false answer); paired with `..._matches_exact_comm_names` |
| `...::test_nlk_rfkill_current_uaccess_tag_warns`, `..._legacy_database_without_current_tags_uses_g_lines`, `..._removed_current_tag_is_restricted`, `..._tag_match_is_exact`, `..._world_writable_node_warns`, `..._absent_without_sysfs_device`, `..._rule_file_state_is_reported`, `..._rule_modified_or_insecure_fails_the_status` | NLK7 | FAILED |
| `...::test_nlk_rfkill_unknown_on_malformed_dev_or_missing_database` | NLK7 | passes on the stub (`Unknown`); paired with the uaccess tests |
| `...::test_nlk_status_json_shape`, `..._output_has_no_network_names`, `..._table_names_the_installed_rule_path` | NLK8 | FAILED: `valid JSON: EOF while parsing` / `assertion failed: !out.is_empty()` |
| `crates/admin-cli/tests/network_lock_install_tests.rs::test_nlk_install_writes_identical_0644_rule_without_temp_file`, `..._replaces_a_modified_rule`, `..._rfkill_writes_the_udev_rule`, `..._without_rfkill_removes_only_the_soos_rfkill_rule`, `..._without_rfkill_needs_no_udev_dir` | NLK2 | FAILED: `install: NetworkLock("not implemented")` |
| `...::test_nlk_install_refuses_a_symlink_target`, `..._dangling_symlink_target`, `..._symlinked_rules_dir`, `..._symlinked_ancestor_dir`, `..._missing_rules_dir`, `..._directory_at_the_target_name`, `..._rfkill_with_missing_udev_dir_writes_nothing` | NLK2 | pass on the stub (it refuses everything); their power comes from the pairing with the positive install tests above, which require writes to succeed on a clean root |
| `...::test_nlk_uninstall_removes_both_files_and_is_idempotent`, `..._on_an_empty_root_is_ok`, `..._unlinks_a_symlink_without_touching_its_target` | NLK3 | FAILED |
| `...::test_nlk_uninstall_refuses_a_symlinked_rules_dir`, `..._directory_at_the_target_name` | NLK3 | pass on the stub (refusal); paired with the positive uninstall tests |
| `...::test_nlk_may_modify_root` | NLK2 | FAILED: `may_modify_root("/", 0)` |
| `...::test_nlk_cli_parses_network_lock_actions` | NLK2 | passes: the clap surface is part of the specified API added with the stub |
| `crates/admin-cli/tests/network_lock_shell_tests.rs::test_nlk_shell_install_status_uninstall_in_temp_root` (runs `crates/admin-cli/tests/network_lock_install_test.sh`: S1–S8 incl. `scripts/uninstall.sh --destdir`) | NLK2, NLK3 | FAILED: `[FAIL] S1 installed rule differs` (binary ignores the command) |
| `tests/docker/network_lock_pkcheck_test.sh` (CI job `network-lock-polkit`, P1–P5) | NLK10 | Ran locally on 2026-10-13 against the pinned image: all checks passed with the rule; with the rule mutated to `AUTH_ADMIN_KEEP`: `[FAIL] P2 network-control: expected an unretained challenge (2 plain), got '2 retained'` |

### 1.1 Added after plan evaluation round 1 (spec §11)

| Test | Finding | Red evidence |
|---|---|---|
| `network_lock_status_tests.rs::test_nlk_status_warns_iwd_dbus_bypass_next_to_network_manager` | F-3 | FAILED (stub reports no backend/warning) |
| `...::test_nlk_rfkill_reboot_pending_after_rule_written_since_boot`, `..._warnings_have_a_fixed_order` | F-4 | FAILED |
| `...::test_nlk_rfkill_reboot_pending_needs_an_installed_rule_and_a_valid_uptime` | F-4 | passes on the stub (no warnings); paired with the positive test |
| `network_lock_install_tests.rs::test_nlk_install_replaces_a_hard_linked_target_with_a_new_inode` | F-7 | FAILED: `install: NetworkLock("not implemented")` |
| `...::test_nlk_install_fails_cleanly_on_a_stale_temp_name`, `..._refuses_group_or_world_writable_directories`, `test_nlk_uninstall_refuses_group_or_world_writable_directories`, `test_nlk_install_as_root_requires_root_owned_directories` | F-6, F-7 | pass on the stub (refusal); paired with the positive install tests |
| `soos-invariants::network_lock_contract::*` updated before Green: five ids, `sleep-wake` must not be listed, uninstall removal before `# 2. Roll Back PAM`, `before removing the package` in the docs, `SOOS_DEBIAN_IMAGE:-` digest pin, sentinel needles, new doc needles | F-1, F-2, F-5, F-8, F-9 | FAILED (docs/script/uninstall not yet updated) |
| `tests/docker/network_lock_pkcheck_test.sh` S1/S2 sentinel differential | F-2 | all checks passed with the rule; with the guard mutated to `indexOf(action.id) > 0`: `[FAIL] S2 org.freedesktop.NetworkManager.network-control: expected '2 plain', got '0 plain'` (the real-default P-checks alone did not catch it, as the evaluator predicted) |

## 2. Docker pkcheck: feasibility and equivalence

Feasible in the existing pinned `ubuntu:24.04` image used by `tests/docker/pam_rollback_test.sh` (packages installed at run time). It cannot create an "active local" subject (no seat in a container), so the subject is classified by `allow_any`; the rule never reads `subject` (static test `test_nlk_rules_file_only_returns_auth_admin`), so the result is identical for an active local session. The active-local case is the owner hardware check NLK12.

## 3. Migrated existing tests

none (no existing test is modified; the new `Commands::NetworkLock` variant is matched by `_ =>` arms in every existing test).

## 4. Flakiness check

The status/install tests are filesystem-only with no timing. Run 10×: see the walkthrough §8.

## Follow-up — GitHub #370 (spec §12, plan evaluation APPROVED with F-1 … F-7 folded in)

Branch `fix/network-lock-status-hardening`. Red evidence: a stub that only adds the specified constants (`MAX_PROC_STATUS_BYTES`, `MAX_PASSWD_BYTES`, `SYSTEMD_NETWORK_USER`, `MAX_RULES_DIR_ENTRIES`) compiles and fails every new test on its assertion; the existing 42 status, 24 install and S1–S8 shell checks stay green on it.

| Test (path::name) | Item / matrix ID | Red evidence (failure message) |
|---|---|---|
| `network_lock_hardening_tests::test_nlk_proc_probe_ignores_user_owned_services` | G-1 / NLK13 | `left: RunningServices { network_manager: true, networkd: true, iwd: true }` |
| `network_lock_hardening_tests::test_nlk_proc_probe_requires_all_four_uids_trusted` | G-1 / NLK13 | `left: RunningServices { network_manager: true, .. }` (position 0) |
| `network_lock_hardening_tests::test_nlk_proc_probe_ignores_missing_or_malformed_status` | G-1 / NLK13 | `left: RunningServices { .., iwd: true }` (no status file) |
| `network_lock_hardening_tests::test_nlk_proc_probe_trusts_systemd_network_only_for_networkd` | G-1 / NLK13 | `left: RunningServices { .., networkd: true, .. }` (user uid) |
| `network_lock_hardening_tests::test_nlk_proc_probe_ignores_uid_line_beyond_the_read_cap` | G-1 (F-3) / NLK13 | `left: RunningServices { .., iwd: true }` |
| `network_lock_hardening_tests::test_nlk_proc_probe_requires_the_status_name_to_match_comm` | G-1 (F-5) / NLK13 | `left: RunningServices { network_manager: true, networkd: true, iwd: true }` |
| `network_lock_hardening_tests::test_nlk_proc_probe_ignores_systemd_network_entry_cut_by_the_read_cap` | G-1 (F-3) / NLK13 | `left: RunningServices { .., networkd: true, .. }` (uid 612) |
| `network_lock_hardening_tests::test_nlk_status_spoofed_network_manager_keeps_unsupported_backend` | G-1 / NLK13 | `left: NetworkManager` |
| `network_lock_hardening_tests::test_nlk_status_insecure_when_an_ancestor_dir_is_group_or_world_writable` | G-2 / NLK14 | `left: Installed` (`etc/polkit-1` 0777) |
| `network_lock_hardening_tests::test_nlk_status_insecure_when_an_ancestor_dir_is_a_symlink` | G-2 / NLK14 | `left: Installed` |
| `network_lock_hardening_tests::test_nlk_status_rfkill_rule_insecure_when_its_ancestor_is_writable` | G-2 / NLK14 | `left: Installed` (`etc/udev` 0777) |
| `network_lock_hardening_tests::test_nlk_install_removes_stale_temp_files_of_dead_processes` | G-5 / NLK16 | `left: [".49-soos-network-lock.rules.4194304.tmp", "49-soos-network-lock.rules"]` |
| `network_lock_hardening_tests::test_nlk_uninstall_removes_stale_temp_files_of_dead_processes` | G-5 / NLK16 | stale temp file listed after uninstall |
| `network_lock_hardening_tests::test_nlk_stale_temp_cleanup_keeps_live_foreign_and_non_regular_entries` | G-5 / NLK16 | `stale file of a dead pid removed` |
| `network_lock_shell_tests::test_nlk_uninstall_script_checks_ancestors_and_cleans_temp_files` (`network_lock_uninstall_script_test.sh` U1–U4) | G-4, G-5 / NLK15, NLK17 | `[FAIL] U1 the rule was removed through a symlinked ancestor` |

Test power (F-4): the dead pids are impossible values (`4194304` = `PID_MAX_LIMIT`, `4194305`, `2147483647`), never a reaped child; every uid position is varied separately; the fake `systemd-network` uid is 61234 (not the host's), and NetworkManager/iwd at that uid must not be credited; the keep test covers pid 1 (`EPERM` as non-root), a live `sleep` child (`Ok`), a temp-named symlink with an untouched outside target, a temp-named directory, a leading-zero pid, an 11-digit pid, a signed pid (`+N`), near names and another file's temp name; the ancestor tests loop over `etc`, `etc/polkit-1` and `etc`, `etc/udev`. The existing `test_nlk_install_fails_cleanly_on_a_stale_temp_name` (current pid) keeps its contract: the current process is alive.

### Migrated existing tests (contract amendments, setup only, assertions untouched)

| Test | Setup change | Reason |
|---|---|---|
| `network_lock_status_tests::test_nlk_proc_probe_matches_exact_comm_names` | adds `proc/<pid>/status` (`Name:` = comm, `Uid:` 0 0 0 0) for each fake process | GitHub #370 item 1: a match is credited only for a root-owned process |
| `network_lock_status_tests::test_nlk_status_uses_the_probe_result_for_the_backend` | same, for the iwd process | idem |
| `network_lock_status_tests::test_nlk_status_output_has_no_network_names` | same, for the NetworkManager process (the status content is not printed) | idem |
| `network_lock_status_tests::test_nlk_proc_probe_ignores_non_numeric_entries_and_near_names` | same, for every entry | keeps the test discriminating: without it the test would pass on the missing uid, not on the name rule |
| `network_lock_status_tests::test_nlk_proc_probe_ignores_oversize_comm_and_missing_proc` | same, for the oversize-comm process | idem, for the comm size rule |

Each change only adds lines (`git diff` of the file has no removed line); every assertion is unchanged.

### Flakiness check

The new tests have no timing dependency (impossible pids, a `sleep 60` child killed on drop). Run 10 times in a row after the implementation (see walkthrough 199).
