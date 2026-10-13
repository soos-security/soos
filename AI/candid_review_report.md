# Candid Review Report

- **Date**: 2026-10-13
- **Target Branch**: `fix/network-lock-status-hardening`
- **Base (merge-base)**: `37d03c4`
- **Reviewed-Diff-Fingerprint**: `e0d2c34b05d92e5564c94d5e2ed4a7364f0561eea0c222a6fda88bf5f127719d`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_network_lock.md`, `AI/auditor_constraints_network_lock.md`, `AI/tester_contract_network_lock.md`, `AI/walkthroughs/199_network_lock_status_hardening.md`, `Docs/NETWORK_LOCK.md`, `crates/admin-cli/Cargo.toml`, `crates/admin-cli/src/network_lock.rs`, `crates/admin-cli/tests/network_lock_hardening_tests.rs` (new), `crates/admin-cli/tests/network_lock_shell_tests.rs`, `crates/admin-cli/tests/network_lock_status_tests.rs`, `crates/admin-cli/tests/network_lock_uninstall_script_test.sh` (new), `scripts/uninstall.sh`, `tests/docker/network_lock_pkcheck_test.sh`

## 1. Executive Summary

The diff addresses all five items of GitHub #370 (follow-up of PR #369 / #365): (1) the backend probe credits a `comm` match only when `/proc/<pid>/status` of the same process names the same `comm` and all four uids are root (or the `/etc/passwd` `systemd-network` uid for networkd); (2) `status` now uses the same ancestor-directory walker (`walk_dirs`) as `install`/`uninstall`; (3) the pkcheck comment says five actions; (4) `scripts/uninstall.sh` checks every path component below `DESTDIR` for a symlink; (5) install/uninstall (Rust) and `uninstall.sh` remove stale `.<name>.<pid>.tmp` regular files of dead pids. No PAM code is touched. Verified locally: `cargo test -p soos-admin-cli` (all suites green, including the new shell test), `cargo test -p soos-invariants` (591 passed), `cargo clippy -p soos-admin-cli --all-targets -- -D warnings` clean, `cargo fmt --all -- --check` clean, `bash -n scripts/uninstall.sh` OK. No CRITICAL or MAJOR finding.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- Test files touched: `crates/admin-cli/tests/network_lock_hardening_tests.rs` (new), `network_lock_shell_tests.rs` (one new test appended, no line removed), `network_lock_status_tests.rs`, `network_lock_uninstall_script_test.sh` (new), `tests/docker/network_lock_pkcheck_test.sh`.
- `grep '^-.*(assert|#[test]|...)'`: **no hit** - no assertion or test attribute removed or changed.
- `grep '^+.*(#[ignore|should_panic|tolerance|epsilon)'`: **no hit**.
- `grep 'mod tests'`: **no hit** (no inline test module touched).
- `network_lock_status_tests.rs`: the only `-` line in its hunks is the file header; every change adds `proc/<pid>/status` fixture files (root uids, matching `Name:`) to five existing tests (`test_nlk_proc_probe_matches_exact_comm_names`, `..._ignores_non_numeric_entries_and_near_names`, `..._ignores_oversize_comm_and_missing_proc`, `test_nlk_status_uses_the_probe_result_for_the_backend`, `test_nlk_status_output_has_no_network_names`). Setup-only contract migration required by acceptance item 1 of #370 (a probe that now requires a root-owned status would otherwise see no status file); each insertion is marked "Contract amendment (GitHub #370)". The near-name test gives the near-name processes root status files, so its assertions still exercise the name check, not the uid check. Assertions unchanged.
- `tests/docker/network_lock_pkcheck_test.sh`: comment only ("six" -> "five"), #370 item 3.
- Existing `test_nlk_install_fails_cleanly_on_a_stale_temp_name` (unchanged) uses the current pid, which `is_stale_temp_name` explicitly excludes; it still passes and still asserts the file is kept.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Probe TOCTOU: `comm` and `status` are separate reads, but `Name:` and `Uid:` are both parsed from the single `status` buffer, and `Name:` must equal the matched `comm`; a pid reused between the two reads cannot pair one process's name with another's uids (test `..._requires_the_status_name_to_match_comm`). The `/proc/<pid>` inode owner is not used (correct: it is root for non-dumpable processes). `Name:` is kernel-escaped, so a user `comm` cannot inject a forged `Uid:` line. -> PASS
- Setuid-root via a symlink named `NetworkManager`: real/saved uid stay the user's, any untrusted position rejects (all four positions tested). Unprivileged user namespaces: the reader in the init namespace sees mapped (real) uids. -> PASS
- `systemd-network` uid from `<root>/etc/passwd` only (no NSS, no `getpwnam`), exact first-field match, ASCII-decimal third field, resolved lazily at most once per call; a near user name, a malformed uid and a line cut by the 1 MiB cap are ignored (tested). Trusted set for NM/iwd is `{0}` only. -> PASS
- `walk_dirs` reproduces `checked_dir`'s former loop exactly (ENOENT -> Missing, other errno -> Error, non-dir/symlink -> NotRealDir, owner/`0o022` -> Insecure, empty `rel` -> NotRealDir, error messages preserved); `rule_file_state` maps these to Missing / InsecurePermissions / Unreadable, consistent with the previous leaf-only mapping. rfkill and polkit paths are walked independently (a writable `etc/udev` does not affect the polkit rule; tested). -> PASS
- `is_stale_temp_name`: exact `.<name>.` prefix and `.tmp` suffix, 1-10 digits, no leading zero, `parse_decimal_u32` rejects signs/spaces, `i32::try_from` caps at `i32::MAX`, `pid <= 0` and own pid excluded; `kill(pid, None)` is signal 0 (no signal is ever sent); only `ESRCH` counts as dead, `EPERM`/`Ok` keep the file. -> PASS
- `remove_stale_temp_files`: type checked with `fstatat(AT_SYMLINK_NOFOLLOW)` relative to the verified directory fd; only `S_IFREG` is removed with `unlinkat(NoRemoveDir)`, so a symlink or directory carrying a stale name is never followed or removed (tested, symlink target content verified). Examination bounded by `MAX_RULES_DIR_ENTRIES`; `ENOENT` race tolerated; directory fsync after removal. Runs before the rule write on install and before rule removal on uninstall. -> PASS (see SUGGESTION 1, 2)
- `uninstall.sh`: `path_has_symlink_component` tests every component of the path below `DESTDIR`, including the leaf directory, with `[[ -L ]]` (never follows); on a hit the file and its temp files are skipped with a warning. Stale cleanup: the glob prefix is the exact name, the anchored regex rejects leading zeros and more than 10 digits, `(( <= 2147483647 ))` is safe (no leading zero, so no octal; 10 digits fit in 64-bit arithmetic), `[[ -f && ! -L ]]` restricts to regular non-symlink files, liveness comes from host `/proc` (never `${DESTDIR}/proc`), and `rm -f --` on an entry swapped to a symlink would only unlink the link. U1-U4 cover symlinked `etc`, `etc/polkit-1`, `etc/udev`, and live/near/leading-zero/out-of-range/symlink entries. -> PASS (see SUGGESTION 4)
- All five #370 items implemented; no scope creep. -> PASS

### PAM Concurrency & Deadlines
- No file under `crates/pam` changed; `soos-admin` is an admin CLI. Added reads are bounded files; no blocking NSS lookup introduced (explicitly avoided). -> PASS (N/A for PAM)

### Panic Safety & Fail-Closed
- No `unwrap`/`expect`/indexing/unchecked arithmetic in production hunks (`saturating_add`, `truncate`, `rposition`, `try_from`, `unwrap_or` only); clippy with the workspace lints is clean. Every probe read/parse failure means the process is ignored (never credited). Status ancestor failures yield `insecure_permissions`/`unreadable` -> exit 1. Cleanup errors abort install/uninstall before the rule is touched (the lock stays in place). -> PASS
- Documented degradation: an unresolvable `systemd-network` account makes a networkd-only host report `installed`/`missing` instead of `unsupported_backend` (diagnostic only, exit code unchanged). -> SUGGESTION 3

### Test Integrity & Anti-Weakening
- See section 2: no assertion removed or altered; existing-test edits are fixture setup only. New tests include positive controls (root-owned processes detected, a long status with `Uid:` inside the cap detected, a passwd entry inside the cap resolved, 0755 ancestors installed), so an always-false probe or an always-insecure status would fail them. The cap tests are constructed so that a reader parsing a cut line would trust uid 612 or read a `Uid:` line beyond 4096 bytes. -> PASS

### Memory, Bounds & Secrets
- `status` capped at 4096 bytes, `/etc/passwd` at 1 MiB, both opened `O_NOFOLLOW | O_CLOEXEC`, and only complete lines parsed; directory listing capped at 4096 entries; proc entries still capped by `MAX_PROC_ENTRIES`. `read_bounded` semantics (`TooLarge`) preserved via `read_at_most(cap + 1)`. No credentials, SSIDs or biometric data logged; status output fields unchanged. -> PASS

### Supply Chain & Automation
- Only change: `nix` feature `signal` added to an existing workspace dependency (no new crate). `scripts/uninstall.sh` uses only bash builtins plus `rm`; no CI workflow changed. -> PASS

### English-Only Policy
- All code, comments, docs, walkthrough and test messages are English. Walkthrough number 199 (197/198 reserved by other open branches, stated in the walkthrough). -> PASS

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `crates/admin-cli/src/network_lock.rs:910` - `remove_stale_temp_files` lists the directory by path (`fs::read_dir(&dir.path)`) instead of through the verified descriptor; harmless because the type check and `unlinkat` are descriptor-relative and the walk guarantees no writable ancestor, but listing via the fd would make the whole operation descriptor-bound.
- **[SUGGESTION]** `crates/admin-cli/src/network_lock.rs:902` - `kill(pid, 0)` liveness is local to the caller's pid namespace; a concurrent install in another pid namespace sharing the same `/etc` could see its temp file removed (its `renameat` then fails cleanly, no corruption). Worth one sentence in `Docs/NETWORK_LOCK.md`.
- **[SUGGESTION]** `crates/admin-cli/src/network_lock.rs:205` - the networkd uid is resolved only from `/etc/passwd`; on hosts where `systemd-network` lives only in another database, a networkd-only host reports `installed` rather than `unsupported_backend`. Accepted and documented trade-off (no NSS), noted for visibility.
- **[SUGGESTION]** `scripts/uninstall.sh:235` - liveness via `-e /proc/<pid>` can see a live process as dead under `hidepid` for a non-root caller; impact is nil in practice (a non-root caller can only modify a `--destdir` it owns), noted for completeness.

## 5. Final Verdict

**VERDICT: APPROVED**
