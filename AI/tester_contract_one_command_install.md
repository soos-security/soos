# Tester Contract — GitHub #372: `scripts/setup.sh` core (PR 1 of #362)

- **Date**: 2026-10-13 — **Branch**: `feat/one-command-install` — **Spec**: `AI/architect_spec_one_command_install.md` (§1–§10, Revision 1 §11, Revision 2 §12; plan evaluation round 2 APPROVED)
- **File**: `tests/invariants/src/setup_contract.rs` (registered in `tests/invariants/src/lib.rs`, `#[cfg(all(test, unix))]`)
- **Harness**: fake checkout (real `setup.sh`, `check_build_deps.sh`, `provision_master_key.sh`, unit, Arch PAM snippet, models manifest; stub `install.sh`, `install_rustup.sh`, `wait_daemon_ready.sh`, `download_models.sh`), fake root (`--prefix/--sysconfdir/--localstatedir/--runstatedir/--pam-dir`), restricted `PATH` (stubs, then symlinks to harmless executables only). Stubs log `uid=<0|1000> <argv>`; `sudo` emulates `env_reset` (`env -i` + allowlist); `soos-enroll` fails "Camera is busy" while the stub unit is active. Nothing runs as root, nothing touches the host.

| Test (`soos-invariants::setup_contract::`) | Row | Red evidence (before `scripts/setup.sh` existed) |
|---|---|---|
| `test_set_s1_script_is_strict_bash_without_eval` | SET1 | `scripts/setup.sh must exist` |
| `test_set_s2_package_lists_come_only_from_check_build_deps` | SET2 | read error |
| `test_set_s3_no_pam_tailscale_remote_or_network_lock_action` | SET3 | read error |
| `test_set_s4_bounded_constants` | SET4 | read error |
| `test_set_help_documents_every_parsed_option` | SET5 | exit 127 (no script) |
| `test_set_invalid_arguments_exit_2_without_change` | SET6 | exit 127 |
| `test_set_dry_run_changes_nothing_and_prints_plan` | SET7 | exit 127 |
| `test_set_detection_parses_os_release_without_sourcing`, `test_set_unsupported_host_refused_before_change` | SET8 | exit 127 |
| `test_set_preconditions_refused_before_change` | SET9 | exit 127 |
| `test_set_full_run_as_user_installs_face_profile`, `test_set_user_in_group_skips_add_user`, `test_set_debian_uses_apt_for_missing_packages`, `test_set_skip_packages_prints_command_only` | SET10 | exit 127 |
| `test_set_second_run_is_idempotent`, `test_set_damaged_install_and_reinstall` | SET11 | exit 127 |
| `test_set_full_run_as_root_builds_as_invoking_user` | SET12 | exit 127 |
| `test_set_remote_session_skips_enrollment_with_command`, `test_set_seat_falls_back_to_the_display_session_in_the_user_half`, `test_set_remote_own_session_never_borrows_the_desk_session`, `test_set_daemon_stop_marker_cleared_only_after_restart` | SET13 | exit 127; the last two were added after the candid review (§ below) |
| `test_set_missing_toolchain_uses_verified_rustup` | SET14 | exit 127 |
| `test_set_build_and_install_failures_stop_cleanly` | SET15 | exit 127 |
| `test_set_runtime_failure_rolls_back_setup_changes`, `test_set_late_failures_keep_the_install` | SET16 | exit 127 |
| `test_set_s5_docs_adr_matrix_and_walkthrough` | SET17 | `README must lead its install section with ./scripts/setup.sh` |
| `test_set_presence_flag_toggles`, `test_set_state_file_untrusted_content_refused` | SET18 | exit 127 |

Red run: `cargo test --locked -p soos-invariants --all-features setup_contract` → 0 passed, 26 failed (first draft 24, revision tests added before Green).

## Migrated existing tests

None. No existing test was changed. During Phase 2 the new harness was corrected before Green (not weakened): tool lookup requires an execute bit (a non-executable `env` file on the PATH of the developer host), SET2 splits shell words without splitting paths (`packaging/pam/arch` is not the package `pam`), SET3 bans `authselect ` as a command rather than the `authselect/custom` path and allows `command -v tailscale`, and the `--help` parser reads only case-pattern lines.

## Flakiness check

`for i in $(seq 10); do cargo test --locked -p soos-invariants --all-features setup_contract -q || break; done` — see walkthrough 200 §8.

## Candid review follow-up (2026-10-13, CHANGES_REQUESTED → spec §13)

Tests were written first and observed Red against the reviewed script (`cargo test --locked -p soos-invariants --all-features setup_contract` → 24 passed, 4 failed):

| Test | Finding | Red evidence |
|---|---|---|
| `test_set_remote_own_session_never_borrows_the_desk_session` (new) | CR-1 | own session `Remote=yes` / seatless with `XDG_SESSION_ID=c7`: the desk session was borrowed and enrollment ran |
| `test_set_remote_session_skips_enrollment_with_command` (existing, unchanged assertions) | CR-1 | turned Red once the `loginctl` stub answered per session id: the old script fell back to the local desk session |
| `test_set_daemon_stop_marker_cleared_only_after_restart` (new) | CR-2 | `DAEMON_STOPPED=false must come after the successful systemctl start` |
| `test_set_full_run_as_root_builds_as_invoking_user` (strengthened: root `CARGO_HOME`, `RUSTUP_HOME`, `RUSTFLAGS` set; build `HOME`/`USER` asserted) | CR-3 | `CARGO_HOME=/root/.cargo RUSTUP_HOME=/root/.rustup RUSTFLAGS=-Cinjected-by-root` reached the user build |

Harness changes (strengthening only): the `loginctl` stub answers per session id; harness paths are baked into every stub so a stub still logs under `env -i`; the cargo stub also logs `HOME`, `USER`, `CARGO_HOME`, `RUSTUP_HOME`, `RUSTFLAGS`. No assertion was relaxed. After the fixes: 28 passed, 0 failed.

## CI follow-up (PR #385, 2026-10-13): fake account uid

`test_set_second_run_is_idempotent`, `test_set_presence_flag_toggles` and `test_set_damaged_install_and_reinstall` failed on the GitHub runner but passed locally. Each failed in the preflight of its second run: "The cargo target directory …/repo/target holds files not owned by the build user 'alice'".

- **Root cause:** a harness defect. The `id` and `getent` stubs hard-coded uid 1000 for the fake account `alice`. The test process creates every file, including the cargo stub's `repo/target`, so those files belong to the real uid of the test process. That uid is 1000 on the developer host, which hid the defect, and 1001 (`runner`) on GitHub's Ubuntu image. The first run passes because `repo/target` does not exist yet. The second run's ownership preflight (`find -H <target> ! -uid <build uid>`) then correctly refuses.
- **Reproduction:** the committed test binary run as uid 1001 in a user namespace (`unshare --user --map-user=1001 --map-group=1001 <test binary> setup_contract`) fails the same three tests with the same error (25 passed, 3 failed). The same binary run as uid 1000 passes (28 passed).
- **Fix (harness only):** the fake account now has the real uid of the test process (`STUB_UID`, baked into every stub). This affects `id -u`, `id -u alice`, `getent passwd`, `SUDO_UID` and the enrolled template file name. `real_uid()` asserts a numeric, non-root uid.
- **Unchanged:** `uid=1000` in log lines stays a role label ("the normal user"). No assertion changed, and the production ownership preflight is unchanged.
- **Result:** after the fix, the binary passes as uid 1000 and as uid 1001.
