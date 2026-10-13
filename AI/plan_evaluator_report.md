# Plan Evaluation Report
- **Date**: 2026-10-13
- **Issue**: GitHub-only #370 — network lock follow-up: status hardening and stale temp-file cleanup (candid review findings of PR #369; no `AI/BACKLOG.md` id)
- **Branch**: `fix/network-lock-status-hardening`
- **Base commit**: `bcd3cd2` (`origin/main`)
- **Plan evaluated**: `AI/architect_spec_network_lock.md` §12, including the orchestrator's mid-evaluation refinements to §12.1 and §12.6 (nix `signal` feature, no-leading-zero pid, per-directory uninstall cleanup order, `/proc/self` guard in the script). Sections 1-11 are the merged spec of PR #369.

## 1. Coverage Matrix

| Acceptance line (GitHub #370) / planned test | Spec element | Status |
|---|---|---|
| Item 1: backend probe requires a root-owned process (or `systemd-network` for networkd) | §12.2 G-1: `/proc/<pid>/status` `Uid:` line, all four uids in the trusted set; `etc/passwd` lookup of `systemd-network` | Covered (F-2, F-3, F-5) |
| Item 2: `status` applies the ancestor checks of `install` | §12.3 G-2: shared `walk_dirs` used by `checked_dir` and `rule_file_state` | Covered |
| Item 3: Docker test comment "six" → "five" | §12.4 G-3 (line 11 of `tests/docker/network_lock_pkcheck_test.sh`, the only "six actions" text) | Covered |
| Item 4: `scripts/uninstall.sh` checks every ancestor for a symbolic link | §12.5 G-4: `path_has_symlink_component` | Covered (F-6) |
| Item 5: clean stale soos temp files on the next install/uninstall | §12.6 G-5: Rust `remove_stale_temp_files`, shell cleanup in step 1b | Covered (F-1, F-4, F-6) |
| `test_nlk_proc_probe_ignores_user_owned_services` | G-1 step 3 | Mapped; fails against today's comm-only probe |
| `test_nlk_proc_probe_requires_all_four_uids_trusted` | G-1 step 3 | Mapped; can fail only if each position is varied (F-4) |
| `test_nlk_proc_probe_ignores_missing_or_malformed_status` | G-1 step 2 | Mapped (empty, 3/5 fields, non-decimal) |
| `test_nlk_proc_probe_trusts_systemd_network_only_for_networkd` | G-1 steps 1, 3 | Mapped; catches an NSS lookup only if the fixture uid differs from the host's (F-4) |
| `test_nlk_proc_probe_ignores_uid_line_beyond_the_read_cap` | G-1 step 2 | Mapped; has no positive counterpart (F-3) |
| `test_nlk_status_spoofed_network_manager_keeps_unsupported_backend` | G-1 end to end | Mapped |
| `test_nlk_status_insecure_when_an_ancestor_dir_is_group_or_world_writable`, `..._is_a_symlink`, `test_nlk_status_rfkill_rule_insecure_when_its_ancestor_is_writable` | G-2 | Mapped; fail against the current leaf-only check (the leaf `symlink_metadata` follows a symlinked `etc/polkit-1`) |
| `test_nlk_install_removes_stale_temp_files_of_dead_processes`, `test_nlk_uninstall_removes_...`, `test_nlk_stale_temp_cleanup_keeps_live_foreign_and_non_regular_entries` | G-5 | Mapped; the dead-pid fixture can flake (F-4) |
| `test_nlk_uninstall_script_checks_ancestors_and_cleans_temp_files` (U1-U3) | G-4, G-5 shell | Mapped; U1 fails against the current `-L "$(dirname ...)"` check |
| Existing contract `test_nlk_install_fails_cleanly_on_a_stale_temp_name` | G-5: the current pid is skipped | Holds: the current pid is skipped, `create_new` fails with `EEXIST`, and the file keeps "stale" |
| Matrix NLK13-NLK17, docs §3/§3.1/§8, walkthrough | §12.1, §12.10 | Mapped (F-7) |

## 2. Facts Verified Against Code

| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| `kill` available after adding the `signal` feature | `crates/admin-cli/Cargo.toml:21` (`features = ["time"]`), root `Cargo.toml:44` (`socket`, `fs`, `user`), nix 0.29.0 `src/sys/signal.rs` | `kill` needs `Signal` (`aio`/`process`/`signal`) and `unistd::Pid` (`process`); `signal = ["process"]`, so adding `signal` is enough. No workspace crate enables either feature today (`cargo tree -i nix -e features`) | Yes after the refinement. The §12.1 table still omits `crates/admin-cli/Cargo.toml` (F-7) |
| `ProcServiceProbe { root }` has only `root` | `network_lock.rs:116-118`, `main.rs:222` | `pub root: PathBuf` | Yes |
| `MAX_PROC_ENTRIES` 65 536, `MAX_COMM_BYTES` 64 | `network_lock.rs:59,61` | 65 536 / 64 | Yes |
| Exact comm match, `systemd-network` = 15-byte truncation | `network_lock.rs:138-144` | exact bytes after one `\n` is stripped | Yes |
| `rule_file_state` checks only the leaf directory | `network_lock.rs:410-421` | leaf `symlink_metadata` only | Yes |
| `checked_dir` walks every component below the root, never the root itself | `network_lock.rs:598-647` | as stated | Yes |
| Temp name `.{name}.{pid}.tmp`, 0600, `create_new` | `network_lock.rs:685-693` | `format!(".{name}.{}.tmp", std::process::id())`, so never a leading zero | Yes |
| Uninstall order: validate polkit dir → remove → validate udev dir → remove | `network_lock.rs:808-819` | as stated; the refined §12.6 keeps it | Yes (F-1 on §12.7 wording) |
| `read_bounded` semantics | `network_lock.rs:529-541` | returns `TooLarge` with no data above the cap; it is not a prefix reader | Plan needs a prefix reader (F-3) |
| `uninstall.sh` step 1b: `-L "$(dirname ...)"`, `rm -f -- "${lock_file}"` before `# 2. Roll Back PAM` | `scripts/uninstall.sh:197-210` | as stated | Yes |
| NLK3 invariant (code line order, message text not asserted) | `tests/invariants/src/network_lock_contract.rs:258-305` | as stated | Yes |
| Docker comment "six locked actions" | `tests/docker/network_lock_pkcheck_test.sh:11` | "six" | Yes |
| Matrix rows end at NLK12 | `AI/VERIFICATION_MATRIX.md` | NLK1-NLK12 | Yes |
| Latest walkthrough | `AI/walkthroughs/` (`origin/main`) | 196 | Plan says 199 (F-7) |
| ADR item (6): backend "from `/proc/<pid>/comm`" | `AI/DECISIONS.md:132` | as quoted | Will become stale (F-7) |
| Other `ProcServiceProbe` users | `grep` over `crates`, `tests` | only `network_lock_status_tests.rs`, `main.rs` (install tests use `NmProbe`, shell S2 `--root` has no `proc`) | Amendment list complete |

## 3. Pillar Analysis

### Pillar 1 — Architecture & threat model
- Spoofing scenario: a local user names a process `NetworkManager` (`prctl(PR_SET_NAME)`, or exec through a symlink of that name) on an iwd-only host. The `Uid:` line shows the kuid translated into the reader's namespace, so being "root" inside a user namespace does not help. A setuid-root binary started through a symlink keeps the user's real uid, and the four-uid rule rejects it. Rejecting the inode owner of `/proc/<pid>` is right: `/proc/<pid>` entries of a non-dumpable process are root-owned. The `status` approach is sound. `status` stays world-readable (hidepid already hid `comm` too, so there is no regression).
- Pid-reuse scenario: `comm` and `status` are two separate opens. If the spoofing process exits between them and a root process takes the pid, the root uids are credited to the spoofed name. The window is tiny and the attacker does not control the incoming process (F-5).
- Unresolved networkd uid: if the `systemd-network` uid cannot be resolved (passwd symlink refused by `O_NOFOLLOW`, userdb-only account, malformed entry), networkd is not credited, and a networkd-only host reports `installed`/`missing` instead of `unsupported_backend`. For iwd/networkd, not crediting is the misleading direction (F-2).
- Result: FINDING (MINOR: F-2, F-5).

### Pillar 2 — PAM deadline & concurrency
- Failure scenario considered: none applies. `soos-admin` is not the PAM module, NSS is deliberately avoided (no LDAP/SSSD blocking), and every new read is bounded (`MAX_PROC_STATUS_BYTES`, `MAX_PASSWD_BYTES`, `MAX_RULES_DIR_ENTRIES`).
- Result: PASS.

### Pillar 3 — Panic safety & fail-closed
- A malformed `status` is ignored, so fail-closed holds for the NetworkManager credit. A prefix cut of `etc/passwd` inside the `systemd-network` line would produce a truncated uid (`192` → `19`). Reusing `read_bounded` would instead drop a real service whose `status` exceeds 4096 bytes (F-3).
- Uninstall scenario: the polkit rule is removed, then the udev-directory cleanup fails. §12.7 says "exit 1, rule files untouched", which is no longer true for uninstall under the kept per-directory order (F-1).
- No `unwrap`/`expect`; `kill`, `fstatat` and `unlinkat` are safe `nix` APIs; `forbid(unsafe_code)` is unaffected.
- Result: FINDING (MINOR: F-1, F-3).

### Pillar 4 — Dependencies
- Scenario: the developer calls `nix::sys::signal::kill` without the feature, and the build fails. After the refinement, §12.1 enables `signal` on the existing `nix` dependency. No new crate is added; `Cargo.lock` lists packages and versions, not features, so it does not change; `deny.toml` is unaffected. No `opencv`/`nokhwa`.
- Result: PASS (bookkeeping gap in the §12.1 table, F-7).

### Pillar 5 — Data confidentiality
- Scenario: the new reads leak uids or account names. Nothing new is printed: vocabulary and JSON shape are unchanged, and `test_nlk_status_output_has_no_network_names` still guards the pid. Cleanup unlinks only soos-pattern regular files, relative to the verified directory descriptor, in directories only their owner can write. The `fstatat`→`unlinkat` TOCTOU needs the directory owner (root), so it is not exploitable. Liveness via `kill(pid, 0)` cannot remove a live same-namespace install: `Ok`/`EPERM` keep the file, and PID reuse can only keep one.
- Result: PASS.

### Pillar 6 — Test integrity
- The declared amendments are setup-only and necessary. Adding root `proc/<pid>/status` files (`Uid:\t0\t0\t0\t0`) to `test_nlk_proc_probe_matches_exact_comm_names`, `test_nlk_status_uses_the_probe_result_for_the_backend` and `test_nlk_status_output_has_no_network_names` changes no assertion. Adding the same files to the two no-match tests keeps their power against a comm-matching bug.
- I checked every existing status test against the G-2 walker; all keep their verdict. Examples: `missing_when_polkit_rules_dir_absent`; `insecure_when_owner_is_not_the_required_owner` now fails at `etc`, same verdict; `insecure_when_rules_dir_is_a_symlink`; `unreadable_when_rules_dir_cannot_be_searched` (a 0000 leaf passes `is_secure`, the file `lstat` gives `EACCES`). The immutable stale-temp contract still holds.
- Test power: F-3 and F-4.
- Result: FINDING (MINOR: F-3, F-4).

## 4. Findings

- **[MINOR] F-1 — §12.7 overstates "rule files untouched" for uninstall.** The refined §12.6 keeps the uninstall order (polkit dir validated, cleaned and its rule removed, then the udev dir). A cleanup or listing error in the udev directory therefore exits 1 after the polkit rule is already gone. — Required plan change: §12.7 should say "install: before any rule change; uninstall: before the rule file of the directory being cleaned is removed (the earlier directory may already be done, as with today's validation order)".

- **[MINOR] F-2 — For iwd/networkd, "not credited" is the misleading direction.** An unresolvable `systemd-network` uid drops a real networkd. Result: `installed`/`missing` instead of `unsupported_backend`. — Required plan change: document this in `Docs/NETWORK_LOCK.md` §3.1 and the ADR amendment. networkd is credited only as root or as the uid of the `systemd-network` entry in `/etc/passwd`; a missing entry, a userdb-only account or a symlinked `/etc/passwd` are not resolved. Optionally pin the behaviour with a sentinel test (no passwd + networkd at uid 192 → not `Networkd`).

- **[MINOR] F-3 — Prefix-read semantics.** `read_bounded` returns `TooLarge` without data. If it is reused, a genuine root service whose `status` exceeds 4096 bytes is dropped, contradicting "only this prefix is examined". A real prefix read of `etc/passwd` can end inside the `systemd-network` line. — Required plan change: specify a prefix reader that keeps up to `cap` bytes and parses only complete `\n`-terminated lines when the file is longer than the cap. Add a positive test: a `status` over the cap with `Uid:` near the top is trusted, which fails against `read_bounded` reuse. Add a passwd test where the cap cuts the `systemd-network` line → `None`.

- **[MINOR] F-4 — Fixture pinning for test power and determinism.**
  - (a) A "spawned and reaped child" pid can be reused before the call. Use a pid that cannot exist, e.g. `4194304` (`PID_MAX_LIMIT`; pids are `< pid_max ≤ 4194304`) or `2147483647`, in Rust and in U3.
  - (b) Vary each uid position separately (`1000 0 0 0`, `0 1000 0 0`, `0 0 1000 0`, `0 0 0 1000`), or an implementation that checks only the real and effective uids passes.
  - (c) The networkd test needs a fixture uid that is not the host's `systemd-network` uid, plus an assertion that `NetworkManager` running at that uid is not credited. This catches a `getpwnam` lookup and a trust set shared by all services.
  - (d) "keeps live foreign" needs pid 1 (`EPERM` for the non-root CI user), a live child of the test (`Ok`), and a temp-named symlink whose outside target stays untouched. This catches an implementation that treats `EPERM` as dead.
  - (e) The G-2 ancestor tests should loop over each component (`etc`, `etc/polkit-1`, `etc/udev`) so that checking only the parent fails.

- **[MINOR] F-5 — `comm`/`status` pid-reuse race.** One process's name can be paired with another's uids. — Required plan change: match the `Name:` line from the same `status` buffer (keeping `comm` as the cheap first filter is fine; the three names contain no kernel-escaped characters), or document the residual race.

- **[MINOR] F-6 — Shell matching details under `set -euo pipefail`.**
  - Expand the glob with `nullglob` (restored afterwards) or guard it with `[[ -e || -L ]]`.
  - Extract the pid with the full-name regex `^\.<file>\.([1-9][0-9]{0,9})\.tmp$` and enforce `≤ 2147483647` like the Rust rule.
  - Remove only `-f && ! -L` entries in a directory that passed G-4.
  - Use `/proc/<pid>` on the host, never `${DESTDIR}/proc`.

- **[MINOR] F-7 — Bookkeeping and documentation drift.**
  - (a) Add `crates/admin-cli/Cargo.toml` (nix `signal` feature) as a row of the §12.1 blast-radius table so the auditor sees it.
  - (b) The ADR "[2026-10-13] Opt-In polkit Wi-Fi Lock" item (6) says the backend comes from `/proc/<pid>/comm`. "No new ADR" is acceptable, but append an *(Amended 2026-10-13, GitHub #370: ...)* note covering the uid rule, status/install symmetry and temp cleanup, following the project convention.
  - (c) The walkthrough is numbered 199 while `origin/main` ends at 196. Confirm 197/198 are reserved by open branches, or use the next free number.

Observations (no change required):
- `kill(pid, 0)` only sees pids in the caller's PID namespace. An install from another namespace (container with a bind-mounted `/etc`) would have its temp file removed, then fail cleanly at `renameat` with `ENOENT`, leaving the rule unchanged. One sentence in the docs is enough.
- `nix::dir::Dir::openat` (feature `dir`) would list through the verified descriptor, avoiding the path-based `read_dir`. The current design is safe because every unlink is relative to the descriptor.
- Residual G-1 risk: a setuid-root binary that sets all four uids to 0 without authentication and is exec'd through a symlink named `NetworkManager`. No standard binary is known to do this; document it only.

## 5. Verdict
VALIDATION_VERDICT: APPROVED

Nothing is CRITICAL or MAJOR. Before the orchestrator's refinement, one finding was MAJOR: `nix::sys::signal::kill` cannot compile in `soos-admin-cli` without the `signal` feature. The refined §12.1 resolves it. Findings F-1 to F-7 are MINOR. The architect should fold them into §12 before the tester writes the contract, especially F-3 (prefix reader and its positive test) and F-4 (fixture pinning), because they decide whether the planned tests can fail.
