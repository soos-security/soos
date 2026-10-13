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

## Follow-up — GitHub #370

- **Date**: 2026-10-13
- **Branch**: `fix/network-lock-status-hardening` (base `bcd3cd2`)
- **Inputs**: spec §12 with §12.11 Revision 1, `AI/plan_evaluator_report.md` (APPROVED, F-1 … F-7), tester contract section "Follow-up — GitHub #370", `crates/admin-cli/tests/network_lock_hardening_tests.rs`, `crates/admin-cli/tests/network_lock_uninstall_script_test.sh`, the new test in `network_lock_shell_tests.rs`, the setup amendments in `network_lock_status_tests.rs`, current `crates/admin-cli/src/network_lock.rs`, `crates/admin-cli/Cargo.toml`, `scripts/uninstall.sh` step 1b, `tests/invariants/src/network_lock_contract.rs`, and the pre-evaluation implementation drafts (`network_lock.rs`, `uninstall.sh` in the session scratchpad).

### Checklist results

| # | Item | Grep / evidence | Result |
|---|---|---|---|
| 1 | Panic paths | `grep -nE '\.unwrap\(|\.expect\(|panic!|todo!|unimplemented!|unreachable!' crates/admin-cli/src/network_lock.rs` | none (the only `unsafe` hit is a comment). The draft adds none: uids filled through `for slot in &mut uids` (no indexing); `unwrap_or` only. Constraint B-12 |
| 2 | Unsafe | `#![forbid(unsafe_code)]` unchanged | `kill`, `fstatat`, `unlinkat`, `lstat` are safe `nix` APIs; no raw fd wrapping |
| 3 | Output isolation | `crates/pam` untouched | n/a; no new print. Status vocabulary unchanged (no uid, pid or passwd field printed) |
| 4 | Bounded I/O | spec §12.2, §12.6 | `status` ≤ `MAX_PROC_STATUS_BYTES + 1`, passwd ≤ `MAX_PASSWD_BYTES + 1` (1 MiB, read lazily only when a `systemd-network` comm matches), listing ≤ `MAX_RULES_DIR_ENTRIES`; `/proc` scan bound unchanged. Constraints B-1, B-9 |
| 5 | Arithmetic | prefix cut and pid range | `saturating_add`, `checked_*`; `u32`/`i32` via `parse`/`try_from`, never `as`. Constraint B-12 |
| 6 | Filesystem safety | spec §12.3, §12.5, §12.6 | constraints B-5 … B-11 |
| 7 | Secrets & privacy | passwd is read for one fixed account; only its uid field is parsed | nothing from `status`/passwd is logged or printed; error messages contain paths only. No `password` keyword in strings ("passwd" only). Constraint B-13 |
| 8 | Fail-closed & lockout | probe errors ⇒ process ignored; status ancestor failures ⇒ exit 1 | no path makes the probe credit a backend on a read error; uninstall order unchanged (NLK3). Constraints B-2, B-6, B-11 |
| 9 | Supply chain | `nix 0.29.0` already locked; feature `signal` implies `process` (`Pid`) in nix 0.29 | no new crate, no new version, `Cargo.lock` unchanged (features are not locked), no `deny.toml` change; `crates/pam` does not depend on nix. Constraint B-14 |
| 10 | CI/workflow | `tests/docker/network_lock_pkcheck_test.sh` comment only; `scripts/uninstall.sh` logic change | no `.github/` change. Shell constraints B-10, B-11 |

Test integrity: `git diff --numstat` shows `network_lock_status_tests.rs` 70 added / 0 removed and `network_lock_shell_tests.rs` 25 / 0; `git diff | grep '^-'` lists no removed line. Every existing assertion is unchanged; the migrations are setup only (root `status` files), as the tester contract states.

### Constraints

| # | Constraint | Applies to (file::fn) | Verified by |
|---|---|---|---|
| B-1 | One prefix reader `read_complete_lines(path, cap)`: opens `O_NOFOLLOW \| O_CLOEXEC`, reads at most `cap.saturating_add(1)` bytes with `Read::take`; when more than `cap` bytes arrive, truncates to `cap` then cuts after the last `\n` found with `rposition` (no `\n` ⇒ empty buffer); a file of at most `cap` bytes is kept whole. No slice indexing (`truncate`/`get`), no unchecked `+ 1`. Used for `status` and `etc/passwd`; `read_bounded` keeps its `TooLarge` semantics for the rule file, `comm`, sysfs, udev db and uptime | `network_lock.rs::read_complete_lines`, `read_bounded` | `test_nlk_proc_probe_ignores_uid_line_beyond_the_read_cap` (both halves), `test_nlk_proc_probe_ignores_systemd_network_entry_cut_by_the_read_cap`, `test_nlk_status_modified_when_over_the_read_cap`, clippy `-D warnings` (`indexing_slicing`, `arithmetic_side_effects`) |
| B-2 | Probe credits a `comm` match only when, in the same `status` buffer, the first `Name:` line's value (after the `Name:` tag and its whitespace) equals the matched `comm` exactly, and the first `Uid:` line has exactly four ASCII-decimal `u32` fields, all in the trusted set. Any read error, missing line, wrong count or parse failure ⇒ that process is ignored, never an error or panic. The `/proc/<pid>` inode owner is never used | `ProcServiceProbe::running`, helpers (`process_uids` or equivalent) | `test_nlk_proc_probe_requires_the_status_name_to_match_comm`, `..._ignores_missing_or_malformed_status`, `..._requires_all_four_uids_trusted`, `..._ignores_user_owned_services` |
| B-3 | Trusted sets: `NetworkManager`, `iwd` ⇒ `{0}`; `systemd-network` ⇒ `{0} ∪ networkd_uid`. `networkd_uid` comes only from `<root>/etc/passwd` (first line whose first field is exactly `systemd-network`, third field ASCII decimal `u32`), never from NSS/`getpwnam`, never from the host when a test root is given; resolved at most once per `running()` call | `ProcServiceProbe::running`, `account_uid` | `test_nlk_proc_probe_trusts_systemd_network_only_for_networkd` (uid 61234; NM/iwd at that uid not credited; no-passwd case), grep `getpwnam\|User::from_name` = 0 in `network_lock.rs` |
| B-4 | Each service arm is explicit: the credit match lists `b"NetworkManager"`, `b"systemd-network"`, `b"iwd"` and `_ => {}`; no catch-all arm sets a service flag (the draft's `_ => found.iwd = true` depends on an earlier filter and must be rewritten) | `ProcServiceProbe::running` | review; `test_nlk_proc_probe_matches_exact_comm_names`, `..._ignores_non_numeric_entries_and_near_names` |
| B-5 | A single private walker `walk_dirs(root, rel, owner) -> DirWalk` checks every component below `root` (root itself not checked) with `lstat`, in path order, stopping at the first failure; `ENOENT` ⇒ `Missing`, other errno ⇒ `Error`, non-directory/symlink ⇒ `NotRealDir`, owner or `0o022` ⇒ `Insecure`. Both `checked_dir` and `rule_file_state` use it; `checked_dir` keeps its existing error messages and the `O_DIRECTORY \| O_NOFOLLOW` open + `(dev, ino)` match of the last component | `walk_dirs`, `checked_dir`, `rule_file_state` | `test_nlk_status_insecure_when_an_ancestor_dir_is_group_or_world_writable`, `..._is_a_symlink`, `test_nlk_status_rfkill_rule_insecure_when_its_ancestor_is_writable`, all existing `test_nlk_install_*`/`test_nlk_uninstall_*` |
| B-6 | Status mapping: `Missing` ⇒ `Missing`; `NotRealDir`/`Insecure` ⇒ `InsecurePermissions`; `Error` ⇒ `Unreadable`; only `Secure` proceeds to the existing file checks. No ancestor failure may yield `Installed` | `rule_file_state` | same tests; `test_nlk_status_unreadable_*` |
| B-7 | Temp-name grammar: exactly `.` + `name` + `.` + 1–10 ASCII digits, first digit not `0` + `.tmp`; digits parse to a pid in `1..=i32::MAX`; anything else (sign, 11 digits, leading zero, near name, other file's name, non-UTF-8 entry) is never a candidate. Parsing is done before, and independent of, any `kill` call | `is_stale_temp_name` (or a split `temp_name_pid` + liveness check) | `test_nlk_stale_temp_cleanup_keeps_live_foreign_and_non_regular_entries` |
| B-8 | Liveness: a candidate is stale only if its pid ≠ `std::process::id()` and `nix::sys::signal::kill(Pid::from_raw(pid), None)` returns exactly `Err(ESRCH)`; `Ok`, `EPERM` or any other errno ⇒ kept. Signal argument is always `None` (no signal is ever delivered) | `is_stale_temp_name` | `test_nlk_stale_temp_cleanup_keeps_live_foreign_and_non_regular_entries` (pid 1, live child), `test_nlk_install_fails_cleanly_on_a_stale_temp_name`, grep `kill(` in `network_lock.rs` = one call with `None` |
| B-9 | Removal: only an entry whose `fstatat(dirfd, entry, AT_SYMLINK_NOFOLLOW)` type is `S_IFREG`, with `unlinkat(dirfd, entry, NoRemoveDir)` on the verified descriptor; `ENOENT` on unlink ignored; listing bounded by `MAX_RULES_DIR_ENTRIES`; listing, type or unlink errors ⇒ `AdminCliError::NetworkLock`; directory `sync_all` after a removal | `remove_stale_temp_files` | `test_nlk_install_removes_stale_temp_files_of_dead_processes`, `test_nlk_uninstall_removes_stale_temp_files_of_dead_processes`, keep test (symlink target untouched, temp-named directory kept) |
| B-10 | Call order: install cleans each directory after both directories are validated (and after the missing-udev check of `--rfkill`) and before the first `write_rule_file`; uninstall cleans each directory right after its `checked_dir` and before its `remove_rule_file`; the polkit-then-udev order of uninstall is unchanged | `install_network_lock`, `uninstall_network_lock` | hardening install/uninstall tests, `test_nlk_install_rfkill_with_missing_udev_dir_writes_nothing`, review |
| B-11 | `scripts/uninstall.sh` step 1b: `path_has_symlink_component` walks every component of the lock directory below `DESTDIR` (DESTDIR itself not checked) and, on a symlink, warns `Keeping <file>: a directory on its path is a symbolic link.` and skips both the temp cleanup and the removal of that file. Temp cleanup only when `/proc/self` exists; each glob result guarded with `[[ -e \|\| -L ]]` (or `nullglob` restored afterwards); basename matched with the full-name regex `^\.<file>\.([1-9][0-9]{0,9})\.tmp$` where the file name is literal (quoted variable or escaped dots), pid from `BASH_REMATCH[1]`; `pid <= 2147483647` tested only inside an `if`/`&&` condition (never a bare `(( ))` under `set -e`); removed only when `-f && ! -L` and the host `/proc/<pid>` is absent (never `${DESTDIR}/proc`); `rm -f --`. The literal line `rm -f -- "${lock_file}"` stays in step 1b before `# 2. Roll Back PAM`; `--dry-run` still exits before step 1 | `scripts/uninstall.sh` step 1b | `test_nlk_uninstall_script_checks_ancestors_and_cleans_temp_files` (U1–U4), `test_nlk_lock_is_opt_in_and_removed_on_uninstall` (NLK3 order), shell S7, `bash -n`, `shellcheck` if available |
| B-12 | No `unwrap`/`expect`/`panic!`/slice indexing/unchecked arithmetic/`as` casts in the new code; `u64::try_from(len).unwrap_or(u64::MAX)` style only | `network_lock.rs` (all new fns) | `cargo clippy --workspace --all-targets -- -D warnings`, grep of checklist item 1 |
| B-13 | Fixed output vocabulary: no uid, pid, `Name:` value, passwd content or staging path added to the status table or JSON; new error messages (`cannot list`, `cannot remove`, `cannot flush`) name only the directory/entry path; no new word from the invariant log-keyword list | `NetworkLockReport::format_table`, `to_json`, `remove_stale_temp_files` | `test_nlk_status_output_has_no_network_names`, `test_nlk_status_json_shape`, invariants suite |
| B-14 | `crates/admin-cli/Cargo.toml`: only `features = ["time", "signal"]` on the existing `nix` line; no other dependency change; `Cargo.lock` byte-identical | `crates/admin-cli/Cargo.toml` | `git diff --exit-code Cargo.lock`, `cargo deny --locked check` |
| B-15 | English only; doc comments ≤ 100 columns (the draft's `is_stale_temp_name` doc line exceeds it); no `#[allow]` without `reason` | all touched files | `./scripts/candid_review.sh`, `cargo fmt --check`, clippy |

### Concrete problems in the implementation drafts

1. `network_lock.rs` draft, `process_uids` / `account_uid`: built on `read_prefix`, which keeps a line cut by the cap. Violates F-3 / B-1; `test_nlk_proc_probe_ignores_systemd_network_entry_cut_by_the_read_cap` fails (uid `612` credited). Replace with `read_complete_lines`.
2. `network_lock.rs` draft: no `Name:` check (F-5 / B-2); `test_nlk_proc_probe_requires_the_status_name_to_match_comm` fails.
3. `network_lock.rs` draft, `running`: the credit `match` ends with `_ => found.iwd = true` (B-4). Correct today only because of the earlier `_ => continue`; rewrite with explicit arms.
4. `network_lock.rs` draft, `walk_dirs`: an empty `rel` now yields `NotRealDir` (message "is not a real directory") instead of "empty target directory". Unreachable with the constant paths; acceptable, no action required.
5. `network_lock.rs` draft, `is_stale_temp_name` doc comment exceeds 100 columns (B-15). Otherwise B-7/B-8 hold: leading zero, length, digits-only, `i32` range and own pid are checked before `kill`, and only `ESRCH` removes.
6. `uninstall.sh` draft: no `pid <= 2147483647` bound, so U4 fails ("pid above i32::MAX removed"; `/proc/2147483648` never exists). The glob is unguarded (a non-matching pattern only survives because the pid regex rejects `*`), and the pid comes from prefix/suffix stripping instead of the full-name regex. F-6 / B-11 fix all three.
7. `uninstall.sh` draft is otherwise sound: `continue` skips both cleanup and removal on a symlinked path, host `/proc` is used, `-f && ! -L` guards the removal, and the NLK3 needle line `rm -f -- "${lock_file}"` stays before `# 2. Roll Back PAM`.

### Pre-existing violations found (not introduced by this change)

None. The `unwrap` hits in `crates/admin-cli/src` are inside `#[cfg(test)]` modules (unchanged since the GitHub #365 audit).

### Residual risks accepted (recorded in `Docs/NETWORK_LOCK.md` / ADR amendment)

- A setuid-root binary that sets all four uids to 0 without authentication, exec'd through a symlink named like a service, is still credited (no standard binary does this).
- `kill(pid, 0)` only sees the caller's PID namespace. A concurrent install from another namespace sharing `/etc` can lose its temp file and then fails cleanly at `renameat` (rule unchanged).
- networkd is credited only as root or as the `/etc/passwd` uid of `systemd-network`. A userdb-only account or a symlinked `/etc/passwd` reports `installed`/`missing` instead of `unsupported_backend` (exit code unchanged).
- `remove_stale_temp_files` lists the directory by path (`read_dir`) and not through the descriptor. Every type check and unlink is descriptor-relative, and swapping the path needs write access to a root-owned, owner-only-writable ancestor.

### Clearance: CLEARED
