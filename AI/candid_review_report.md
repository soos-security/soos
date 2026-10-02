# Candid Review Report

- **Date**: 2026-10-02
- **Target Branch**: `fix/presence-review-followups`
- **Base (merge-base)**: `35a708a`
- **Reviewed-Diff-Fingerprint**: `bfdc896454cae33a647afd2f6b6292aba649eb08894227751d6536a8c6d422f4`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_presence_followups.md`, `AI/auditor_constraints_presence_followups.md`, `AI/tester_contract_presence_followups.md`, `AI/walkthroughs/176_presence_review_followups.md`, `Docs/BIOMETRIC_STORE_CRATE.md`, `Docs/DAEMON.md`, `crates/admin-cli/tests/cli_deadline_json_tests.rs`, `crates/biometric-store/src/store.rs`, `crates/biometric-store/tests/enrollment_probe_tests.rs`, `crates/daemon/src/presence/account.rs`, `crates/daemon/src/presence/logind.rs`, `crates/daemon/src/presence/mod.rs`, `crates/daemon/src/presence/worker.rs`, `crates/daemon/tests/presence_account_tests.rs`, `crates/daemon/tests/presence_candid_review_tests.rs`, `crates/daemon/tests/presence_followups_connect_tests.rs`, `crates/daemon/tests/presence_followups_pam_conf_tests.rs`, `crates/daemon/tests/presence_followups_tests.rs`, `crates/daemon/tests/presence_logging_tests.rs`, `crates/daemon/tests/presence_worker_tests.rs`, `tests/invariants/src/lib.rs`, `tests/invariants/src/presence_followups_contract.rs`

## 1. Executive Summary

GitHub #325 follow-ups to the presence auto-unlock worker (PR #324). Reviewed from the raw
patch (3741 lines) and the surrounding code, not from the author documents. The changes are a
fresh attempt stamp taken under the policy write lock, an explicit `connect()` step bounded
outside the call bound, a bounded, non-decrypting enrollment probe that runs before any D-Bus
traffic, an `/etc/pam.conf` rule, and a substring-superset PAM line scan with an include-of-a-path
rule. Every change moves toward fail-closed. No new route reaches `UnlockSession`. The
pre-existing test edits are setup-only, except the one owner-approved assertion migration, and
that migration matches the owner decision exactly. The local gates are green: fmt, clippy
`-D warnings`, tests for the four crates (1316 tests ok, 0 failed) and `cargo deny`. There are
no CRITICAL or MAJOR findings, one MINOR and two SUGGESTION.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

Step 3 greps on `target/candid_diff.patch`:
- Removed or changed assertion lines (`^-` with assert, `#[test]`, `should_panic`): **none**.
- New escape hatches (`#[ignore]`, `cfg(any())`, `should_panic`, tolerance, epsilon): **none**.
- Inline `mod tests` changes: **none**.

Pre-existing test files touched, checked one by one:
1. `crates/daemon/tests/presence_account_tests.rs:161`: adds `.with_pam_conf(self.path("etc/pam.conf"))` to the guard builder. Setup only, so the host `/etc/pam.conf` is never read. No assertion touched.
2. `crates/daemon/tests/presence_candid_review_tests.rs:112`: adds `.with_pam_conf(root.join("pam.conf"))`. Setup only.
3. `crates/daemon/tests/presence_logging_tests.rs:383`: adds `.with_pam_conf(root.join("pam.conf"))`. Setup only.
4. `crates/daemon/tests/presence_worker_tests.rs:460-464`: in the "not enrolled" case, UID 1001 (not the session owner `UID`) is now enrolled. The expected `SkipReason::NotEnrolled`, the `NoCandidate` alternative and `assert_nothing_spent` are unchanged. Justification: with an empty store the new probe would short-circuit before `select`. With another UID enrolled, the case still exercises the original path, where the session owner has no template (`select`, `Ok(None)`). The empty-store path is covered by the new `test_pfu_no_logind_traffic_while_nobody_is_enrolled`. Setup only.
5. `crates/admin-cli/tests/cli_deadline_json_tests.rs`: the owner-approved migration (owner decision 2026-10-02, PFU7). Checked against the decision:
   - Only `test_simulate_pam_auth_zero_timeout_is_clamped_to_pam_minimum` (line 128) uses `Completion::TimeoutTolerated`. The 250 ms test and the `u64::MAX` test still use `capture_deadline`, which maps to `Completion::Required` and therefore `Ok` only.
   - The match (lines 103-106) accepts `Ok(_)` always, and `Err(AdminCliError::Timeout)` only under `TimeoutTolerated`. Any other error, in any mode, hits `panic!`.
   - The `deadline >= before + budget && deadline <= after + budget` assertion is byte-identical and always runs. The deadline comes from `rx.recv_timeout(CAPTURE_WAIT = 5 s)` with `.expect`, so a missing capture fails the test. It is never skipped.
   - The server write-failure tolerance (`written.expect` only under `Required`) is limited to the tolerated mode. In `Required` mode, a write failure still panics the server thread, and `server.join().expect` fails the test.
   - `server.join()` now runs after the receive, which removes no check.
   - Conclusion: the implementation matches the owner decision exactly. No other test was weakened.
6. `tests/invariants/src/lib.rs`: only registers the new `presence_followups_contract` module.

New test files: `enrollment_probe_tests.rs`, `presence_followups_tests.rs`, `presence_followups_connect_tests.rs`, `presence_followups_pam_conf_tests.rs` and `presence_followups_contract.rs`. Sampled tests can fail against plausible wrong implementations:
- the stale stamp is caught by the 39-remaining-attempts probe at `before + 60.5 s`;
- the reserve test fails if the step-3 clock is used;
- the clock regression and clock failure tests assert `ClockUnavailable` and `assert_nothing_spent`;
- the lock-ordering test parks a policy writer;
- the probe bound test checks 4096 entries (`false`) against 4097 (error);
- the no-template test checks `seat_calls() == 0`, `lid_calls() == 0` and `state_calls() == 0`.

## 3. Deep Reasoning Audit

### Logic & Architecture

- **(a) Single `UnlockSession` route.** `unlock_session` is still called once, at `worker.rs:759`, after the unchanged re-check chain. The new early returns are:
  - the probe `Ok(false)` and `Err` (lines 500-510);
  - the connect failure (511-525);
  - the clock error or regression at step 10 (600-606);
  - the rate limiter.

  All of them return `Skipped` before any camera, inference or unlock work. `current()` (`logind.rs:260`) never opens a connection. After a mid-tick `reset()`, the re-check and the unlock get `BusUnavailable`, which ends as `SessionChanged` or the unlock failure path. Fail closed. **PASS.**
- **(b) Worker.**
  - The probe runs after the kill switch, the backoff and the clock, and before step 4a. No logind access precedes it in any tick.
  - `Err` maps to `TemplateStoreError` and `Ok(false)` to `NotEnrolled`. Both clear the tracker.
  - The connect step is wrapped in `tokio::time::timeout(DBUS_CONNECT_TIMEOUT_MS)` (worker side) and again around `builder.build()` inside `ZbusLogind::connect`. The outer bound also covers waiting for the slot mutex.
  - A connect `Err` or a timeout runs `logind_failed` (backoff and a single warn), clears the tracker and returns `LogindUnavailable`, all before any snapshot.
  - Step 10 acquires `policy.write().await`, then reads the clock synchronously. `Ok(ns) if ns >= now_ns` is required. A regression or an error drops the guard and returns `ClockUnavailable` before `record_attempt_with_reserve`.
  - There is no `.await` while the guard is held: the guard is dropped explicitly before `skip`, `mark_scan_started` and `scan`.
  - `attempt_ns` is used for both the recording and `mark_scan_started`. **PASS.**
- **(c) `account.rs` superset.** Old rule: tokenize with `_pam_mkargv` semantics, find the first token *ending* with `pam_faillock.so`, then any later token (after trimming `[`) equal to `even_deny_root` or starting with a policy prefix. Break attempts against the new rule:
  - *Bracket unescape.* `\]` becomes `]` in the old tokenizer. None of the prefixes or the flag contain `]` or `\`, so unescaping can never create a match that is absent from the raw text.
  - *Bracket stripping and leading `[` trim.* The raw text still holds the option text, and `starts_with` implies `contains`.
  - *First occurrence vs module token.* `pam_faillock.so` cannot overlap itself (no proper prefix equals a suffix), so the first occurrence ends at or before the start of the module token's occurrence. The text after it therefore contains every later token. A first occurrence inside a non-module token (`xpam_faillock.sox`) only widens the searched text.
  - *Whitespace.* The substring search is independent of the separators.

  No line detected by the old rule escapes the new one. The include rule uses one iterator, so `any(directive) && any('/')` finds a `/`-token strictly after the directive token, in any later position. `eq_ignore_ascii_case` covers `INCLUDE`, `SubStack` and `@Include`.

  The `pam.conf` rule:
  - `any_pam_dir` uses `metadata`, which follows symlinks like libpam's `stat`. NotFound or a non-directory does not count; other errors are `Undeterminable`.
  - `pam.conf` is detected with `symlink_metadata`, so a dangling symlink counts as present.
  - If `pam.conf` is present and no PAM directory is a directory, the result is `Undeterminable`.
  - If both are present, `pam.conf` is scanned. `Absent` (a race or dangling link), a directory, a non-regular file, an unreadable file, more than `MAX_PAM_FILE_BYTES` or non-UTF-8 all give `Undeterminable`.
  - An `any_pam_dir` error is `Undeterminable` even when `pam.conf` is absent. That is conservative.
  - A system where `/usr/lib/pam.d` exists but libpam lacks that vendordir still has `pam.conf` scanned with the superset rule.

  **PASS.**
- **(d) `has_enrolled_template`.**
  - `enumerate()` counts every entry, error entries and non-UTF-8 names included. The bound check precedes `entry?`, so exactly 4096 entries are examined and the 4097th returns `InvalidPath`.
  - The error message is static: no path, no name.
  - `DirEntry::file_type()` does not follow symlinks; `is_file()` is required.
  - The canonical name check is `parse::<u32>()` plus a `to_string()` round trip. It rejects `+1`, `01` and overflow while accepting `0`.
  - `TEMPLATE_EXTENSION` is `.cbor.enc`, so temporary names never match.
  - Nothing is opened or decrypted, and the store lock is not taken.
  - An I/O failure maps to `Io` through `#[from]`, a missing directory included, never `false`.

  **PASS.**

### PAM Concurrency & Deadlines
`crates/pam` is untouched. The daemon-side D-Bus steps stay bounded: connect by 1000 ms on both sides, every other step by `bounded()` at 500 ms plus the zbus `method_timeout`. The probe is synchronous, bounded at 4096 directory entries and makes no network call. **PASS.**

### Panic Safety & Fail-Closed
The production hunks contain no `unwrap`, `expect`, indexing or `panic!`. Every new error path skips or refuses. None of them yields an unlock or an attempt-free scan. **PASS.**

### Test Integrity & Anti-Weakening
See §2. The only assertion-level migration is the owner-approved PFU7 change, and it is implemented exactly as decided. **PASS.**

### Memory, Bounds & Secrets
- **(e) Logs and secrets.**
  - Probe and connect errors are discarded (`Err(_)`) or logged only through the existing `logind_failed` warn (error kind only).
  - No file content, path content, template or user data is logged.
  - The PAM scan works on content bounded by `MAX_PAM_FILE_BYTES`, and its logical line is bounded by the same limit.
  - The probe holds no template bytes.

  **PASS.**

### Supply Chain & Automation
No `Cargo.*`, `deny.toml`, `.github/` or `scripts/` change. `cargo deny --locked check` reports advisories, bans, licenses and sources ok. **PASS.**

### English-Only Policy
- **(f) Docs and English.** Code, comments and docs are in English: the accented-character grep found no non-English content in the added lines. The docs match the code:
  - `Docs/DAEMON.md` §6 steps 2-4 and 9, and the `pam.conf` and over-detection bullets;
  - `Docs/BIOMETRIC_STORE_CRATE.md` §3.6 (bound semantics, the `InvalidPath` and `Io` mapping, no lock);
  - the ADR amendment items (i) to (vi);
  - matrix rows PFU1-PFU7;
  - walkthrough 176, including an honest record of the residual PFU7 flake.

  **PASS.**

### Verification Commands (run by the reviewer)
- `cargo fmt --all -- --check`: exit 0.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --locked -p soos-daemon -p soos-biometric-store -p soos-admin-cli -p soos-invariants --all-targets --all-features`: exit 0, 0 failed.
- `cargo deny --locked check`: exit 0.

## 4. Detailed Findings & Action Items

- **[MINOR]** `crates/admin-cli/tests/cli_deadline_json_tests.rs:107-109`. A residual load-only flake is documented in walkthrough 176 §6 (2/576 runs under 64-way single-CPU pinning). Under that stress, the client misses its 10 ms deadline before it writes the request, so no deadline is captured and `recv_timeout(...).expect` fails. This is the strict reading of the owner rule ("fail if the captured value never arrives"), so it is not a defect in the migration. Required correction: none for this PR. If the flake shows up in CI, it needs a new owner decision; the tests must not be relaxed unilaterally.
- **[SUGGESTION]** `crates/daemon/src/presence/worker.rs:500-510`. A transient probe error (or `Ok(false)`) clears the tracker, including any "locker ignored UnlockSession" marker. On the next enrolled tick, that lock period is therefore observed afresh and can be scanned again after the grace period. A logind outage already behaves this way, the shared rate limiter still bounds attempts, and this direction is fail-closed for unlocking. An optional follow-up is a one-line note in `Docs/DAEMON.md` §6 that a store-probe skip, like a logind outage, restarts every lock period.
- **[SUGGESTION]** `crates/daemon/src/presence/logind.rs:358-380`. `ZbusLogind::connect` holds the slot mutex across `builder.build()`. This is harmless today, because the worker is the only caller and is sequential. A comment stating that single-caller assumption would keep a future concurrent caller from inheriting a connect-long lock wait.

## 5. Final Verdict

No CRITICAL or MAJOR finding after concrete break attempts on every pillar.

**VERDICT: APPROVED**
