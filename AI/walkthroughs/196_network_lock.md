# Walkthrough 196 — Opt-In polkit Wi-Fi Lock and `soos-admin network-lock`

- **Date**: 2026-10-13
- **Issue**: GitHub-only #365 (Part A of #359), no backlog id — **Branch**: `feat/network-lock` (not registered in `scripts/sync_issue.py`; the commit message carries `Closes #365`)
- **Matrix criteria**: NLK1–NLK12

## 1. Context & Objectives

Owner request of 2026-10-12 (#359 Part A): changing, disconnecting or disabling Wi-Fi must need administrator authentication instead of being a free action of the logged-in session. Owner decisions (#359 comments of 2026-10-12 and 2026-10-13):
- the scope is Wi-Fi change and disconnect only;
- `AUTH_ADMIN` on every request;
- the rfkill udev rule is a separate option, off by default;
- there is no separate admin account;
- face verification is not used;
- the one-command installer (#362) does not exist yet, so the step ships as a command #362 can call.

## 2. Architect Design

Spec `AI/architect_spec_network_lock.md` (§11 holds Revision 1).
- `packaging/polkit/49-soos-network-lock.rules` answers `polkit.Result.AUTH_ADMIN` for five NetworkManager actions: `network-control`, `enable-disable-wifi`, `enable-disable-network`, `settings.modify.system` and `settings.modify.own`. It applies to any subject and falls through for everything else.
- `packaging/udev/72-soos-network-lock-rfkill.rules` holds the single rule `TAG-="uaccess"` on `/dev/rfkill`.
- New module `crates/admin-cli/src/network_lock.rs`:
  - embedded rule contents (`include_str!` from `packaging/`) and the constants `NETWORK_LOCK_ACTION_IDS`, `MAX_RULE_FILE_BYTES` (16 KiB), `MAX_PROC_ENTRIES`, `MAX_COMM_BYTES`, `MAX_SYSFS_DEV_BYTES`, `MAX_UDEV_DB_BYTES`, `MAX_UPTIME_BYTES`;
  - the enums `NetworkBackend`, `RuleFileState`, `RfkillAccess` and `NetworkLockState`;
  - the seams `NetworkLockEnv` (root + required owner) and `NetworkServiceProbe` (`ProcServiceProbe` reads `/proc/<pid>/comm`);
  - the functions `network_lock_status`, `install_network_lock`, `uninstall_network_lock` and `may_modify_root`.
- CLI: `soos-admin network-lock status|install [--rfkill]|uninstall`, each with `--root DIR`.
- `AdminCliError::NetworkLock`, and the workspace `sha2` dependency added to admin-cli.
- `scripts/uninstall.sh` removes both files before the PAM rollback. The new Docker test runs in a new CI job.

## 3. Plan Evaluation

`AI/plan_evaluator_report.md`, run by an independent sub-agent.

**Round 1: REVISION_REQUIRED.**
- MAJOR findings:
  - F-1: `sleep-wake` defaults to `no` for every subject, so `AUTH_ADMIN` would have widened it. It was dropped.
  - F-2: in a seatless container, the default for `network-control` already equals `auth_admin`, so the Docker test proved nothing. A sentinel-rule differential was added.
- MINOR findings:
  - F-3: new `iwd_dbus_bypass` warning.
  - F-4: new `rfkill_reboot_pending` warning.
  - F-5: more documented gaps (`docker` group, cached `sudo`, `auth-polkit=false`, `20-gdm.rules`).
  - F-6: install now checks the owner and mode of every directory and works relative to the directory descriptor.
  - F-7: hard-link and stale-temp-file tests.
  - F-8: the uninstall removal runs before the PAM rollback; the package hooks are deferred, and the docs say to uninstall first.
  - F-9: harness conventions.
  - F-10: `unreadable` is recorded as an addition to the issue's state list.

**Round 2: APPROVED.** Remaining MINOR items, all folded in:
- N-1: negative test for a rule older than boot;
- N-2: the uninstall-order invariant only counts code lines;
- N-3: text drift fixed;
- N-4: the root production install path is covered by the NLK12 hardware row.

## 4. Tester Contract

`AI/tester_contract_network_lock.md`. The tests:
- `crates/admin-cli/tests/network_lock_status_tests.rs` (42 tests);
- `network_lock_install_tests.rs` (24);
- `network_lock_shell_tests.rs` (1, running `network_lock_install_test.sh` S1–S8 against `mktemp` roots, including `scripts/uninstall.sh --destdir`);
- `tests/invariants/src/network_lock_contract.rs` (8);
- `tests/docker/network_lock_pkcheck_test.sh` (P1–P5, S1–S2).

Red evidence came from a compiling stub, which failed on assertions such as `left: Missing right: Installed` and `install: NetworkLock("not implemented")`. The refusal tests pass vacuously on a refuse-everything stub; each one is paired with a positive test. The tests that passed on the Phase 1 rule file were mutation-checked:
- `AUTH_ADMIN_KEEP`, `else { return polkit.Result.NO; }` and a `subject.local` guard each fail an invariant;
- `TAG+=` fails the rfkill invariant;
- in Docker, `AUTH_ADMIN_KEEP` fails P2 and `indexOf(...) > 0` fails S2.

Before hand-off, the fixtures were made independent of the umask; this changed setup only, not assertions. **Migrated existing tests: none.**

## 5. Auditor Constraints

`AI/auditor_constraints_network_lock.md`, constraints A-1 to A-16, **Clearance: CLEARED**.
- No panics in production code, and no `unsafe` (`fstatat`, `renameat` and `unlinkat` come from safe `nix` APIs; the temp file is proven by inode because `openat` would need `unsafe`).
- Every read is bounded and every read error maps to a state.
- Directories are validated before the first write; the target is replaced atomically; symlinks and hard links are never followed.
- The output uses a fixed vocabulary.
- Unverifiable or insecure files exit 1.
- The CI job follows the existing hardening.

## 6. Implementation

Files:
- `packaging/polkit/49-soos-network-lock.rules` and `packaging/udev/72-soos-network-lock-rfkill.rules` (new);
- `crates/admin-cli/src/network_lock.rs` (new), plus changes to `args.rs`, `main.rs`, `lib.rs`, `error.rs` and `Cargo.toml`, and the `Cargo.lock` edge to `sha2`;
- `scripts/uninstall.sh`;
- `.github/workflows/ci.yml` (job `network-lock-polkit`, added to `CI Success`);
- `tests/docker/network_lock_pkcheck_test.sh`;
- the tests listed in §4;
- `Docs/NETWORK_LOCK.md` (new), plus `Docs/PACKAGING_AND_PROVISIONING.md` §5, `Docs/README.md` and `Docs/PAM_MODULE.md`;
- the ADR in `AI/DECISIONS.md` and the matrix section `network-lock`.

Notable decisions:
- `install` is declarative: without `--rfkill` it removes a soos rfkill rule.
- A missing `/etc/polkit-1/rules.d` is an error; the directory is never created.
- A non-root `status` on Arch reports `unreadable` and says "run as root".
- The backend is detected from `comm`, not D-Bus.

## 7. Candid Review

Not run in this session. The orchestrator runs Phase 5 (`./scripts/candid_subagent.sh --prepare` and a fresh reviewer) on the final diff.

## 8. Verification Results

```bash
cargo fmt --all -- --check                                                    # pass
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings # pass
cargo check --locked -p soos-admin-cli --all-targets --all-features --target i686-unknown-linux-gnu  # pass
cargo test --locked --workspace --all-targets --all-features                  # pass (see below)
./scripts/candid_review.sh                                                    # pass
./tests/docker/network_lock_pkcheck_test.sh                                   # pass (P1-P5, S1-S2; 17 NetworkManager actions)
```

Network lock tests: admin-cli 67 (42 status, 24 install, 1 shell) and invariants 8. Workspace totals are in §10. Flakiness: the status, install and shell tests were run 10 times in a row with no failure. The only timing-dependent test, `test_nlk_rfkill_no_reboot_pending_when_rule_predates_boot`, has a margin of about 390 ms.

## 9. Known Limitations / Follow-ups

- **NLK12 (owner hardware check)**, on the Arch host with NetworkManager 1.58 and polkit 127, which is also the only run of the root-owned production install path:
  1. Run `sudo soos-admin network-lock install`.
  2. Switch Wi-Fi network and disconnect from the desktop, and run `nmcli radio wifi off`: each attempt must ask for the administrator password, with no caching.
  3. Run `sudo soos-admin network-lock status`: it should report `installed` with the `rfkill_uaccess` warning.
  4. Run `sudo soos-admin network-lock uninstall`: switching must no longer ask for a password.
- The package removal hooks do not remove the files. This is deferred to #362 and packaging; the docs say to uninstall first.
- Not detected:
  - foreign polkit rules that sort before `49-` (only `gdm` is known);
  - membership of the rfkill group;
  - D-Bus access through the Debian wpa_supplicant `netdev` group;
  - root-equivalent groups (the owner's account is in `docker`).
- The Docker test cannot create an active local subject (no seat). That case rests on the invariant that the rule never reads `subject`, and on NLK12.
- Face verification for polkit prompts needs a separate ADR on polkit session binding.
- Parts B (offline alerts) and later tiers of #359 are separate issues.

## 10. Workspace test totals

`cargo test --locked --workspace --all-targets --all-features --no-fail-fast`: 3500 passed, 0 failed, 6 ignored over 394 test binaries. A first run under a cold, fully parallel build hit the pre-existing timing test `soos-admin-cli` `status::systemctl_bound_tests::test_igf11_normal_output_is_parsed` (1 s bound on a fake `systemctl`; `crates/admin-cli/src/status.rs` is unchanged by this branch). It passed 5 out of 5 times in isolation and in the full re-run. It was not modified.
