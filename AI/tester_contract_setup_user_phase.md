# Tester Contract — GitHub #400: `scripts/setup.sh` user phase (PR 2 of #362)

- **Date**: 2026-10-15
- **Branch**: `feat/setup-user-phase`
- **Spec**: `AI/architect_spec_setup_user_phase.md` (§1–§9, Revision 1 §10, Revision 2 §11); plan evaluation APPROVED in round 3 (`AI/plan_evaluator_report.md`).
- **Files**: `tests/invariants/src/setup_user_phase_contract.rs` (new, registered in `tests/invariants/src/lib.rs`), `crates/remote/tests/setup_render_tests.rs` (new), fixture `tests/fixtures/setup/remote.toml` (new). `tests/invariants/src/setup_contract.rs` is not edited by this contract (its SET3 migration is the owner-approved commit 40b0fb1).

| Test (path::name) | Matrix | Red evidence (before the implementation) |
|---|---|---|
| `setup_user_phase_contract::test_set_u1_static_rules_of_the_user_phase` | SET20 | `setup.sh must declare readonly EXIT_COMPANION=5` |
| `setup_user_phase_contract::test_set_u3_help_documents_the_companion_options` | SET21 | `--help must document --no-remote` |
| `setup_user_phase_contract::test_set_u4_inconsistent_options_exit_2_without_change` | SET22 | `--yes --funnel on` exited 0 instead of 2 (unknown option before; then the identity check missing) |
| `setup_user_phase_contract::test_set_u5_dry_run_prints_the_companion_plan_without_change` | SET23 | `Unknown option: --tailscale-login` (exit 2, expected 0) |
| `setup_user_phase_contract::test_set_u6_full_run_sets_up_the_default_on_companion` | SET24 | exit 2 (unknown option) instead of 0 |
| `setup_user_phase_contract::test_set_u7_second_run_is_idempotent` | SET25 | first run exit 2 |
| `setup_user_phase_contract::test_set_u8_explicit_false_is_kept_unless_its_flag_says_on` | SET26 | first run exit 2 |
| `setup_user_phase_contract::test_set_u9_prerequisites_cascade_and_offline_needs_push` | SET27 | exit 2 |
| `setup_user_phase_contract::test_set_u10_without_identity_the_companion_is_skipped` | SET28 | the re-run command was not printed |
| `setup_user_phase_contract::test_set_u11_root_start_skips_the_companion` | SET29 | `[PLAN] companion: skipped` missing |
| `setup_user_phase_contract::test_set_u12_template_replaced_user_file_untouched` | SET30 | exit 2 |
| `setup_user_phase_contract::test_set_u13_unsafe_paths_refused_before_change` | SET31 | message `symbolic link` missing |
| `setup_user_phase_contract::test_set_u14_existing_daemon_toml_is_never_edited` | SET32 | exit 2 |
| `setup_user_phase_contract::test_set_u15_companion_failures_roll_back_and_exit_5` | SET33 | exit 2 instead of 5 |
| `setup_user_phase_contract::test_set_u22_rollback_stops_a_unit_started_by_the_run` | SET33 | added after the candid review round 2; red against the draft that only journaled enabled units |
| `setup_user_phase_contract::test_set_u16_daemon_side_failure_removes_daemon_toml` | SET34 | exit 2 instead of 1 |
| `setup_user_phase_contract::test_set_u17_install_remote_uses_cargo_target_dir` | SET35 | `install: cannot stat '<repo>/target/release/soos-remote'` (hard-coded path) |
| `setup_user_phase_contract::test_set_u2_install_remote_honours_cargo_target_dir_statically` | SET35 | `install_remote.sh must resolve CARGO_TARGET_DIR` |
| `setup_user_phase_contract::test_set_u18_docs_adr_matrix_and_walkthrough` | SET36 | `Docs/PACKAGING_AND_PROVISIONING.md must mention --tailscale-login` |
| `setup_user_phase_contract::test_set_u19_disabled_unit_stays_disabled` | SET37 | added with Revision 1 (R-U2); red against the draft that re-enabled the unit |
| `setup_user_phase_contract::test_set_u21_push_switched_on_later_enables_the_sender` | SET37 | added with Revision 2 (R2-F1); red against the draft that kept the sender off |
| `setup_user_phase_contract::test_set_u20_writable_config_dir_refused_and_render_matches_fixture` | SET38 | added with Revision 1 (R-U13, R-U14) |
| `soos-remote::setup_render_tests::test_set_render_with_every_default_on_is_accepted` | SET38 | evidence test (the parser exists; it pins the fixture) |
| `soos-remote::setup_render_tests::test_set_render_cascade_variants_are_accepted` | SET38 | evidence test |
| `soos-remote::setup_render_tests::test_set_render_never_written_variants_are_refused` | SET38 | evidence test |

The first Red run of the round-1 draft: `cargo test --locked -p soos-invariants --all-features setup_user_phase` → `0 passed; 18 failed`. The draft was realigned with Revisions 1 and 2 (F-m12) before the implementation was finished; the harness is this file's own (fake root, fake HOME, `XDG_RUNTIME_DIR` owned by the test uid, stub `systemctl` with per-unit `--user` state, stub `install_remote.sh` with the real contract, fake `soos-remote` enforcing the prerequisites with exit 78).

### Migrated existing tests

| Test | Old assertion | New assertion | Mandating decision |
|---|---|---|---|
| `setup_contract::test_set_s3_no_pam_tailscale_remote_or_network_lock_action` | bans `install_remote.sh`, `remote.toml`, `[preview]`; any `tailscale ` other than `tailscale version` | those three words allowed; `tailscale ` allowed only as `tailscale version` or inside a quoted string | owner approval of 2026-10-15 on #400 (commit 40b0fb1, applied by the lead, not by this contract) |

No other existing test or assertion is changed.

### Flakiness check

The behavioral tests use no timing assertion; the only sleep is the production settle of `COMPANION_SETTLE_S` (2 s) after a unit start. The whole file was run three times in a row with `--test-threads=3`: all green.
