# Candid Review Report

- **Date**: 2026-10-13
- **Target Branch**: `feat/network-lock`
- **Base (merge-base)**: `1b6d211`
- **Reviewed-Diff-Fingerprint**: `0f45e1004efce5d80c4f4886da662aeac5b480127c1cd28ea99189e49e9fe118`
- **Audited Files**: `.agents/skills/dev-workflow/references/project-facts.md`, `.github/workflows/ci.yml`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_network_lock.md`, `AI/auditor_constraints_network_lock.md`, `AI/tester_contract_network_lock.md`, `AI/walkthroughs/196_network_lock.md`, `Cargo.lock`, `Docs/NETWORK_LOCK.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `Docs/PAM_MODULE.md`, `Docs/README.md`, `crates/admin-cli/Cargo.toml`, `crates/admin-cli/src/args.rs`, `crates/admin-cli/src/error.rs`, `crates/admin-cli/src/lib.rs`, `crates/admin-cli/src/main.rs`, `crates/admin-cli/src/network_lock.rs`, `crates/admin-cli/tests/network_lock_install_test.sh`, `crates/admin-cli/tests/network_lock_install_tests.rs`, `crates/admin-cli/tests/network_lock_shell_tests.rs`, `crates/admin-cli/tests/network_lock_status_tests.rs`, `packaging/polkit/49-soos-network-lock.rules`, `packaging/udev/72-soos-network-lock-rfkill.rules`, `scripts/uninstall.sh`, `tests/docker/network_lock_pkcheck_test.sh`, `tests/invariants/src/lib.rs`, `tests/invariants/src/network_lock_contract.rs`

## 1. Executive Summary

The diff adds Part A of #359 (GitHub #365). It contains an opt-in polkit rule that answers `AUTH_ADMIN` for five NetworkManager Wi-Fi change and disconnect actions, an optional rfkill udev rule (off by default), and `soos-admin network-lock status|install|uninstall`. It also adds `scripts/uninstall.sh` removal, a Docker `pkcheck` CI job, invariant, unit and shell tests, and docs and an ADR.

The owner decisions recorded on #359 are respected:
- Scope is limited to Wi-Fi change and disconnect.
- The rule uses `AUTH_ADMIN`, never `_KEEP`.
- rfkill is a separate option, off by default.
- No separate admin account is required.

The root-owned install path does the following:
- validates every directory component, refusing symbolic links and group- or world-writable directories;
- holds the leaf directory open with `O_DIRECTORY|O_NOFOLLOW` and proves it by inode;
- creates the temporary file exclusively with `O_NOFOLLOW` and mode `0600`, then proves it by inode;
- sets mode `0644`, sets owner `0:0`, flushes the file, calls `renameat` on the directory descriptor and flushes the directory.

The status command is read-only and uses a fixed vocabulary. It never reads SSIDs, keys or connection names.

All new tests are additive and pass locally:
- 24 install tests, 42 status tests and the shell test;
- 8 `soos-invariants` network-lock tests;
- `cargo fmt --check` and `clippy -D warnings` on the two crates.

No CRITICAL or MAJOR finding. Three MINOR robustness and documentation items and three suggestions are listed in §4.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: `crates/admin-cli/tests/network_lock_install_test.sh`, `network_lock_install_tests.rs`, `network_lock_shell_tests.rs` and `network_lock_status_tests.rs` (all new); `tests/docker/network_lock_pkcheck_test.sh` (new); `tests/invariants/src/network_lock_contract.rs` (new); `tests/invariants/src/lib.rs` (+5 lines that register `mod network_lock_contract` under `#[cfg(all(test, unix))]`).
- Removed or changed assertions (`^-` with assert, `#[test]` or `proptest`): **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance or epsilon): **none**.
- Inline `mod tests` changes: **none**.

Justification: every change is additive. No existing contract was modified, so no Contract Migration is needed.

## 3. Deep Reasoning Audit

### Logic & Architecture

- **Exact action list.** Rule file = `NETWORK_LOCK_ACTION_IDS` = the docs table, pinned by an invariant. Checked against the host NetworkManager policy defaults (Arch):
  - `enable-disable-wifi` and `enable-disable-network` are `yes` for the active subject;
  - `network-control` is `yes` for active and inactive subjects;
  - `settings.modify.own` is `yes`;
  - `settings.modify.system` is `auth_admin_keep` and is overridden by `org.freedesktop.NetworkManager.rules` (`YES` for local `wheel`).

  All five listed actions are therefore really tightened for the owner. Excluded actions are justified in the ADR:
  - `sleep-wake` is already `no`;
  - `wifi.share.*` needs a connection, which needs `settings.modify.*` and `network-control`;
  - `checkpoint-rollback`, `reload`, hostname and DNS default to admin or are out of scope.

  PASS.
- **Rule ordering.** Checked against `/usr/share/polkit-1/rules.d` on the owner host: `20-gdm.rules` (gdm group only) sorts before `49-`; `org.freedesktop.NetworkManager.rules` and `empower.rules` sort after it. The rule therefore wins against the wheel `YES`. Earlier-sorting rules are documented as an undetected gap (Docs §6). PASS.
- **Status state machine.** Ordering: insecure, then modified, then unreadable, then unsupported backend, then installed or missing. A file failure always outranks `unsupported_backend`, so a tampered file is never hidden. Exit code 1 for `modified`, `insecure_permissions` and `unreadable`, matching the acceptance line plus the documented `unreadable` addition. PASS.
- **`may_modify_root`.** `/`, `//`, `.`, `/./` and an empty root are treated as the system root, which needs euid 0 (empty is refused). Paths such as `/etc/..` bypass this usability guard, but `checked_dir` then requires the caller to own every component, so a non-root caller can never write to the real `/etc`. PASS.
- **Installer integration.** The issue asks for a `network_lock` option of the one-command installer. That installer (#362) does not exist yet. The deviation is recorded (spec OD-6, ADR item 4), and the install step is exposed as a callable subcommand with a staging `--root`. Acceptable.
- **Backend detection.** Detection uses `/proc/<pid>/comm` without an owner check, so it is spoofable. See MINOR-1.
- **Status directory check.** Status checks only the leaf directory, while install checks every component. See MINOR-2.

### PAM Concurrency & Deadlines

`crates/pam` is untouched. There is no Tokio, async code or thread in the PAM path. The polkit agent helper never uses face authentication, which is documented. N/A, PASS.

### Panic Safety & Fail-Closed

- `network_lock.rs` contains no `unwrap`, `expect`, indexing or `panic!` on production paths. `unwrap_or` and `unwrap_or_else` have safe fallbacks.
- `serde_json` failure falls back to `"{}"`.
- `std::process::exit(code)` runs after line-buffered output that ends with a newline, so nothing is lost. The shell test S2 checks this through a pipe.
- **Fail-closed rule.** `AUTH_ADMIN` is the only result. There is no `YES`, no `NO` and no `_KEEP`, and every other action falls through. This is checked by the invariant and by the Docker sentinel differential: every other NetworkManager action stays `0` through a later `YES` sentinel, and each locked action returns `2 plain` (no retained authorization).
- Root stays authorized (P5): polkit short-circuits uid 0.
- **Tightening.** Recorded deviation (ADR item 2): for inactive or remote subjects whose default is `no`, the result becomes an administrator challenge. An administrator password already allows `sudo nmcli`, so no new privilege is reachable. A strictly monotone rule would need per-subject branches. Branching on subject, or falling through for inactive subjects, would re-open `network-control` (`allow_inactive=yes`). The chosen design is the safer one. See SUGGESTION-1.

### Test Integrity & Anti-Weakening

- All tests are new. Plausible wrong implementations are caught:
  - a `>= 0` versus `> 0` index guard (sentinel S2 covers every action that `pkaction` lists);
  - a `_KEEP` result (`retains_authorization_after_challenge` check);
  - a missing fall-through (P3 and S2);
  - a rule that is never loaded (P4);
  - writing through a symbolic-link file (S4) or a symbolic-link directory (S5);
  - a leftover temporary file (S1);
  - a one-byte change, truncation, an empty file or a file over the cap (status tests);
  - a group- or world-writable directory or file, a wrong owner, or `0750 root:polkitd` (status tests).
- The container-has-no-seat limitation is acknowledged. Active-local behavior is covered by the subject-independence invariant (`!code.contains("subject.")`) and the owner hardware check NLK12.
- PASS.

### Memory, Bounds & Secrets

- Every read is bounded: rule file 16 KiB, comm 64 B, sysfs dev 32 B, udev database 64 KiB, uptime 64 B, `/proc` entries 65 536. Each read uses `O_NOFOLLOW|O_CLOEXEC` and `take(cap+1)`.
- The status never opens NetworkManager connection files, D-Bus or `/proc/<pid>` beyond `comm`. The output uses fixed enums plus the two fixed production paths; `--root` is not echoed. **Privacy PASS.**
- **Install TOCTOU scenarios attempted:**
  - a symbolic link at the target: `entry_type` with `AT_SYMLINK_NOFOLLOW` refuses it, and `rename` would replace the link, not its target;
  - a hard link at the target: `rename` replaces the directory entry and leaves the other link untouched;
  - a component swapped between `lstat` and `open`: the inode check on the held descriptor catches it, and every component is root-owned and not writable by others anyway;
  - the temporary file name pre-created as a symbolic link: `create_new` plus `O_NOFOLLOW` fails;
  - the temporary file created by path but resolved elsewhere: an `fstat`/`fstatat` inode proof runs before `renameat`;
  - a partial write that polkitd could see: the temporary file is dot-named and does not end in `.rules`, and is published only by `rename`.

  Mode `0644` is set by `fchmod`, independent of umask, and owner `0:0` by `fchown` before publish. PASS.
- `scripts/uninstall.sh` uses `rm -f --` on the file or symbolic link itself and keeps the file when its parent is a symbolic link. See SUGGESTION-2.
- No `unsafe` code: the crate keeps `#![forbid(unsafe_code)]`, and the design deliberately avoids wrapping `openat` file descriptors. PASS.

### Supply Chain & Automation

- **New dependency.** `sha2` is a workspace dependency already in `Cargo.lock`. The lock file only gains the edge. No new crate or source. PASS.
- **CI job `network-lock-polkit`.**
  - `actions/checkout` is pinned by SHA (same as the other jobs) with `persist-credentials: false`.
  - Workflow-level `permissions: contents: read`; the job adds nothing.
  - No `${{ github.event.* }}` inside `run:`.
  - `timeout-minutes: 15`, `needs: lint`, and the job is wired into `ci-success`, whose `needs.*.result` loop fails on any non-success.
  - The Docker image is pinned by digest (`ubuntu:24.04@sha256:0081…`), identical to the pin in `pam_rollback_test.sh`.
  - The repository is mounted read-only.
  - The `apt-get` packages come from the signed Ubuntu archive and are not version-pinned, which is consistent with the existing Docker jobs.

  PASS.

### English-Only Policy

A grep of the added lines for common French words returned no hits. Code, comments, docs, ADR and walkthrough are in English. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `crates/admin-cli/src/network_lock.rs:134-143`: `ProcServiceProbe` matches `/proc/<pid>/comm` alone. Any local user can name a process `NetworkManager` or `iwd` (`prctl(PR_SET_NAME)`). On an iwd-only host, a spoofed name turns `unsupported_backend(iwd)` into `installed`, which misleads the owner. The status is diagnostic only and does not affect enforcement. Correction (follow-up): also require the `/proc/<pid>` owner to be uid 0 for `NetworkManager` and `iwd`, and the `systemd-network` user for networkd.
- **[MINOR]** `crates/admin-cli/src/network_lock.rs:410-421`: `rule_file_state` checks only the leaf directory (`rules.d`). Install checks every component (`/etc`, `/etc/polkit-1`). A non-root-owned or group-writable `/etc/polkit-1` (which would let its owner swap `rules.d`) still reports `installed`. The module doc at lines 154-155 describes the leaf only, so the code is consistent with its doc but weaker than the install check. Correction (follow-up): apply the same per-component check in status and report `insecure_permissions`.
- **[MINOR]** `tests/docker/network_lock_pkcheck_test.sh:11`: the header comment says "the six locked actions", but `LOCKED` has five (`sleep-wake` was dropped in plan round 1). Correction: change the comment to "five".
- **[SUGGESTION]** `packaging/polkit/49-soos-network-lock.rules:22-24`: for inactive or remote subjects, `AUTH_ADMIN` turns a shipped `no` (`enable-disable-wifi`, `enable-disable-network`) into an administrator challenge. This deviates from #359's literal "only tightens". It is privilege-equivalent (an admin password already allows `sudo`) and recorded in ADR item 2. Make sure the owner explicitly acknowledges this deviation on #365 or #359.
- **[SUGGESTION]** `scripts/uninstall.sh:204`: only the immediate parent is tested with `-L`. A symbolic-link `${DESTDIR}/etc/polkit-1` (an ancestor) is not detected. This is harmless for `DESTDIR=""`, where `/etc` is root-owned, but checking each component would match the Rust installer.
- **[SUGGESTION]** `crates/admin-cli/src/network_lock.rs:685`: a run killed between `create_new` and `rename` leaves `.<name>.<pid>.tmp` (root-owned `0600`, not loaded by polkitd or udev). Neither `uninstall` nor a later `install` removes it. Consider removing stale `.<name>.*.tmp` entries in the verified directory during uninstall.

## 5. Final Verdict

No CRITICAL or MAJOR finding. The polkit rule only answers `AUTH_ADMIN` (never `YES`, `NO` or `_KEEP`) and falls through for every other action. The root install path is safe against symbolic links, hard links and time-of-check-to-time-of-use races. The status output is private, and the new CI job is pinned with least privilege.

**VERDICT: APPROVED**
