# Plan Evaluation Report
- **Date**: 2026-10-13
- **Issue**: GitHub-only #365 — opt-in polkit Wi-Fi lock and `soos-admin network-lock status` (Part A of #359; no `AI/BACKLOG.md` id)
- **Branch**: `feat/network-lock`
- **Base commit**: `1b6d211`
- **Round**: 2 (re-evaluation of `AI/architect_spec_network_lock.md` §11 "Revision 1" against round 1 findings F-1 … F-10)
- **Inputs re-read**: spec §11, `packaging/polkit/49-soos-network-lock.rules`, `packaging/udev/72-soos-network-lock-rfkill.rules`, `tests/docker/network_lock_pkcheck_test.sh`, `tests/invariants/src/network_lock_contract.rs`, `crates/admin-cli/tests/network_lock_{status,install,shell}_tests.rs`, `crates/admin-cli/tests/network_lock_install_test.sh`, `Docs/NETWORK_LOCK.md`, `AI/tester_contract_network_lock.md`

Owner decisions OD-1 … OD-6 remain authoritative.

## 1. Coverage Matrix

| Acceptance line (#359 Part A, narrowed by #365 / OD) | Spec element (after Revision 1) | Status |
|---|---|---|
| Rule lists exactly the documented ids, never `YES`, never `NO`, falls through | R-1 (5 ids), static `test_nlk_rules_file_lists_exactly_the_documented_action_ids` (rule = constant = docs table, `sleep-wake`/hostname/DNS/wwan excluded), `…_only_returns_auth_admin` (single `return`, forbidden results, no `subject.`), `…_falls_through_for_other_actions` (exact guard text `indexOf(action.id) >= 0`, no `else`/`switch`) | Covered |
| Opt-in install writes root-owned `0644`, symlink refused, temp + `rename`; uninstall removes; temp-root shell test | R-6 (owner + mode checks on every component, dirfd-relative `fstatat`/`renameat`/`unlinkat`, inode proof of the temp file), R-7 (hard-link and stale-temp tests), `test_nlk_install_as_root_requires_root_owned_directories`, shell test S1–S7 incl. `scripts/uninstall.sh --destdir` | Covered |
| `status` states + `rfkill_uaccess`; injected seams; exit codes; no network names | §4 + R-3 `iwd_dbus_bypass`, R-4 `rfkill_reboot_pending`, fixed warning order test, privacy test | Covered (test-power gap N-1) |
| Docker `pkcheck`: `network-control` `auth_admin` with the rule, default without | R-2 sentinel differential S1/S2 over every `pkaction` NetworkManager id, plus P1–P5 real-default comparison; owner hardware row NLK12 for the active local subject | Covered (subject-class limit recorded) |
| Docs: face not used, rfkill / iwd / hardware switch / root gaps | `Docs/NETWORK_LOCK.md` §6 gap list (R-5) | Covered (wording inconsistency N-3) |
| OD-3 rfkill option off by default | D-4, declarative removal without `--rfkill` | Covered |
| OD-6 no host install by agents, staging root | `--root`, `may_modify_root` | Covered |
| Removal on uninstall | R-8 (a) removal before `# 2. Roll Back PAM`, invariant asserts order; (b) package hooks deferred to #362 with documented "uninstall before removing the package" | Covered (invariant power N-2) |

## 2. Round 1 Findings — Verification of Claimed Resolution

| Round 1 | Claimed resolution | Verified in files | Result |
|---|---|---|---|
| F-1 MAJOR `sleep-wake` | dropped; 5 ids | rule file has 5 ids; invariant asserts `len()==5` and that `sleep-wake` is absent; `Docs/NETWORK_LOCK.md:27` lists it as excluded ("already refused for every subject"); Docker `CONTROLS` keeps `sleep-wake` as an unchanged control | **Resolved** |
| F-2 MAJOR Docker differential | sentinel `90-soos-test-sentinel.rules` returning `YES` for all `org.freedesktop.NetworkManager.*`; S1 all rc 0 without soos rule; S2 locked ids `2 plain`, every other NM id `0 plain`, root `0 plain`; guard pinned statically | `tests/docker/network_lock_pkcheck_test.sh` S1/S2 iterate `pkaction` output (≥ 10 ids required), assert every `LOCKED` id exists in the policy; invariant pins `if (soosNetworkLockActions.indexOf(action.id) >= 0) {` and exactly one `return polkit.Result.AUTH_ADMIN;` over comment-stripped code | **Resolved**. An `indexOf > 0`, `NOT_HANDLED`, extra/missing id or `AUTH_ADMIN_KEEP` mutation now fails S2 or the invariant. |
| F-3 NM + iwd bypass | warning `iwd_dbus_bypass` | `test_nlk_status_warns_iwd_dbus_bypass_next_to_network_manager` (NM+iwd → warning, exit 0; NM alone → none; iwd-only → `unsupported_backend`, no warning); docs §6 | **Resolved** |
| F-4 stale rfkill ACL | warning `rfkill_reboot_pending` (rule `ctime` > `now − uptime`, `MAX_UPTIME_BYTES` 64, invalid uptime → no warning) | positive test and invalid-input negatives present | **Resolved in design**; negative timing case missing (N-1) |
| F-5 gap list | extended | `Docs/NETWORK_LOCK.md:93-99`: docker/lxd/libvirt, sudo timestamp, `run0` / `manage-units` retention, suspend, `auth-polkit=false`, `20-gdm.rules`, `empower.rules`, netdev for iwd and Debian wpa_supplicant | **Resolved** (wording N-3) |
| F-6 install enforces owner/mode, dirfd-relative | R-6 | tests: group/world-writable `rules.d`, `etc/polkit-1`, `etc` refused (install and uninstall); `chown_root` without root-owned dirs refused. Workspace `nix 0.29` (`fs` feature) offers `fstatat`/`renameat`/`unlinkat` on `Option<RawFd>` / `as_raw_fd()` without `unsafe`; `openat` returns a bare `RawFd`, so the path-created temp file plus `(dev, ino)` proof is a sound `forbid(unsafe_code)` compromise, given that every ancestor is now verified root-owned and not writable | **Resolved** |
| F-7 atomicity untested | hard-link + stale-temp tests | `…_replaces_a_hard_linked_target_with_a_new_inode` (outside file unchanged, inode differs); `…_fails_cleanly_on_a_stale_temp_name` (target and foreign temp untouched) | **Resolved**. The in-place-write mutant now fails. |
| F-8 removal paths | uninstall.sh removal before `# 2. Roll Back PAM`; package hooks deferred with reason | invariant `test_nlk_lock_is_opt_in_and_removed_on_uninstall` checks both paths and the order; docs line 47 "run … uninstall before removing the package" | **Resolved**; invariant power N-2 |
| F-9 harness conventions | read-only `:ro` mount (POA3 not applicable), digest default via `SOOS_DEBIAN_IMAGE:-`, no `${{ github.event`, job in `ci-success.needs` | script uses `-v "${repo}:/workspace:ro"` and pinned `ubuntu:24.04@sha256:008173c2…`; invariant asserts the digest, `:ro`, CI job, `run:` line and `ci-success` membership | **Resolved** |
| F-10 non-root `unreadable` | documented + ADR | `Docs/NETWORK_LOCK.md:65` | **Resolved** |

## 3. Pillar Analysis (round 2)

### Pillar 1 — Architecture & threat model
- Scenario: a remaining listed action whose default is already stronger than `AUTH_ADMIN`. Re-checked against the host NM 1.58 policy:
  - `network-control`, `enable-disable-wifi`, `enable-disable-network` and `settings.modify.own` are `yes` for the active subject;
  - `settings.modify.system` is `auth_admin_keep` (and Arch's rule grants `YES` to local `wheel`).

  Every listed id is therefore tightened for the active local subject. The only widening is for inactive or remote subjects (default `no`), and it is recorded as the equivalent-privilege deviation (D-2, R-1).
- Scenario: a rule sorting before `49-`. Documented; `status` does not parse foreign rules (accepted limitation).
- Result: PASS.

### Pillar 2 — PAM deadline & concurrency
- Untouched (no PAM, daemon or IPC change). All new reads are bounded (`MAX_UPTIME_BYTES` added).
- Result: PASS.

### Pillar 3 — Panic safety & fail-closed
- Scenario: a malformed uptime (`inf`, `NaN`, negative, empty, missing). It adds no warning and does not change the exit code, which the tests cover. Informational warnings never mask `modified` / `insecure_permissions` (a modified rfkill rule is not "pending").
- Scenario: a guard mutation in the shipped rule. It is now caught by the static invariant and by Docker S2.
- Result: PASS.

### Pillar 4 — Dependencies
- No new crate beyond the existing `sha2` edge. The dirfd operations use the workspace `nix` features already enabled. `forbid(unsafe_code)` is preserved, and the spec explicitly avoids `OwnedFd::from_raw_fd`.
- Result: PASS.

### Pillar 5 — Data confidentiality & file safety
- Scenario: an attacker-writable ancestor races the path-based temp creation. Every ancestor component is now required to be owned by the required owner and not group/world-writable, and the temp inode is proven with `fstatat(dirfd, …)` before `renameat`, so a swapped path makes the install abort. A foreign file with the temp name is never removed.
- Privacy is unchanged: no file under `/etc/NetworkManager` is opened, and the output uses a fixed vocabulary (`test_nlk_status_output_has_no_network_names`).
- Result: PASS.

### Pillar 6 — Test integrity & test power
- No existing test is modified. New contracts are additive.
- Mutants checked:
  - non-atomic write → fails (hard-link test);
  - G-first tag parsing, substring tag match, prefix `comm` match → fail;
  - guard off-by-one, extra or missing id → fail (S2 + invariant);
  - **"always warn `rfkill_reboot_pending` when the rule is installed and uptime is valid"** → passes every planned test (N-1);
  - **removal placed after `# 2. Roll Back PAM` but the path mentioned in a header comment earlier** → passes the order invariant (N-2).
- Result: FINDING (MINOR only).

## 4. Findings (round 2)

- **[MINOR] N-1 No negative timing test for `rfkill_reboot_pending`.** An implementation that skips the `ctime > boot` comparison passes every planned test, because all negatives use an invalid uptime, no rule, or a modified rule. — *Required change*: add a test where the installed rfkill rule is older than boot. Either back-date the file's `ctime` reference by writing it, sleeping ≥ 50 ms, then writing `proc/uptime` as `0.01 0.01`, so boot = now − 10 ms is later than the coarse `ctime`; or inject the clock through the env seam. Assert no `rfkill_reboot_pending`.
- **[MINOR] N-2 The uninstall order invariant matches the first textual occurrence.** `uninstall.find(path)` would be satisfied by a comment or `usage` mention placed before `# 2. Roll Back PAM` even if the actual `rm` came later. — *Required change*: search only non-comment lines containing `rm ` (or the removal helper) for each path, then compare positions.
- **[MINOR] N-3 Documentation and spec drift.**
  - `Docs/NETWORK_LOCK.md:94` says "NetworkManager configured with the iwd Wi-Fi backend is covered", but line 93 and the `iwd_dbus_bypass` warning say iwd stays directly reachable by `netdev`. Reword to "NetworkManager's own requests are covered; direct iwd D-Bus access is not".
  - Spec §2 (line 62 constant) and §3 D-1 (line 179 row) still list `sleep-wake`. §11 supersedes them, but update them inline so the developer does not copy the stale constant.
  - The invariant doc comment says "the same six actions" (should be five).
- **[MINOR / observation] N-4 Root production path only exercised by unit tests run as non-root.** The Docker test installs the rule with `install(1)`, not with `soos-admin`, so `chown_root = true` with real root-owned directories never runs in CI. This is acceptable for this PR because the CI cost is high and the logic is covered with `required_owner_uid` injection. Record it in the matrix (NLK12 owner hardware check covers the real `sudo soos-admin network-lock install`).

No CRITICAL or MAJOR finding remains. N-1 … N-3 are small, mechanical additions that the tester/developer phases can absorb without another evaluation round.

## 5. Verdict

VALIDATION_VERDICT: APPROVED
