# Tester Contract — GitHub #379: first-run onboarding tour with per-feature toggles in `soos-gui`

- **Date**: 2026-10-13
- **Spec**: `AI/architect_spec_onboarding_tour.md` (with §10 Revision 1; plan evaluator round 2 APPROVED)
- **Branch**: `feat/onboarding-tour` (GitHub-only issue, commit carries `Closes #379`, `Refs #363`)
- **Red run**: `cargo test --locked -p soos-gui --all-features --test onboarding_state_tests --no-run` (and the three other files) before `crates/gui/src/onboarding.rs`, `feature_control.rs` and `onboarding_view.rs` existed: `error[E0432]: unresolved import soos_gui::onboarding` (the missing items are exactly the specified API; no other compile error).
- **Revision**: the routing and state tables were revised in Phase 2 after the plan evaluator's round 1 (findings 1–3, 11) before any implementation was judged against them; nothing was weakened after the developer phase started.

These tests are immutable contracts for the developer phase.

## 1. Contract table

| Test file | Tests | Matrix | Covers |
|---|---|---|---|
| `crates/gui/tests/onboarding_state_tests.rs` | `test_obt_page_count_*`, `test_obt_closed_tour_*`, `test_obt_first_run_*`, `test_obt_back_saturates_at_intro`, `test_obt_close_restart_and_overview`, `test_obt_generation_*` | OBT1 | page machine, skip/close, replay, overview, motion hook |
| same | `test_obt_state_path_*`, `test_obt_parse_tour_state_accepts_only_seen_records`, `test_obt_should_show_tour_only_when_not_seen_or_older` | OBT2 | per-user path, record parsing, first-launch-only rule |
| same | `test_obt_store_*` | OBT3 | 0600 file / 0700 dir, atomic write without leftovers, bound, malformed → shown again, symlink refused on read and write, directory → unavailable |
| `crates/gui/tests/onboarding_feature_tests.rs` | `test_obt_catalogue_*`, `test_obt_wording_*`, `test_obt_presence_explanation_*` | OBT4 | nine features of #363, unique titles, plain non-childish English |
| same | `test_obt_unknown_snapshot_*`, `test_obt_all_on_snapshot_shows_on`, `test_obt_global_flag_*`, `test_obt_presence_flag_*`, `test_obt_presence_unknown_*`, `test_obt_companion_state_*`, `test_obt_remote_key_states_*`, `test_obt_remote_keys_not_installed_or_unknown`, `test_obt_wifi_lock_state_*`, `test_obt_daemon_presence_key_off_*`, `test_obt_camera_view_needs_the_daemon_remote_view` | OBT5 | state shown only from probed facts (flags, daemon.toml, unit, remote.toml, rules) |
| same | `test_obt_root_flags_*`, `test_obt_wifi_lock_routes_*`, `test_obt_wifi_lock_on_never_drops_*`, `test_obt_presence_on_is_unavailable_*`, `test_obt_presence_off_on_both_sides_*`, `test_obt_companion_*`, `test_obt_masked_companion_routes`, `test_obt_remote_keys_turn_off_*`, `test_obt_remote_keys_as_root_*`, `test_obt_turn_on_commands_name_their_prerequisites`, `test_obt_offline_alerts_have_no_control_yet`, `test_obt_shown_commands_carry_no_user_data` | OBT6 | toggle routing (privileged action / user manager / command / unavailable) |
| `crates/gui/tests/feature_control_tests.rs` | `test_obt_programs_are_absolute`, `test_obt_flag_paths_are_the_closed_daemon_set`, `test_obt_system_flag_commands_*`, `test_obt_network_lock_command_*`, `test_obt_network_lock_rule_path_matches_the_admin_cli`, `test_obt_companion_commands_*`, `test_obt_feature_control_runs_no_shell_*` | OBT7 | fixed argument vectors, absolute programs, closed path set, constants cross-checked with daemon / admin CLI sources |
| same | `test_obt_parse_unit_show`, `test_obt_parse_daemon_keys_*`, `test_obt_remote_config_path_*`, `test_obt_max_remote_config_bytes_*`, `test_obt_parse_remote_keys` | OBT8 | parsers and bounds |
| same | `test_obt_disable_*`, `test_obt_keys_to_clear_*` | OBT9 | off-only `remote.toml` edit: one line, comments kept, cascade, refusals, verification |
| same | `test_obt_read_remote_config_*`, `test_obt_write_user_file_atomically_*`, `test_obt_user_files_never_block_on_a_fifo` | OBT10 | bounded non-blocking reads, symlink refusal, atomic 0600 writes |
| same | `test_obt_probe_*` | OBT11 | probes never write and never run `pkexec`; root skips the user manager |
| same | `test_obt_executor_*`, `test_obt_action_privilege_classification` | OBT12 | executor calls, denied authorization, root refusals, edit then restart, no-op writes nothing |
| same | `test_obt_runner_*` | OBT13 | one action at a time, non-blocking poll, privileged-in-flight flag, recovery from a dead worker |
| `crates/gui/tests/onboarding_view_tests.rs` | `test_obt_every_page_renders_headless_without_events`, `test_obt_closed_tour_renders_nothing`, `test_obt_escape_closes_only_when_idle` | OBT14 | headless rendering of every page and state; Escape closes only when idle |
| same | `test_obt_view_code_performs_no_io`, `test_obt_app_ui_code_still_spawns_no_process` | OBT15 | no I/O in the view, no process spawn in `app.rs` |

## 1a. Additive contract from the audit (Phase 3)

| Test file | Tests | Matrix | Covers |
|---|---|---|---|
| `crates/gui/tests/onboarding_audit_tests.rs` | `test_obt_state_file_is_written_only_in_the_users_own_tree`, `test_obt_mark_seen_checks_ownership_before_creating_anything` | OBT19 | auditor constraint 2 |
| same | `test_obt_dead_worker_reports_a_failed_change` | OBT20 | auditor constraint 3 |
| same | `test_obt_every_task_submission_goes_through_submit_task` | OBT21 | auditor constraint 6 |
| same | `test_obt_program_runner_kills_and_reaps_after_the_deadline` | OBT22 | auditor advisory A4 |

Added after the audit; no existing assertion changed.

## 2. Migrated existing tests

None. `PrivilegedAction` / `PrivilegedOutcome` are unchanged (the exhaustive matches in `responsiveness_tests.rs` and `camera_source_tests.rs` stay untouched); `program_path_tests.rs` and `gcv_review_contract.rs` pass unchanged.

## 3. Flakiness check

`test_obt_runner_*` poll with the shared `common::wait_until` helper (5 s bound); run 10 times in a row with `for i in $(seq 10); do cargo test --locked -p soos-gui --all-features --test feature_control_tests test_obt_runner -q || break; done`.
