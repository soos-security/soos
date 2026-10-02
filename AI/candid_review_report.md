# Candid Review Report

- **Date**: 2026-10-02
- **Target Branch**: `feat/presence-auto-unlock`
- **Base (merge-base)**: `f76a80b`
- **Reviewed-Diff-Fingerprint**: `567a007536902c405d0cc28ce5bd3cde7383659336647b680ef47c0ce6b1b5ef`
- **Re-review**: the previous fingerprint
  `a934eeb53bc8f3a048bf9699c461bd12aada1d77b946433528d0215f43ac2d21` was APPROVED with one
  MINOR finding. A line-by-line `diff` of the two frozen patches shows that only the hunk of
  `AI/walkthroughs/175_presence_auto_unlock.md` changed (321 → 355 lines). No code, test or
  other document changed, so the earlier gate results still apply.
- **Audited Files**: `.agents/skills/dev-workflow/references/project-facts.md`, `AI/ARCHITECTURE.md`,
  `AI/DECISIONS.md`, `AI/MOCK_STRATEGY.md`, `AI/VERIFICATION_MATRIX.md`,
  `AI/architect_spec_presence_unlock.md`, `AI/auditor_constraints_presence_unlock.md`,
  `AI/tester_contract_presence_unlock.md`, `AI/walkthroughs/175_presence_auto_unlock.md`,
  `Cargo.lock`, `Cargo.toml`, `Docs/DAEMON.md`, `Docs/DISTRIBUTION_DEPLOYMENT.md`,
  `Docs/PACKAGING_AND_PROVISIONING.md`, `Docs/POLICY_CRATE.md`,
  `Docs/SECURITY_AND_QUALITY_GUIDELINES.md`, `README.md`, `crates/daemon/Cargo.toml`,
  `crates/daemon/src/{config,consensus,dispatcher,inference,lib,main,pipeline,session_policy}.rs`,
  `crates/daemon/src/presence/{account,config,display,logind,mod,switch,tracker,worker}.rs`,
  `crates/daemon/tests/common/mod.rs`, `crates/daemon/tests/inference_priority_tests.rs`,
  `crates/daemon/tests/presence_{account,candid_review,config,consensus,display,logging,logind_mapping,tracker,worker}_tests.rs`,
  `crates/policy/src/{decision,rate_limit}.rs`, `crates/policy/tests/rate_limit_reserve_tests.rs`,
  `packaging/soos-daemon.service`, `tests/invariants/src/{daemon_docs_contract,lib,presence_unlock_contract}.rs`,
  `tests/physical/screensaver_test.md` (53 files, 15,383 patch lines)

## 1. Executive Summary

I froze the target with `./scripts/candid_subagent.sh --prepare`, read the whole patch, and then
read the code around every production hunk. I did not rely on the walkthrough, the spec or the
earlier report. The account guard was compared line by line with current upstream Linux-PAM
sources (`libpam_internal/pam_line.c`, `libpam/pam_misc.c` `_pam_tokenize` / `_pam_mkargv`,
`libpam/pam_handlers.c`, `modules/pam_faillock/{pam_faillock,faillock_config,faillock}.c`).

Results:

- **Unlock call.** There is one production `UnlockSession` call
  (`presence/worker.rs:716` → `logind.rs:439`). It is reached only after all of these:
  - a consensus `Allow`;
  - a fresh `GetSession` / `GetAll` of the same ID (locked, bound, same UID, same `Name`);
  - a fresh account check;
  - a lid re-check;
  - a kill-switch re-check;
  - a shutdown check;
  - the 1000 ms window, checked on both `CLOCK_MONOTONIC` and `CLOCK_BOOTTIME`.

  There is no retry and no other route.
- **Account guard.** It is equal to libpam / `pam_faillock` or stricter on every path I tried.
  The round-4 fixes are present: bracketed arguments, `*` / `!` markers, `CLOCK_BOOTTIME`, and
  the lid re-check.
- **Consensus.** The extraction keeps the former dispatcher loop's behaviour.
- **Gates.** All four pass: fmt, clippy, tests (1,084 passed, 0 failed) and `cargo deny`.

There is no CRITICAL or MAJOR finding. The one MINOR finding (walkthrough 175 did not describe
round 4) is **resolved** in this diff; I checked it against the code and tests in §4. Three
SUGGESTIONs remain open, and the walkthrough now lists them in §9 as follow-ups.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- **Test files touched.** The only pre-existing test file touched is
  `tests/invariants/src/daemon_docs_contract.rs`. `tests/invariants/src/lib.rs` only gains a
  `mod presence_unlock_contract;` declaration. Every other test file in the patch is new:
  `crates/daemon/tests/presence_*_tests.rs`, `common/mod.rs`, `inference_priority_tests.rs`,
  `crates/policy/tests/rate_limit_reserve_tests.rs` and
  `tests/invariants/src/presence_unlock_contract.rs`.
- **Removed or changed checks** (`^-` lines with `assert`, `#[test]`, `#[tokio::test`,
  `proptest!` or `#[should_panic`): **none**.
- **New escape hatches** (`#[ignore`, `#[cfg(any())]`, `should_panic`, `tolerance`, `epsilon`):
  **none**.
- **Inline test modules.** Lines 14547 and 14554 match only because they are string fixtures
  inside `presence_unlock_contract.rs`. They are test inputs for the "strip trailing test module"
  grep. No `mod tests` was added or removed.
- **`daemon_docs_contract.rs`.** `CONFIG_FILE_TABLES` goes from `[(&str, &str); 9]` to `10` and
  gains `("PresenceConfigFile", "presence")`. This is the one allowed migration. No assertion
  changed, and the test now checks more.
- **Inline `#[cfg(test)]` modules of `dispatcher.rs`, `inference.rs`, `rate_limit.rs`,
  `decision.rs` and `config.rs`.** Untouched: no deleted test lines in their diffs.
- **The one ignored test in the run.** It existed before this branch; no `#[ignore` was added.

## 3. Deep Reasoning Audit

### Logic & Architecture

- **Path to `UnlockSession` (worker.rs:477–738).** Scenarios tried:
  - Re-check returns `None`, an error, another ID, unlocked, inactive, remote, another UID or
    another `Name` → `SessionChanged`.
  - Account refused after the `Allow` → `AccountRefused`.
  - `lid_closed() == Ok(true)` → `LidClosed`.
  - Kill switch engaged → `KillSwitchEngaged`.
  - Shutdown → `ShuttingDown`.
  - Either clock fails → `AllowExpired`.
  - Suspend between the `Allow` and the unlock (only `CLOCK_BOOTTIME` advances) → `AllowExpired`.
  - The worst case of the post-`Allow` steps (account 500 ms + logind 2 × 500 ms) is above the
    1000 ms window, so it expires (fail closed).

  All PASS.
- **Unlock failure.** `UnlockSession` failure or timeout → `UnlockFailed` plus backoff, with no
  retry.
- **Alias or indirect route.** I grepped for `UnlockSession`, `unlock_session` and
  `call_method`. `call()` takes a `&'static str` method, and its callers name only
  `ListSessions`, `GetSession`, `GetAll`, `Get` and `UnlockSession`. PASS.
- **Tracker.** I checked:
  - the grace boundary;
  - UID change → new lock period;
  - kill switch / overflow / logind failure → clear (grace restarts);
  - `unlock_requested` → no rescan, then `locker_ignored` after 5 s;
  - D5 ambiguity (two due sessions) → no scan.

  A lock period that restarts during one scan is not re-graced. That is ADR accepted risk (e).
  PASS.
- **Binding.** The same `check_local_seat_session_of` as the `Auth` path (UID, active,
  `REMOTE=0`, seat, `CLASS=user`), plus `LockedHint == Bool(true)`. Greeter, remote and
  inactive (fast-user-switched) sessions are never eligible. PASS.
- **Consensus vs `origin/main`.** I diffed the removed dispatcher loop against
  `consensus.rs:155–379`:
  - the `Interactive` branch is token-for-token identical (freshness, `can_start`,
    `decay_estimate`, `acquire_within`, the re-check after acquire, every `VisionError` mapping,
    the spoof veto, the poll sleep);
  - the clock is `self.clock_fn`, which is the same thing as `now_nanos()`;
  - wake constants 1200 / 1000 / 15 ms are unchanged;
  - aborts map to the same verdict, reason and `completion_error`;
  - `Preempted` in the interactive path fails closed (`Unavailable` / `InternalError`).

  PASS.
- **Config.** `[presence]` intervals are validated whether or not presence is enabled, and `0`
  is refused. The warning arithmetic uses `div_ceil` and a saturating budget. The defaults
  (40 attempts, 2000 ms → 30 scans per window ≤ 35) give no warning. PASS.

### PAM Concurrency & Deadlines

- **Scope.** `crates/pam` is not touched by this diff.
- **Interactive demand.** The dispatcher registers interactive demand at the start of Step 8.
  This is before the 8c reservation and before the wake. The RAII guard releases it on every
  return path.
- **Background acquisition.** `try_acquire_background` never waits, and it re-checks demand
  after acquiring. A background run is preempted before every new inference.
- **Worst-case PAM delay.** One background inference already in flight. This matches the ADR.
- **Rate-limit reserve (`rate_limit.rs:202`).** Pruning, evaluation and recording all happen in
  one `&mut self` call under the policy write lock. `reserve >= max` always refuses. Because
  `remaining_attempts` (filter count) is never above what `check_and_record` sees (deque
  length), the reserve can only be stricter. PASS.

### Panic Safety & Fail-Closed

- **No panic paths.** There is no `unwrap`, `expect`, indexing or `panic!` in the new
  production modules. `#![forbid(unsafe_code)]` is set on `presence` and `consensus`.
- **Account guard failures.** Every `Option` / `Result` failure in the guard maps to
  `Undeterminable`.
- **Worker panic.** A panic is logged once and the worker is never respawned (main.rs). A dead
  worker never unlocks.
- **Shutdown.** The worker is stopped before the drain, joined for at most
  `min(drain, 500 ms)`, then aborted. `stop_requested()` is checked before recording an attempt
  and again before the unlock. PASS.

### Test Integrity & Anti-Weakening

- **Mechanical listing.** Clean (§2).
- **Spot-checked round-4 tests** (`presence_candid_review_tests.rs`):
  - a suspend injected through the boot clock must give `AllowExpired`;
  - a lid closed from the extractor hook must give `LidClosed`;
  - bracketed `[deny=2]` / `[dir=/x y]` are detected while a bracketed control field is not.

  Each would fail against the pre-round-4 code.
- **Contract migration.** The only one is the `CONFIG_FILE_TABLES` entry. PASS.

### Memory, Bounds & Secrets

- **Account guard vs libpam.**
  - **Tokenizer** (`account.rs:470`):
    - same `[`-at-token-start rule as `_pam_tokenize`;
    - same "first `]` ends it" rule;
    - same `\]` → `]` rule;
    - same unclosed-bracket-to-end rule;
    - `"[a b]c"` → two tokens, as in libpam.

    The tokenizer splits on Unicode whitespace, which is a superset of libpam's `" \n\t"`.
    Continuation lines that end in a comment are joined, while libpam ends the logical line at
    any `#`. Both differences only over-detect, so they fail closed.
  - **Blank line inside a continuation.** It ends the logical line in both implementations
    (`_pam_line_buffer_add_eol` / `buffer_valid`).
  - **Argument matching.** It matches `set_conf_opt` (split at the first `=`).
    `even_deny_root=x` is not detected, but it only matters for admins, and the guard already
    treats admins as lockable. Option and module names are case-sensitive, as upstream.
- **Faillock.**
  - `check_tally`: the same `latest`, the same `<` / `>=` comparisons, the same
    `latest + unlock_time < now`.
  - `struct tally`: `status` is at byte 54 and `time` at 56, as upstream.
  - The admin case is evaluated as both admin and non-admin. That is stricter than upstream,
    which never locks an admin without `even_deny_root`.
  - `faillock.conf`: same grammar. An unknown key or a bad value is `Undeterminable`, where
    upstream logs it and ignores it, so the guard is stricter.
  - Vendor file: evaluated together with the defaults. That is stricter than upstream, which
    evaluates the vendor file alone.
  - `pam_faillock` takes the same `flock(LOCK_EX)`, so the shared non-blocking `flock` is
    consistent with it.
  - An unreadable tally (`EACCES`) is `Undeterminable`. Upstream treats it as success, so the
    guard is stricter.
- **Shadow.** Same rules as `pam_unix` `check_shadow_expiry`:
  - expire `>=`, including 0;
  - `lastchg == 0` → `PasswordChangeForced`;
  - the inactive and max ordering.

  The `!` / `*` markers are an extra, stricter refusal. Nine fields are required, and `-1` or
  non-digits are `Undeterminable`.
- **File handling.**
  - `O_NONBLOCK | O_CLOEXEC` everywhere; the tally adds `O_NOFOLLOW`;
  - type decided by `fstat` on the open descriptor;
  - a single allocation of at most `limit + 1` bytes, `Zeroizing`, never reallocated;
  - a file that grew after `fstat` → refused;
  - the user name cannot hold `/` and cannot start with `.` or `-`;
  - tally directory: `symlink_metadata`, and not group- or other-writable unless it is
    root-owned and sticky.
- **One check in flight.** `swap(true)` with an RAII release inside the blocking job. A check
  that times out keeps the flag set until its thread ends, so checks never pile up.
- **Bounds.** Sessions (1024 / 16), tracker (16), DRM entries (64), PAM directory entries
  (512 × 64 KiB), shadow (4 MiB / 65,536 lines), and D-Bus error text (256 bytes).
- **Logs.** UID and session ID only above `debug`, never the user name. No frame, embedding,
  file content or D-Bus body is logged. A spoof veto is logged and never sealed as evidence.
  PASS.

### Supply Chain & Automation

- **zbus.** `zbus 5` with `default-features = false` and `tokio` adds 22 crates (MIT or
  Apache-2.0). I checked `zcheapstr`: its author is the zbus maintainer (`z-galaxy`
  repository), and the crate comes from crates.io with a checksum. `uds_windows` is built only
  for Windows targets.
- **cargo deny.** `advisories ok, bans ok, licenses ok, sources ok`.
- **Pinned bus address.** No `Builder::system` and no environment variable.
- **Unit file.** Only a comment changed. `AF_UNIX` and `PrivateNetwork` keep the filesystem
  bus socket reachable.
- **`.github/` and `scripts/`.** Not changed. PASS.

### English-Only Policy

All added code, comments and documents are in English. The only non-ASCII characters are
Unicode test fixtures (`"é"`, `"ünïcode"`) used to test truncation and validation. PASS.

### Documentation accuracy

These documents match the code, including the round-4 fixes:

- ADR 2026-10-02 (clauses 5 and 10);
- `Docs/DAEMON.md` §1.7 and §6;
- `Docs/DISTRIBUTION_DEPLOYMENT.md` §5.3 and §5.5;
- the README Quick Start;
- `tests/physical/screensaver_test.md` §3.6 (lid re-check, `LidClosed`);
- `VERIFICATION_MATRIX` rows PAU11, PAU25, PAU26 and PAU28.

Walkthrough 175 now matches too (FINDING 1 resolved, see §4).

## 4. Detailed Findings & Action Items

- **[MINOR — RESOLVED]** `AI/walkthroughs/175_presence_auto_unlock.md` — **The walkthrough did
  not describe round 4.**
  - **Resolution, checked against the code and tests.**
    - The counts now say 220 tests.
    - The §4 table lists `presence_candid_review_tests.rs` with 9 tests. That file has 9
      `#[test]` / `#[tokio::test]` functions.
    - `presence_unlock_contract` is listed with 19 tests. It has 19, including
      `test_pau_allow_window_uses_the_boot_clock` at line 1058.
    - A Round 4 entry is added.
    - §7 now records both reviews and the four fixes, and each description matches
      `account.rs`, `worker.rs` and the tests.
    - §8 is updated.
    - The `soos-admin-cli` timing test it cites exists:
      `crates/admin-cli/tests/cli_deadline_json_tests.rs:98`.
    - §9 lists the three suggestions below.

  Original finding:
  - **Defect.**
    - It still says the contract has **210 tests**. `AI/tester_contract_presence_unlock.md:474`
      says 220.
    - Its §4 table does not list `crates/daemon/tests/presence_candid_review_tests.rs`.
    - §7 says "Candid Review — Not run yet". In fact a `CHANGES_REQUESTED` review happened and
      its fixes were applied:
      - bracketed `pam_faillock.so` arguments are detected;
      - any `!` / `*` shadow marker is locked;
      - the `Allow` window is also measured on `CLOCK_BOOTTIME`;
      - the lid is re-checked after the `Allow` (`ScanOutcome::LidClosed`).
    - None of these four fixes appears anywhere in the walkthrough. §8 shows the earlier test
      counts.
  - **Correction.**
    - Add a "Round 4 (candid review)" entry in §4 and §7 listing the four fixes and the new
      test file (9 runtime tests and 1 invariant).
    - Update the counts to 220.
    - Refresh the §8 numbers.
- **[SUGGESTION]** `crates/daemon/src/presence/worker.rs:565-570` — **Stale attempt time.**
  - The attempt is recorded with `now_ns` read at step 3. Steps 4–9 (snapshot, account checks,
    lid call) can take more than 1.5 s, so the timestamp is older than the real attempt and can
    land behind a newer PAM entry in the per-UID deque.
  - This only shifts the window by that delay, and it is never fail-open.
  - Read `(self.clock_fn)()` again just before `record_attempt_with_reserve`. On error, skip
    with `ClockUnavailable`.
- **[SUGGESTION]** `crates/daemon/src/presence/logind.rs:246-267` — **Connect timeout cannot be
  reached.**
  - `DBUS_CONNECT_TIMEOUT_MS = 1000` is never reached, because every worker call is wrapped in
    `bounded()` at 500 ms (worker.rs:189).
  - A bus whose handshake takes 500–1000 ms never connects, and the request is refused.
  - Either lower the connect timeout below the call bound, or connect outside `bounded()`.
    Also document that the 500 ms bound covers the whole snapshot (`ListSessions` plus up to 16
    `GetAll`), not each call.
- **[SUGGESTION]** `crates/daemon/src/presence/account.rs:446-503` — **Document the tokenizer
  differences.**
  - Note in the doc comment that it deliberately over-detects compared with libpam: Unicode
    whitespace, and a `\` continuation line that ends in `#` is still joined.
  - Note that `/etc/pam.conf` is not scanned. libpam reads it only when `/etc/pam.d` is absent.
    Optionally refuse (`Undeterminable`) when `/etc/pam.conf` exists and `/etc/pam.d` does not.

## 5. Final Verdict

There is no CRITICAL or MAJOR finding. Gates run on this tree:

- `cargo fmt --all -- --check`: rc=0.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`: rc=0.
- `cargo test --locked -p soos-daemon -p soos-policy -p soos-invariants --all-targets --all-features`:
  rc=0, 1,084 passed, 0 failed.
- `cargo deny --locked check`: rc=0.

**VERDICT: APPROVED**
