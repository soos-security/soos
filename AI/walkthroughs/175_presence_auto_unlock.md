# Walkthrough 175 — Presence Auto-Unlock of Locked Local Sessions

- **Date**: 2026-10-02
- **Issue**: GitHub #323 (GitHub-only, no backlog sub-issue; the branch is not registered in
  `scripts/sync_issue.py`, the commit carries `Closes #323`) — **Branch**: `feat/presence-auto-unlock`
- **Matrix criteria**: PAU1–PAU29 (`AI/VERIFICATION_MATRIX.md` section `presence-auto-unlock`);
  PAU1–PAU20 and PAU22–PAU29 `✅ Verified`, PAU21 (hardware) `⬜ Pending`
- **Phase documents**: `AI/architect_spec_presence_unlock.md` (revision 4),
  `AI/plan_evaluator_report.md` (rounds 1–5), `AI/tester_contract_presence_unlock.md` (220 tests,
  rounds 1–4), `AI/auditor_constraints_presence_unlock.md` (constraints 1–33, re-audit `CLEARED`)

## 1. Context & Objectives

Before this change, a soos face match could only unlock a screen through PAM, when the locker
called `pam_authenticate`, usually after a keypress (GNOME raises its shield; `swaylock` waits
for Enter). GitHub #323 asks for the Windows Hello behaviour: when the owner of a locked local
session comes back in front of the camera, the session unlocks on its own.

Owner decisions of 2026-10-02 (binding, recorded in the ADR "Presence Auto-Unlock Through
logind" of `AI/DECISIONS.md`):

1. **On by default**: `[presence] enabled = true` applies even when there is no `daemon.toml`.
2. **One global limit of 40 attempts per 60 s per UID** (`RateLimitConfig::DEFAULT_MAX_ATTEMPTS`,
   previously 5). It covers every face request, `sudo` and GDM included, plus each presence
   scan. There is no separate quota. Presence never uses the last 5 attempts of a window.
3. **Same pipeline**: capture, detection, PAD consensus (`k = 3` of `n = 5`, any spoof vetoes),
   then cosine match with `[pipeline.thresholds]`.
4. **Unlock goes through systemd-logind** (`Manager.UnlockSession`).
5. **Q1**: respect `pam_faillock` and account / password expiry. A locked, expired or
   undeterminable account is never unlocked.
6. **Q2**: presence spoof vetoes are logged and never sealed as evidence.
7. **Q3**: `/etc/soos/gdm.disable` does not stop presence. Only `/etc/soos/presence.disable` and
   the global `/etc/soos/disabled` do.

## 2. Architect Design

- **New daemon modules** (`#![forbid(unsafe_code)]`, no `unwrap`/`expect`/`panic!`):
  - `crates/daemon/src/consensus.rs`: the dispatcher's camera wake and PAD/match loop moved out
    unchanged as `run_face_consensus`. It is now the only place that builds `PadAggregator`.
  - `crates/daemon/src/presence/`, with these modules:
    - `config`: `[presence]`;
    - `logind`: `PresenceLogind` trait, `ZbusLogind`, `session_state_from_properties`;
    - `display`: DRM DPMS probe;
    - `switch`: kill switch;
    - `tracker`: lock periods, grace, scan interval;
    - `account`: the account guard;
    - `worker`: the 1 s tick.
- **Inference gate** (`crates/daemon/src/inference.rs`): adds `InferencePriority`,
  `InteractiveDemandGuard` (held by the dispatcher for every `Auth` request) and
  `try_acquire_background`. A presence scan never waits for the slot, never starts while PAM is
  waiting, and is preempted between captures.
- **Policy** (`crates/policy`): `RateLimiter::check_and_record_with_reserve` and
  `AuthorizationEngine::record_attempt_with_reserve` prune, evaluate and record in a single
  `&mut self` call. `DEFAULT_MAX_ATTEMPTS` is now 40.
- **Logind access**: `zbus` 5 (`default-features = false`, `tokio`, 22 MIT / Apache-2.0 crates)
  on the pinned `unix:path=/run/dbus/system_bus_socket`. The connection has no object server, no
  well-known name, no signal match and no property cache. It reads `LockedHint` with a fresh
  `GetAll` (logind does not write it to `/run/systemd/sessions`, checked on systemd 262).
- **Constants** (`crates/daemon/src/presence/mod.rs`):

  | Constant | Value |
  |---|---|
  | `LOCK_POLL_INTERVAL_MS` | 1000 |
  | `PRESENCE_RESERVED_ATTEMPTS` | 5 |
  | `DBUS_CALL_TIMEOUT_MS` | 500 |
  | Reconnect backoff | 1–30 s |
  | `MAX_ALLOW_TO_UNLOCK_MS` | 1000 |
  | `UNLOCK_CONFIRM_TIMEOUT_MS` | 5000 |
  | `ACCOUNT_CHECK_TIMEOUT_MS` | 500 |
  | `MAX_PRESENCE_SEAT_SESSIONS` / `MAX_TRACKED_LOCKED_SESSIONS` | 16 / 16 |
  | Faillock defaults (`deny` / `fail_interval` / `unlock_time`) | 3 / 900 s / 600 s |

  `[presence]` defaults are `scan_interval_ms = 2000` and `lock_grace_ms = 3000`. Both must be
  within `1000..=60000`.
- **Invariants**: ARCHITECTURE invariant 3 gains the presence clause, and invariant 6 is new.
  Logind is asked to unlock only after a fresh, single-use `Allow` that comes after the grace,
  and only if a fresh re-check of the same session still finds it bound and locked and the
  account check still passes. Any error leaves the session locked.

## 3. Plan Evaluation

`AI/plan_evaluator_report.md` ran five rounds and ended with `VALIDATION_VERDICT: APPROVED`.

- **Round 1** (`REVISION_REQUIRED`):
  - F1: migrate `daemon_docs_contract::CONFIG_FILE_TABLES` from 9 to 10 entries;
  - F2: the vacuous "same epoch" re-check was dropped and the residual risk stated;
  - F3: explicit `#![forbid(unsafe_code)]`;
  - F4–F7: `Builder::system` forbidden, the diagram line preserved, gate transitions logged at
    `info`, no-D-Bus behaviour in the systemd container.
- **Round 3** (`REVISION_REQUIRED`), after the owner answers:
  - F8: a token-based scan of PAM lines that handles bracketed controls and `\` continuations;
  - F9: when `/etc/security/faillock.conf` is absent, the vendor `faillock.conf` is evaluated
    together with the built-in defaults, and the strictest wins;
  - F10, F11: wording.
- **Round 5** (revision 4, auditor items A1–A6) was approved. Two items close fail-open paths:
  - A5: PAM stack files are read through symlinks, like libpam, so authselect stacks are seen;
  - A6: the kill switch is checked again right before `UnlockSession`.
- **Observations**:
  - O1: logind is polled even when nobody is enrolled;
  - O2: DPMS, KDE, Cinnamon and MATE behaviour, and old systemd, can only be checked on
    hardware;
  - O4: the guard is deliberately stricter than PAM;
  - O5: other account modules are not consulted (accepted risk (d)).

## 4. Tester Contract

`AI/tester_contract_presence_unlock.md` has 220 tests.

| File | Tests |
|---|---|
| `crates/policy/tests/rate_limit_reserve_tests.rs` | 14 |
| `crates/daemon/tests/presence_config_tests.rs` | 16 |
| `presence_tracker_tests.rs` | 13 |
| `presence_display_tests.rs` | 13 |
| `presence_logind_mapping_tests.rs` | 9 |
| `inference_priority_tests.rs` | 10 |
| `presence_consensus_tests.rs` | 12 |
| `presence_worker_tests.rs` | 44 |
| `presence_logging_tests.rs` | 7 |
| `presence_account_tests.rs` | 54 |
| `presence_candid_review_tests.rs` | 9 |
| `tests/invariants/src/presence_unlock_contract.rs` | 19 |

The test doubles live in `crates/daemon/tests/common/mod.rs`: `MockPresenceLogind`,
`TestDisplay`, `StaticAccountGuard` / `ScriptedAccountGuard`, `SpyCamera`, `CountingExtractor`
and `test_clock!`.

- **Round 1**: 198 tests. Red state:
  - compile errors on the specified API only;
  - 14 invariant assertions failing.

  The tests were checked against a reference overlay kept in scratch space and deleted
  afterwards.
- **Round 2**: auditor findings T1–T8 added 11 tests and tightened 11 others, for 209 in total:
  - symlinked PAM stacks;
  - a guard that memoises its results;
  - a re-check bound to the wrong session ID;
  - a session swapped during the scan;
  - a hardened check that there is a single `unlock_session` call site;
  - a runtime log check of the account guard;
  - tighter bounds on hung calls.

  Each fail-open reference variant was shown to fail.
- **Round 3**: T9 added `test_pau_kill_switch_engaged_before_unlock_never_unlocks`, for 210 in
  total.
- **Round 4** (first candid review): 9 runtime tests in the new
  `crates/daemon/tests/presence_candid_review_tests.rs` and the invariant
  `presence_unlock_contract::test_pau_allow_window_uses_the_boot_clock`, for 220 in total:
  - bracketed `pam_faillock.so` arguments (`[deny=2]`, `[dir=/x y]`) detected, a bracketed
    control field before the module not treated as an argument, also through the guard;
  - shadow password fields starting with `*` or `!` (`*LK*`, `*NP*`) refused as locked;
  - suspend between `Allow` and unlock (boot clock jump), boot-clock failure, and a closed lid
    at the re-check all prevent the unlock; no false refusal without suspend or on a lid error.
- **Migrated test**: `daemon_docs_contract::test_daemon_doc_documents_every_daemon_toml_key`.
  `CONFIG_FILE_TABLES` gains `("PresenceConfigFile", "presence")`. No assertion was changed, and
  the test is now stricter.

## 5. Auditor Constraints

`AI/auditor_constraints_presence_unlock.md` lists 33 constraints. How they were met:

| Constraints | Topic | Evidence |
|---|---|---|
| 1, 4 | No panic paths, no `unsafe` | Invariants `test_pau_new_modules_never_unwrap_or_expect`, `test_pau_new_modules_forbid_unsafe_code`, workspace lints |
| 5–8 | D-Bus: pinned address, no cache, bounded connection, bounded replies | `test_pau_presence_connects_only_to_the_pinned_system_bus`, the hung-call worker tests |
| 9 | At most one account check in flight | The `account_check_in_flight` flag in `presence/worker.rs`. No behavioural test (T11); the candid reviewer checks it |
| 10–14 | Non-blocking, bounded, `O_NOFOLLOW` reads; symlinks followed for PAM stacks only; tally directory check; shared non-blocking `flock` | The `presence_account_tests` PAU22–PAU25 tests |
| 15 | Read-only, unit unchanged | C8 and C9 invariants. The unit only got a new comment saying why `CAP_DAC_OVERRIDE` is needed (A1) |
| 17–21 | Checked arithmetic, `Zeroizing` shadow buffer, nothing sensitive and no user name above `debug` in logs | `test_pau_account_guard_never_logs_its_sources`, `test_pau_presence_logs_no_biometric_field_above_debug` |
| 22, 23, 33 | One guarded unlock site, no retry, kill switch re-checked | C3, `test_pau_unlock_call_error_is_reported_as_unlock_failed`, `test_pau_kill_switch_engaged_before_unlock_never_unlocks` |
| 25–27 | Atomic reserve; no lock held across an `.await`; the dispatcher's `_interactive` guard | PAU3 and PAU12 tests |
| 30, 31 | Spawned after `READY=1`, bounded stop; kill switch first | `test_pau_main_spawns_presence_after_ready_and_stops_it_at_shutdown`, PAU4 tests |
| 32 | No test edits | Only the planned `CONFIG_FILE_TABLES` migration touched an existing test |

Audit history:

- **Round 1**: `BLOCKED` on constraints 12, 22 and 32, through T1–T5 and A5.
- **Re-audit**: `CLEARED`.
- **Review items left for the candid reviewer**:
  - T10: residual ways around the single-unlock-site greps;
  - T11: constraint 9.

## 6. Implementation

Production files:

- `crates/daemon/src/consensus.rs` and `crates/daemon/src/presence/{mod,config,logind,display,switch,tracker,account,worker}.rs` (new)
- `crates/daemon/src/dispatcher.rs`: Step 8 calls `run_face_consensus` and holds an
  `InteractiveDemandGuard`
- `crates/daemon/src/inference.rs`: priorities and demand counter; the gate is `Clone`
- `crates/daemon/src/config.rs`: `[presence]` and the rate-limit interplay warnings
- `crates/daemon/src/main.rs`: worker started after `READY=1` when `enabled && enforce_active_session`; at shutdown it is stopped, joined for at most `min(drain, 500 ms)`, then aborted
- `crates/daemon/src/pipeline.rs`: `PipelineComponents: Clone`
- `crates/daemon/src/session_policy.rs`: `non_empty` and `is_valid_session_id` become `pub(crate)`
- `crates/policy/src/{rate_limit,decision}.rs`
- `Cargo.toml` and `crates/daemon/Cargo.toml`: `zbus`
- `packaging/soos-daemon.service`: comment only

Developer documentation:

- the ADR;
- ARCHITECTURE invariants 3 and 6 and the diagram;
- `AI/MOCK_STRATEGY.md`;
- `Docs/DAEMON.md` §1.7 and §6;
- `Docs/POLICY_CRATE.md`;
- `Docs/DISTRIBUTION_DEPLOYMENT.md` §5.5;
- `Docs/PACKAGING_AND_PROVISIONING.md` §8.1;
- `Docs/SECURITY_AND_QUALITY_GUIDELINES.md` §4.

Phase 6 (this walkthrough):

- **Matrix**: new `AI/VERIFICATION_MATRIX.md` section with PAU1–PAU29. All 220 contract tests
  are cited (the round-4 tests under PAU11, PAU25, PAU26 and PAU28).
- **Hardware procedure**: new `tests/physical/screensaver_test.md` §3.6, the PAU21 procedure.
  It covers:
  - GNOME/GDM, KDE Plasma, and the `swayidle` hooks for `swaylock` / `hyprlock`;
  - the grace, lid and DPMS gates;
  - `pam_faillock`, `chage -E 0`, `passwd -l`, and a policy option set on a PAM line;
  - the kill switches, including that `gdm.disable` does not stop presence, and an unknown face;
  - a table of journal strings. Each string was checked against `crates/daemon/src`.

  The §3.1 note, the §4 behaviour matrix and the §6 rescue steps were updated to match.
- **`Docs/DISTRIBUTION_DEPLOYMENT.md`**:
  - §5.3 said "no background verification while the lock screen is displayed". It now says this
    holds for the PAM path only, and that the §5.5 daemon path scans only when logind reports
    the session locked. A plain `swaylock` never reports that.
  - §5.5 now states that it is not validated on hardware, and gives the `hyprlock` variant of
    the hook.
- **README Quick Start**: presence auto-unlock is on by default as soon as a user is enrolled,
  even before PAM is activated, and can be turned off with `presence.disable` or
  `[presence] enabled = false`. The kill-switch lines now say what `disabled`,
  `presence.disable` and `gdm.disable` each do to presence.
- **ADR**: now cites the hardware procedure and this walkthrough.
- **`.agents/skills/dev-workflow/references/project-facts.md`**: lists the new rate-limit and
  presence constants and the `presence.disable` flag.

**Developer deviations.** No developer deviation report reached Phase 6. Differences found by
reading the code against the spec:

- At step 7, an account refusal is logged only at `debug` (`reason=account_refused`). The `info`
  line `Presence unlock refused by the account guard; the session stays locked` is written only
  by the re-check made after the `Allow`. The hardware procedure tells the operator to enable
  `soos_daemon::presence=debug` to see the first case.
- A session whose `Name` is missing in the step-7 snapshot is dropped as `AccountRefused` rather
  than `NoCandidate`. Spec ambiguity 8 of the contract allows either.

**Drift check.** A grep of `Docs/`, `README.md`, `AI/ARCHITECTURE.md`, `AI/BACKLOG.md` and the
matrix found no other statement of the former 5-attempt default. The only remaining "5 attempts"
are:

- the history notes ("was `5`");
- the explicit `RateLimitConfig::new(5, …)` example in `Docs/POLICY_CRATE.md`, which configures
  a value and does not state the default;
- `PRESENCE_RESERVED_ATTEMPTS` (5).

## 7. Candid Review

Two independent reviews ran on the full diff, Phase 6 documentation included.

- **Review 1: `CHANGES_REQUESTED`.**
  - MAJOR: libpam strips `[ ]` around module arguments, so `pam_faillock.so authfail [deny=2]`
    or `[dir=...]` escaped the PAM stack scan and the guard could report a faillock-locked
    account as usable. Fixed with a tokenizer that follows libpam `_pam_mkargv` (bracketed
    tokens run to the matching `]`, spaces included, `\]` literal) and checks only the
    arguments after the `pam_faillock.so` token.
  - MINOR: the `Allow`-to-unlock window ignored suspend. It is now checked on both
    `CLOCK_MONOTONIC` and `CLOCK_BOOTTIME` (`PresenceWorker::with_boot_clock_fn` test hook), and
    the step-13 re-check reads `lid_closed()` again (`ScanOutcome::LidClosed` on `Ok(true)`).
  - MINOR: any shadow password field starting with `*` or `!` is `PasswordLocked`.
  - SUGGESTION: screen-off gating is documented as best effort (atomic KMS may keep `dpms` at
    `On`); §3.6 case 6 records the observed `display_state`.
- **Review 2: `APPROVED`** (no CRITICAL or MAJOR). T10 (single unlock site) and T11 (one account
  check in flight) were verified by reading the code. Its remaining MINOR finding was this
  walkthrough, updated here; its three suggestions are listed in §9.

## 8. Verification Results

Results on 2026-10-02, toolchain 1.98.1:

- **Presence contract targets**, each run with
  `cargo test --locked -p soos-daemon --all-features --test <target>`; all passed:

  | Target | Passed |
  |---|---|
  | `presence_config_tests` | 16 / 16 |
  | `presence_tracker_tests` | 13 / 13 |
  | `presence_display_tests` | 13 / 13 |
  | `presence_logind_mapping_tests` | 9 / 9 |
  | `inference_priority_tests` | 10 / 10 |
  | `presence_consensus_tests` | 12 / 12 |
  | `presence_worker_tests` | 44 / 44 |
  | `presence_logging_tests` | 7 / 7 |
  | `presence_account_tests` | 54 / 54 |
  | `presence_candid_review_tests` | 9 / 9 |

- `cargo test --locked -p soos-policy --all-features --test rate_limit_reserve_tests`: 14 / 14.
- `cargo test --locked -p soos-invariants --all-features presence_unlock_contract`: 19 / 19.
- `cargo test --locked -p soos-daemon -p soos-policy --all-targets --all-features --no-fail-fast`:
  678 passed and 0 failed over 80 targets, including the pre-existing daemon suites unchanged.
  An earlier fail-fast run of the same command had one failure,
  `daemon_followups_tests::test_shutdown_interrupts_accept_backoff_promptly`, a pre-existing
  wall-clock test of `crates/daemon/src/shutdown.rs`, which this branch does not touch. It then
  passed 5 / 5 runs in a row and in the full rerun.
- `cargo deny --locked check`: `advisories ok, bans ok, licenses ok, sources ok`.
- After the round-4 fixes: `cargo test --locked --workspace --all-targets --all-features`
  2568 passed, 0 failed (`--no-fail-fast`); the pre-existing timing test
  `cli_deadline_json_tests::test_simulate_pam_auth_zero_timeout_is_clamped_to_pam_minimum`
  (`soos-admin-cli`, untouched crate, 10 ms clamped deadline) failed in some full runs under
  load and passed alone 20 / 20.
- `cargo test --locked -p soos-invariants --all-features`, run after the Phase 6 edits: 396 / 396
  passed, including `matrix_citations` over the new section, `pam_deadline_contract` over
  `tests/physical/screensaver_test.md`, and the README Quick Start contract.
- `cargo fmt --all -- --check`: clean.
- `python3 scripts/sync_issue.py --check`: passes. The branch is not in `BRANCH_TO_ISSUE`, as
  expected for a GitHub-only issue.
- **Docker**: `tests/docker/systemd_unit_acceptance_test.sh` (PAU19 READY ordering and clean
  stop) was not run locally. It is unchanged and runs in the CI `systemd-unit` job.

## 9. Known Limitations / Follow-ups

- **PAU21 is pending.** Hardware validation (`tests/physical/screensaver_test.md` §3.6) has not
  been run. It must confirm, or report as defects:
  - `UnlockSession` from the sandboxed unit;
  - the ~4 s return-to-unlock delay;
  - KDE behaviour;
  - DPMS semantics on the target driver;
  - the `swayidle` hook;
  - `pam_faillock` / `chage` / `passwd -l` refusals;
  - the kill switches.
- **Accepted risks** (owner, ADR items (a)–(h)):
  - (a) default-on: camera LED on and about 15 % of one CPU core used while a session is locked
    and the screen is on;
  - (b) up to about 30 presentation attempts per minute at an unattended lock screen, and 40 /
    min on every face path including `sudo`, so PAD carries the whole defence against photos,
    screens and masks;
  - (c) locking while seated unlocks again after the grace;
  - (d) `pam_nologin`, `pam_access`, `pam_time` and `pam_tally2` are not consulted, and the
    owner can truncate its own tally;
  - (e) a lock period that restarts between two ticks skips its grace;
  - (f) the owner can drive `LockedHint`;
  - (g) unicast signals sent to the daemon's bus name are read and dropped;
  - (h) more than 1024 logind sessions disable presence for everyone (fail closed).
- **Strict by design**:
  - users without an `/etc/shadow` line (LDAP/SSSD, systemd-homed) never get presence unlock;
  - any `pam_faillock.so` policy option on a PAM line disables presence for every user;
  - a presence unlock never resets the faillock tally.
- **Follow-ups**:
  - O1: skip logind polling while no template is enrolled;
  - T11: add a behavioural test for "one account check in flight";
  - review 2 suggestions: re-read the clock right before `record_attempt_with_reserve`; make
    `DBUS_CONNECT_TIMEOUT_MS` reachable under the 500 ms `bounded()` call bound (or connect
    outside it); document that the PAM line tokenizer over-detects (Unicode whitespace, `\`
    continuation ending in `#`) and that `/etc/pam.conf` is not scanned;
  - make the `soos-admin-cli` 10 ms clamped-deadline test robust under load;
  - check Cinnamon and MATE on hardware.
