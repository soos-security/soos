# Audit Constraints — GitHub #365: opt-in polkit Wi-Fi lock and `soos-admin network-lock`

- **Date**: 2026-10-13
- **Inputs**: `AI/architect_spec_network_lock.md` (with §11 Revision 1), `AI/plan_evaluator_report.md`, `AI/tester_contract_network_lock.md`, current `crates/admin-cli`, `scripts/uninstall.sh`, `.github/workflows/ci.yml`.
- **Branch**: `feat/network-lock`

## Checklist results

| # | Item | Grep / evidence | Result |
|---|---|---|---|
| 1 | Panic paths | `grep -rnE '\.unwrap\(|\.expect\(|panic!|todo!|unimplemented!|unreachable!' crates/admin-cli/src` | only inside `#[cfg(test)]` modules of `status.rs`, `logs.rs`, `pam_stack.rs` (pre-existing, test code). No PAM code touched. |
| 2 | Unsafe | `grep -rn unsafe crates/admin-cli/src`; `#![forbid(unsafe_code)]` in `lib.rs` and `main.rs`; `admin-cli` listed in `test_business_crates_forbid_unsafe_code` | none; `fchown`, `O_NOFOLLOW` and `O_DIRECTORY` are reachable through safe std APIs (`std::os::unix::fs::fchown`, `OpenOptionsExt::custom_flags`, `libc` constants only) |
| 3 | Output isolation | `crates/pam` untouched | n/a; printing only in `main.rs` (already `#[allow(clippy::print_stdout, clippy::print_stderr)]`) |
| 4 | Bounded I/O | spec §2 constants | every read bounded (constraints A-3, A-4) |
| 5 | Arithmetic | sizes are `u64` caps compared with `>`; `take(cap + 1)` uses `saturating_add` | constraint A-5 |
| 6 | Filesystem safety | spec §5 | constraints A-6 – A-10 |
| 7 | Secrets & privacy | status never opens NetworkManager connection files; output from fixed vocabulary | constraint A-11 |
| 8 | Fail-closed & lockout | rule never returns `NO`; root unaffected; status never reports a healthy lock it could not verify | constraints A-1, A-12 |
| 9 | Supply chain | `sha2` is already a workspace dependency (`Cargo.toml:54`, used by other crates); `Cargo.lock` gains only the edge `soos-admin-cli -> sha2` | no new crate, no new version, no `deny.toml` change |
| 10 | CI changes | new job `network-lock-polkit` | constraint A-13 |

## Constraints

| # | Constraint | Applies to (file::fn) | Verified by |
|---|---|---|---|
| A-1 | The rule never returns `YES`, `NO`, `AUTH_SELF*`, `AUTH_ADMIN_KEEP`, `NOT_HANDLED`; exactly one `return` (AUTH_ADMIN) inside the membership guard; no `subject.` read; every other action falls through | `packaging/polkit/49-soos-network-lock.rules` | `test_nlk_rules_file_only_returns_auth_admin`, `test_nlk_rules_file_falls_through_for_other_actions`, Docker P2/P3/P5 |
| A-2 | Rule contents single-sourced by `include_str!` from `packaging/`; no copy of the rule text in Rust | `network_lock.rs` constants | `test_nlk_rule_contents_single_sourced_from_packaging` |
| A-3 | Rule file read opens with `O_NOFOLLOW | O_CLOEXEC` and reads at most `MAX_RULE_FILE_BYTES + 1` bytes (`Read::take`); larger ⇒ `Modified`; comparison of SHA-256 digests | `network_lock.rs::rule_file_state` | `test_nlk_status_modified_when_over_the_read_cap`, invariant needle `O_NOFOLLOW`/`Sha256` |
| A-4 | `/proc` scan bounded by `MAX_PROC_ENTRIES` directory entries, `comm` by `MAX_COMM_BYTES`, sysfs `dev` by `MAX_SYSFS_DEV_BYTES`, udev db by `MAX_UDEV_DB_BYTES`; oversize ⇒ ignored (`comm`) or `Unknown` (rfkill); every read error mapped to a state, never `Err`/panic | `ProcServiceProbe::running`, `rfkill_access` | `test_nlk_proc_probe_ignores_oversize_comm_and_missing_proc`, `test_nlk_rfkill_unknown_on_malformed_dev_or_missing_database` |
| A-5 | No unchecked arithmetic: caps use `saturating_add(1)`; major/minor parsed with `u32::from_str` (no sign, no overflow) | same | clippy `arithmetic_side_effects` (`-D warnings`), malformed-dev test (`-1:5`) |
| A-6 | Install and uninstall validate every target directory **before the first write**: each component below `root` is a real directory (not a symlink), owned by uid 0 when `chown_root` (else the caller), not group- or world-writable; the final one is opened `O_DIRECTORY | O_NOFOLLOW` and matched by `(dev, ino)`; a missing directory is an install error, never created | `install_network_lock`, helper `checked_dir` | `test_nlk_install_refuses_a_symlinked_rules_dir`, `..._symlinked_ancestor_dir`, `..._missing_rules_dir`, `..._rfkill_with_missing_udev_dir_writes_nothing`, `..._refuses_group_or_world_writable_directories`, `..._as_root_requires_root_owned_directories`, `test_nlk_uninstall_refuses_group_or_world_writable_directories` |
| A-7 | Existing target that is a symlink (dangling or not) or not a regular file ⇒ refused; nothing created through the link | `write_rule_file` | `test_nlk_install_refuses_a_symlink_target`, `..._dangling_symlink_target`, `..._directory_at_the_target_name`, shell S4 |
| A-8 | Temp file: same directory, name starting with `.` and not ending in `.rules`, `create_new(true)` + `O_NOFOLLOW | O_CLOEXEC`, mode `0600` at creation; proven to be inside the verified directory (`fstat` = `fstatat(dirfd, tmp, AT_SYMLINK_NOFOLLOW)`) before any descriptor-relative call; `fchmod 0644` and `fchown(0,0)` (when euid 0) via the descriptor; `sync_all`; `renameat(dirfd, …)`; directory `sync_all`; temp removed by `unlinkat(dirfd, …)` on every error after the proof; a pre-existing temp name is never removed | `write_rule_file`, `write_and_publish` | `test_nlk_install_writes_identical_0644_rule_without_temp_file`, `test_nlk_install_replaces_a_hard_linked_target_with_a_new_inode`, `test_nlk_install_fails_cleanly_on_a_stale_temp_name`, shell S1 |
| A-9 | Uninstall unlinks (never follows) the target name with `unlinkat(dirfd, …)`, refuses symlinked or insecure directory components, `NotFound` ⇒ `Absent`, a directory at the name ⇒ error | `uninstall_network_lock` | `test_nlk_uninstall_*`, shell S6 |
| A-10 | `main` refuses install/uninstall on the real root unless euid 0 (`may_modify_root`), and an empty `--root`; status never requires root | `main.rs`, `may_modify_root` | `test_nlk_may_modify_root`, shell S8 |
| A-11 | Status output contains only the enum names, `rfkill_uaccess`, the two fixed production paths and fixed sentences; no staging-root path, pid, SSID, PSK, UUID or connection name; no file under `/etc/NetworkManager` is opened | `NetworkLockReport::format_table`, `to_json` | `test_nlk_status_output_has_no_network_names`, `test_nlk_status_json_shape` |
| A-12 | A file that cannot be verified is never reported healthy: `Unreadable` ⇒ exit 1; insecure and modified ⇒ exit 1, precedence per spec §4 | `NetworkLockReport::exit_code`, `network_lock_status` | `test_nlk_status_unreadable_*`, `test_nlk_status_insecure_*`, `test_nlk_status_modified_*` |
| A-13 | CI job: `needs: lint`, `timeout-minutes`, checkout pinned by SHA with `persist-credentials: false` (same as `pam-rollback`), no secrets, no `${{ github.event.* }}` in `run:`; added to `ci-success.needs`; workspace mounted read-only in the container | `.github/workflows/ci.yml`, `tests/docker/network_lock_pkcheck_test.sh` | `test_nlk_docker_pkcheck_job_wired_into_ci`, `test_read_write_workspace_mounts_overlay_target_with_a_volume` (existing) |
| A-14 | `scripts/uninstall.sh` removes the two files right after the systemd teardown and before `# 2. Roll Back PAM`, only when the parent directory is not a symlink (`[[ ! -L ... ]]`), honours `--dry-run` (exits before), uses `rm -f --` | `scripts/uninstall.sh` | `test_nlk_lock_is_opt_in_and_removed_on_uninstall`, shell S7 |
| A-16 | Warnings are fixed words in a fixed order (`rfkill_uaccess`, `rfkill_reboot_pending`, `iwd_dbus_bypass`); `rfkill_reboot_pending` only for an installed rfkill rule whose `ctime` is after `now - uptime`, uptime parsed as a finite non-negative `Duration` (`Duration::try_from_secs_f64`), no float-to-int casts; malformed or missing uptime ⇒ no warning | `network_lock_status`, `rfkill_rule_newer_than_boot` | `test_nlk_rfkill_reboot_pending_*`, `test_nlk_rfkill_no_reboot_pending_when_rule_predates_boot`, `test_nlk_status_warnings_have_a_fixed_order`, `test_nlk_status_warns_iwd_dbus_bypass_next_to_network_manager` |
| A-15 | English only; no new `#[allow]` without `reason` | all files | `./scripts/candid_review.sh`, clippy |

## Pre-existing violations found (not introduced by this change)

none relevant (the `unwrap` hits are inside `#[cfg(test)]` modules).

## Residual risks accepted (recorded in the ADR)

- TOCTOU between the directory checks and the `rename`: exploiting it requires write access to `/etc/polkit-1` or `/etc/udev`, i.e. root.
- A non-root `--root /etc/..` bypasses the friendly root guard, but the kernel refuses the writes (EACCES); the guard is UX, permissions are the control.
- Foreign polkit rules sorting before `49-` can override the lock; documented, not detected.

### Clearance: CLEARED
