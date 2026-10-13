# Walkthrough 199 — Network Lock Status Hardening and Stale Temp-File Cleanup

- **Date**: 2026-10-13
- **Issue**: GitHub-only #370 (candid review findings of PR #369 / #365), no backlog id. **Branch**: `fix/network-lock-status-hardening`. It is not registered in `scripts/sync_issue.py`; the commit message carries `Closes #370`.
- **Matrix criteria**: NLK13–NLK17 (NLK2, NLK3 and NLK6 unchanged)
- **Numbering**: 197 and 198 are reserved by other open branches.

## 1. Context & Objectives

The candid review of the network lock (walkthrough 196) left five findings, all fixed here:

1. **MINOR**: the backend probe matched process names only. Any user could name a process `NetworkManager` and turn `unsupported_backend` into `installed` on an iwd-only host.
2. **MINOR**: `status` checked only the leaf `rules.d` directory, while `install` checks every directory on the path.
3. **MINOR**: a Docker test comment said "six" locked actions; there are five.
4. **SUGGESTION**: `scripts/uninstall.sh` checked only the leaf directory for a symbolic link.
5. **SUGGESTION**: an interrupted install could leave a hidden `.tmp` file behind.

## 2. Architect Design

The design is `AI/architect_spec_network_lock.md` §12, with §12.11 as Revision 1.

- **G-1 (item 1)**: `ProcServiceProbe` keeps its shape (`{ root }`). After an exact `comm` match, it reads the complete lines of the first `MAX_PROC_STATUS_BYTES` (4096) bytes of `/proc/<pid>/status`. Two conditions must hold:
  - the `Name:` line equals the matched `comm`;
  - all four uids of the `Uid:` line (real, effective, saved, filesystem) are trusted: root, or for networkd also the `systemd-network` uid (`SYSTEMD_NETWORK_USER`).

  The `systemd-network` uid comes from the complete lines of the first `MAX_PASSWD_BYTES` (1 MiB) bytes of `<root>/etc/passwd`, never NSS. The `/proc/<pid>` owner is not used, because a non-dumpable process shows root there. Checking the real uid as well rejects a set-uid root program started through a link with a service's name.
- **G-2 (item 2)**: a private walker, `walk_dirs(root, rel, owner) -> DirWalk`, is shared by `checked_dir` (install/uninstall, messages unchanged) and `rule_file_state` (status). The results map to status states:

  | Walker result | Status state |
  |---|---|
  | `Missing` | `missing` |
  | `NotRealDir` / `Insecure` | `insecure_permissions` |
  | `Error` | `unreadable` |
- **G-3 (item 3)**: the Docker test comment now says "five".
- **G-4 (item 4)**: `uninstall.sh` step 1b has a new helper, `path_has_symlink_component`, which walks every component below `DESTDIR`. When it finds a symbolic link, the script warns and skips that file.
- **G-5 (item 5)**: `remove_stale_temp_files` removes `.<name>.<pid>.tmp` files.
  - The pid must be 1–10 digits with no leading zero and lie in `1..=i32::MAX`.
  - The pid must not be the current process, and `kill(pid, None)` must return `ESRCH`.
  - Only regular files are removed, with `fstatat` + `unlinkat` relative to the verified descriptor.
  - At most `MAX_RULES_DIR_ENTRIES` (4096) directory entries are examined.

  `install` runs the cleanup after both directories are validated and before the first write. `uninstall` runs it on each directory right after validating that directory. `uninstall.sh` does the same with the host `/proc`. admin-cli's existing `nix` dependency gains the `signal` feature; `Cargo.lock` is unchanged.

## 3. Plan Evaluation

`AI/plan_evaluator_report.md` was written by an independent sub-agent. Verdict: **APPROVED**, with MINOR findings F-1 … F-7, all folded into §12.11:

- F-1: corrected the wording of the uninstall order.
- F-2: documented the networkd/userdb limitation.
- F-3: a complete-lines prefix reader, so a line cut by the cap is never parsed.
- F-4: stronger fixtures:
  - impossible pids instead of a reaped child;
  - every uid position varied;
  - a non-host networkd uid (61234);
  - live pid 1 and a live child;
  - per-component ancestor loops.
- F-5: the `Name:` line must match `comm`, which closes the pid-reuse pairing.
- F-6: shell regex, pid bound and glob guard.
- F-7: a blast-radius row and the ADR amendment.

During evaluation, the first draft's missing `signal` feature was caught and fixed.

## 4. Tester Contract

See `AI/tester_contract_network_lock.md`, section "Follow-up — GitHub #370".

- New `crates/admin-cli/tests/network_lock_hardening_tests.rs` (14 tests).
- New shell script `network_lock_uninstall_script_test.sh` (U1–U4), run by the new `network_lock_shell_tests::test_nlk_uninstall_script_checks_ancestors_and_cleans_temp_files`.
- **Red**: a stub with only the new constants compiles. All 14 Rust tests fail on assertions (for example `left: RunningServices { network_manager: true, networkd: true, iwd: true }`, `left: Installed`, and `left: [".49-soos-network-lock.rules.4194304.tmp", ...]`). The shell test fails with `[FAIL] U1 the rule was removed through a symlinked ancestor`.
- **Contract amendments (setup only, assertions untouched)**: five existing tests in `network_lock_status_tests.rs` now also write a root `proc/<pid>/status` (`Name:` = comm, `Uid:` 0 0 0 0) for each fake process. The file diff has no removed line.
  - Three tests need it to keep their match: `test_nlk_proc_probe_matches_exact_comm_names`, `test_nlk_status_uses_the_probe_result_for_the_backend` and `test_nlk_status_output_has_no_network_names`.
  - Two tests stay discriminating with it: `test_nlk_proc_probe_ignores_non_numeric_entries_and_near_names` and `test_nlk_proc_probe_ignores_oversize_comm_and_missing_proc`.
- The existing `test_nlk_install_fails_cleanly_on_a_stale_temp_name` (current pid) keeps its contract, because the current process is alive.

## 5. Auditor Constraints

See `AI/auditor_constraints_network_lock.md`, section "Follow-up — GitHub #370": constraints B-1 to B-15, **Clearance: CLEARED**.

- B-1: complete-lines reader.
- B-2 to B-4: probe trust rules, with explicit service match arms.
- B-5, B-6: shared walker and its status mapping.
- B-7 to B-10: temp-name grammar, liveness rule, removal rules and call order.
- B-11: shell rules.
- B-12: no panics or unchecked arithmetic.
- B-13: fixed vocabulary.
- B-14: `Cargo.lock` byte-identical.
- B-15: 100-column doc lines.

The auditor's notes on the pre-evaluation drafts (cut-line reader, missing `Name:` check, catch-all arm, shell pid bound) were all applied. No pre-existing violations were found.

## 6. Implementation

Files changed:
- `crates/admin-cli/src/network_lock.rs`: new constants; `trusted_process_uids`, `account_uid`, `parse_decimal_u32`, `read_complete_lines` / `read_at_most`, `DirWalk` / `walk_dirs`, `is_stale_temp_name` and `remove_stale_temp_files`; `checked_dir` and `rule_file_state` now use the walker.
- `crates/admin-cli/Cargo.toml`: `nix` features `["time", "signal"]`.
- `scripts/uninstall.sh`: step 1b.
- `tests/docker/network_lock_pkcheck_test.sh`: comment only.
- The tests listed in §4.
- Docs and traceability: `Docs/NETWORK_LOCK.md` §3, §3.1, §8; ADR amendment note; matrix NLK13–NLK17; the spec, plan, tester and auditor sections.

The output vocabulary, JSON shape, public types and `main.rs` are unchanged. The literal NLK3 line `rm -f -- "${lock_file}"` still comes before `# 2. Roll Back PAM`.

## 7. Candid Review

Not run in this session. The orchestrator runs Phase 5 (`./scripts/candid_subagent.sh --prepare` and a fresh reviewer) on the final diff.

## 8. Verification Results

```bash
cargo fmt --all -- --check                                                    # pass
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings # pass
ORT_SKIP_DOWNLOAD=1 cargo check --locked -p soos-admin-cli --all-targets --all-features --target i686-unknown-linux-gnu  # pass
cargo test --locked --workspace --all-targets --all-features --no-fail-fast   # see §10
./scripts/candid_review.sh                                                    # see §10
git diff --exit-code Cargo.lock                                               # unchanged
```

Network lock tests: admin-cli 14 hardening + 42 status + 24 install + 2 shell (S1–S8, U1–U4); invariants `network_lock` 8. Flakiness: the four network lock test binaries ran 10 times in a row with no failure.

## 9. Known Limitations / Follow-ups

- networkd is recognised only as root or through the `/etc/passwd` entry of `systemd-network`. A userdb-only account or a symlinked `/etc/passwd` makes a networkd-only host report `installed`/`missing` instead of `unsupported_backend`; the exit code is unchanged and the case is documented.
- A set-uid root program that sets all four uids to root without authentication, started through a link with a service's name, would still be counted. No standard program does this; the risk is documented.
- `kill(pid, 0)` sees only the caller's PID namespace. An install running at the same time from a container that shares `/etc` could lose its temp file; it would then fail at `renameat` without changing the rule. This is documented.
- NLK12 (owner hardware check) is still pending, unchanged.

## 10. Workspace test totals

`cargo test --locked --workspace --all-targets --all-features --no-fail-fast`: 3515 passed, 0 failed, 6 ignored over 395 test binaries. `soos-invariants` was re-run after the documentation edits: 579 passed. `./scripts/candid_review.sh` (layer 1): passed.
