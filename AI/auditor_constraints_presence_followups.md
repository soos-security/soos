# Auditor Constraints — GitHub #325 (presence auto-unlock review follow-ups of #324)

- **Branch**: `fix/presence-review-followups` (base `origin/main` `35a708a`)
- **Inputs**: `AI/architect_spec_presence_followups.md` rev 3 (PFU1–PFU7), `AI/plan_evaluator_report.md`
  (round 3, APPROVED), `AI/tester_contract_presence_followups.md`, the new test files, the
  migrated tests, and the current code of `crates/daemon/src/presence/{worker,logind,account,mod}.rs`,
  `crates/daemon/src/main.rs` and `crates/biometric-store/src/store.rs`.
- **Date**: 2026-10-02

## 1. Audit Checklist Results

| # | Item | Result |
|---|---|---|
| 1 | Panic paths | `grep -nE '\.unwrap\(\|\.expect\(\|panic!\|todo!\|unimplemented!\|unreachable!'` on `crates/daemon/src/presence/*.rs` and `crates/biometric-store/src/store.rs` finds hits only in the `#[cfg(test)]` module of `store.rs` (line 1100 and later). Production is clean. PAM is not touched. |
| 2 | Unsafe | `presence/mod.rs` has `#![forbid(unsafe_code)]` and `biometric-store/src/lib.rs` has `#![forbid(unsafe_code)]`. The planned change needs no `unsafe`: `DirEntry::file_type` and `std::fs::metadata` / `symlink_metadata` are safe. |
| 3 | Output isolation | No `println!`, `eprintln!` or `dbg!` in the affected sources. No PAM change. |
| 4 | Bounded I/O and deadlines | Connect: two bounds of 1000 ms (worker and `ZbusLogind`). Snapshot and calls: 500 ms. Store probe: at most 4096 entries. `pam.conf`: `MAX_PAM_FILE_BYTES`. PAM directories: `MAX_PAM_DIR_ENTRIES` (unchanged). The worst tick gains at most 1000 ms, and ticks run one after another. |
| 5 | Arithmetic | The probe counter must use `checked_add`, or `enumerate` compared with the bound. The clock comparison `attempt_ns >= now_ns` cannot overflow. |
| 6 | Filesystem safety | Every new access is read-only. The probe must not follow symlinks (it uses the entry type, `lstat` semantics). `pam.conf` is opened through `read_source` (`O_NONBLOCK`, regular files only, bounded). The `presence_unlock_contract::test_pau_presence_code_never_writes` invariant keeps applying to `presence/`. |
| 7 | Secrets and privacy | The new code reads no secret. The probe returns a `bool` and never opens a template. File content of `pam.conf` is never logged. The new skip paths log only the stable `SkipReason` codes, at debug level and on change. |
| 8 | Fail-closed | Every new error path ends in a skip or an `Undeterminable` refusal (spec §4). The analysis is in §2. |
| 9 | Supply chain | There is no new dependency, so `cargo deny` is unaffected. |
| 10 | CI and workflow | `.github/`, `scripts/` and `.githooks/` are not touched. |

## 2. Fail-Open Analysis of the Planned Production Changes

- **Store probe (item 5).** The probe is only a gate in front of the per-UID path. A probe
  `false` while a template exists makes presence skip, so the user is never unlocked by
  mistake. A probe `true` while nothing usable exists falls through to the unchanged step-7
  `get()`, which decrypts and checks the UID. A probe error has to fail closed, as
  `Skipped(TemplateStoreError)` with no D-Bus call. It must never be mapped to `Ok(true)` or
  `Ok(false)`. In short, no probe result can cause an unlock that the old code would have refused.
- **Connect outside `bounded()` (item 2).** The connect step is still bounded, by
  `DBUS_CONNECT_TIMEOUT_MS` in the worker and again inside `ZbusLogind::connect`. If a
  connection is reset in the middle of a tick, the next call returns `BusUnavailable`, which maps as follows:
  - snapshot: ends the tick;
  - re-check: `SessionChanged`;
  - lid: does not gate (as today);
  - unlock: `UnlockFailed`.

  None of these unlocks.
- **Line-scan superset (item 3).** I checked by hand that the substring rule detects
  everything the current token rule detects:
  - A module token that ends with `pam_faillock.so` puts the substring at or before the
    token's end, and every argument token comes after it.
  - Bracket stripping and `\]` only change `]`, which no policy prefix contains.
  - Joining extra lines only adds text, and it can only move the first occurrence earlier.

  The include rule adds detections and removes none. It under-detects only if it is written
  as "the next token" instead of "any later token" (see A14), or if it looks after the
  *last* occurrence of the module name instead of the first (see A13).
- **`/etc/pam.conf`.** Every outcome other than "absent" or "scanned with no option found" is
  `Undeterminable`. When no PAM directory exists, any `pam.conf` (even a comment-only one) is
  `Undeterminable` in every case.
- **Fresh stamp (item 1).** A later stamp keeps the attempt in the window longer, which is
  conservative for presence. The reserve is checked at the actual recording time, which is the purpose of the fix. A failed or regressed
  clock skips the tick before the attempt, the camera wake, the inference and the unlock.

## 3. Migration Review (`git diff origin/main -- crates/daemon/tests crates/admin-cli/tests tests/invariants`)

| File | Change | Verdict |
|---|---|---|
| `presence_account_tests.rs` (`Accounts::guard`) | `+ .with_pam_conf(self.path("etc/pam.conf"))` | Setup only (the file is absent, so the host `/etc` is never read). No assertion touched. |
| `presence_candid_review_tests.rs` (`guard_tree`) | `+ .with_pam_conf(root.join("pam.conf"))` | Setup only. |
| `presence_logging_tests.rs` (`test_pau_account_guard_never_logs_its_sources`) | `+ .with_pam_conf(root.join("pam.conf"))` | Setup only. |
| `presence_worker_tests.rs` (`test_pau_unusable_templates_cost_no_attempt_and_no_camera`, "not enrolled") | `vec![]` → `vec![(1001, Enrollment::LiveIdentity)]` | Setup only. The expected `SkipReason::NotEnrolled`, the `tick_past_grace` assertion and `assert_nothing_spent` are unchanged. UID 1001 has no session, so UID 1000 still goes through the per-UID not-enrolled path the case was written for. The empty-store path is covered by the new PFU6 tests. |
| `tests/invariants/src/lib.rs` | registers `presence_followups_contract` | Additive. |
| `cli_deadline_json_tests.rs` (PFU7) | the clamp test accepts `Ok` or `Err(AdminCliError::Timeout)` only; any other `Err` panics | **Owner-approved assertion migration (option a), matches the decision exactly.** The deadline assertion is unchanged and always runs: the server captures the deadline before writing, and `recv_timeout(5 s)` fails the test if no deadline arrives. The 250 ms and oversized tests still use `Completion::Required`, which keeps the response-write `expect`. Replacing `rx.recv()` after `join` with `recv_timeout` before `join` keeps the same checks and adds a time bound. No other weakening. |

The migrations, the PFU7 one included, weaken no contract.

## 4. Test Power Review (fail-open mutants)

| Fail-open variant | Killed by |
|---|---|
| Clock read before step 7 (step-3 stamp) | `test_pfu_attempt_is_stamped_with_a_fresh_clock_read`, `test_pfu_reserve_is_evaluated_at_the_fresh_timestamp` |
| Clock read before the policy write lock | `test_pfu_attempt_is_stamped_after_the_policy_lock_is_acquired` (M2) |
| Clock error at the re-read ignored or mapped to a scan | `test_pfu_clock_failure_before_the_attempt_skips_without_attempt_or_scan` |
| Clock regression accepted | `test_pfu_clock_regression_before_the_attempt_skips_the_tick` (M5) |
| `connect()` under `bounded()` (500 ms) | `test_pfu_connect_slower_than_the_call_bound_is_still_awaited` (M3) + invariant |
| `connect()` unbounded | `test_pfu_hung_connect_is_bounded_by_the_connect_timeout` |
| Connect error ignored or snapshot attempted | `test_pfu_connect_failure_skips_before_the_snapshot` (`seat_calls == 0`, all error kinds) |
| A reconnect hidden inside `call()` | `presence_followups_contract::test_pfu_only_connect_opens_a_bus_connection` |
| `pam.conf`-only system accepted | `test_pfu_pam_conf_without_any_pam_directory_is_undeterminable` (M9) |
| `pam.conf` next to `pam.d` not scanned | `test_pfu_pam_conf_next_to_pam_d_is_scanned_for_faillock_options` |
| `pam.conf` checked with `metadata` (dangling symlink treated as absent) | `test_pfu_non_directory_pam_paths_do_not_hide_pam_conf`, `test_pfu_unscannable_pam_conf_is_undeterminable` |
| FIFO, directory, oversized or non-UTF-8 `pam.conf` accepted | `test_pfu_unscannable_pam_conf_is_undeterminable` |
| Token-scan under-detection (F1 counter-examples) | `test_pfu_pam_scan_unterminated_bracket_after_unicode_space_is_detected`, `test_pfu_pam_scan_line_after_a_commented_continuation_is_detected` |
| Include of a path not refused, or matched case-sensitively | `test_pfu_pam_scan_include_of_a_path_is_detected` |
| Scan after the **last** `pam_faillock.so` instead of the first | **NOT KILLED**: no test line has two occurrences. Gap, see A13. |
| Include target checked only in the **next** token instead of any later token | **NOT KILLED**: every positive case has the `/` in the token right after `include`. Gap, see A14. |
| Probe follows symlinks, accepts non-canonical names or has an off-by-one bound | `enrollment_probe_tests` (M6, M7, M8) |
| Probe error treated as enrolled or as empty | `test_pfu_store_listing_error_skips_before_logind` |
| Second account check concurrent | `test_pfu_second_account_check_never_runs_while_one_is_in_flight` (M1) |

These gaps are not fail-open, so no added test is required for them:
- `mark_scan_started` is not tested with `attempt_ns`. A wrong value only shifts the
  scan interval by about one tick.
- A `symlink_metadata(pam_conf)` error other than `NotFound`, treated as absent, is not tested. As root
  this can only be `EIO`, `ELOOP` or `ENOTDIR`. A tester could cover it with
  `<regular file>/pam.conf` (`ENOTDIR`). That test is optional, and A9 requires the behaviour anyway.

## Audit Constraints — Issue #325

| # | Constraint | Applies to (file::fn) | Verified by (test / lint / invariant / grep) |
|---|---|---|---|
| A1 | No `unwrap`, `expect`, `panic!`, `unreachable!`, slice indexing or unchecked arithmetic in the new production code. | `presence/{worker,logind,account,mod}.rs`, `biometric-store/src/store.rs` | `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`; `presence_unlock_contract::test_pau_new_modules_never_unwrap_or_expect` |
| A2 | Step 10: the clock is read **after** `policy.write().await` returns and **before** `record_attempt_with_reserve`. There is no `.await` between taking the guard and dropping it. `Err(_)` or `attempt_ns < now_ns` drops the guard and returns `Skipped(ClockUnavailable)` before any attempt, camera wake, inference or unlock. `attempt_ns == now_ns` is accepted. The same `attempt_ns` is passed to `mark_scan_started`. | `worker.rs::tick` step 10 | PFU1 tests (5); code review |
| A3 | The worker awaits `self.logind.connect()` once per tick, only after the kill-switch check, the backoff check, the step-3 clock and the store probe, and directly before the snapshot. It is wrapped in `tokio::time::timeout(Duration::from_millis(DBUS_CONNECT_TIMEOUT_MS), …)` and never in `bounded()`. On an error or timeout the worker calls `logind_failed(&err)` (backoff, warn on transition), clears the tracker and returns `Skipped(LogindUnavailable)`, never `TooManySessions`. A successful connect alone must not reset the backoff or `logind_down`: only a successful snapshot calls `logind_succeeded()`. | `worker.rs::tick` step 4a | `presence_followups_connect_tests` (5); `presence_followups_contract::test_pfu_worker_bounds_connect_outside_the_call_bound` |
| A4 | `ZbusLogind::connect` is the only place that builds a connection. It uses `zbus::connection::Builder::address(self.address())`, the pinned `SYSTEM_BUS_ADDRESS`, and never `Builder::system`, `Connection::system` or the environment. It keeps `method_timeout(DBUS_CALL_TIMEOUT_MS)`, `max_queued(MAX_QUEUED_MESSAGES)` and its own `tokio::time::timeout(DBUS_CONNECT_TIMEOUT_MS)` around `build()`. Build error maps to `BusUnavailable`, expiry to `Timeout`, and a held slot to `Ok(())` with no I/O. It registers no object server, name or match. The former `connection()` becomes a pure accessor that returns `Err(BusUnavailable)` on an empty slot and never builds. A transport failure still calls `reset()`. | `logind.rs::ZbusLogind::{connect, current/connection, call}` | `test_pfu_only_connect_opens_a_bus_connection`; `presence_unlock_contract::test_pau_presence_connects_only_to_the_pinned_system_bus` |
| A5 | The default trait method `PresenceLogind::connect` returns `std::future::ready(Ok(()))` and performs no I/O. `MockPresenceLogind` stays unchanged. Update the stale `ZbusLogind` doc ("connection opened on first use") so that it names `connect()`. | `logind.rs::PresenceLogind` | compile + existing presence tests |
| A6 | `BiometricStore::has_enrolled_template` lists the directory with `std::fs::read_dir(&self.base_dir)`. It never opens, reads, decrypts or locks an entry: no `lock_store()`, whose 5 s `STORE_LOCK_TIMEOUT` would block the async worker. It classifies each entry with `DirEntry::file_type()`, which does not follow symlinks, and returns `true` only for `is_file()` entries whose name is valid UTF-8, ends with `TEMPLATE_EXTENSION` and has a stem equal to `stem.parse::<u32>()?.to_string()`. | `biometric-store/src/store.rs::has_enrolled_template` | `enrollment_probe_tests` (8); code review |
| A7 | Probe bound: every directory entry counts toward `MAX_ENROLLMENT_PROBE_ENTRIES` (= 4096), including non-UTF-8 names. Iteration stops at the first match or at entry 4097, which returns `Err` (`InvalidPath` recommended). Entries are never collected into an unbounded `Vec`. An `Err` from `read_dir` or from an entry maps to `Err(BiometricStoreError::Io)`, never to `Ok(false)`. The error message holds no file content. | same | `test_pfu_probe_is_bounded_by_max_entries`, `test_pfu_probe_missing_directory_is_an_error` |
| A8 | Worker step 3b runs after the step-3 clock and before any `self.logind.` access, every tick. `Ok(false)` gives `tracker.clear()` and `Skipped(NotEnrolled)`. `Err(_)` gives `tracker.clear()` and `Skipped(TemplateStoreError)`. The backoff state is not touched, and the store error is not logged (only the reason code, through `skip`). `Ok(true)` continues, and step 7 is unchanged: the per-UID `get()` keeps deciding. | `worker.rs::tick` step 3b | PFU6 tests; `test_pfu_worker_probes_the_store_before_any_dbus_call` |
| A9 | `/etc/pam.conf` step at the start of `check_pam_stacks`, after the UID 0 refusal. First, `any_dir` uses `std::fs::metadata`, which follows symlinks: `Ok(m) if m.is_dir()` counts as a directory, while `NotFound` or a non-directory does not. Any other `metadata` error is `Undeterminable`, never "not a directory" combined with continuing. Second, `std::fs::symlink_metadata(pam_conf)`: `NotFound` means skip, and any other error is `Undeterminable`. If it is present and no directory exists, the result is `Undeterminable`. If it is present and a directory exists, the file is read with `read_source(pam_conf, MAX_PAM_FILE_BYTES)`: `Content` must be UTF-8 and pass `scan_pam_faillock_options == false`, and `Absent`, `Directory`, `None`, non-UTF-8 or an option found is `Undeterminable`. `DEFAULT_PAM_CONF = "/etc/pam.conf"`, and `with_pam_conf` is `#[must_use]`. | `account.rs::SystemAccountGuard::{check_pam_stacks, with_pam_conf, new}`, `mod.rs` | `presence_followups_pam_conf_tests` (10) |
| A10 | Line-scan rule: a logical line sets policy when it contains `pam_faillock.so` and the text after the end of its **first** occurrence (`find` / `split_once`, never `rfind` / `rsplit_once`) contains, as a substring, one of the 7 `PAM_POLICY_ARGUMENT_PREFIXES` or `even_deny_root`. The logical-line assembly is unchanged: `#` is stripped per physical line, then a trailing `\` after `trim_end` joins the next line. The final predicate is the substring rule OR the include rule. It is never ANDed with, or gated by, the old token rule. Remove the token rule (`pam_tokens`) or reduce it to the substring rule. Do not leave dead code. | `account.rs::{scan_pam_faillock_options, line_sets_faillock_policy}` | PFU4 tests; existing PAU scan tests; A13 |
| A11 | Include rule: split with `char::is_whitespace`. If **some** token equals `include`, `substack` or `@include` (ASCII case-insensitive) and **any later** token (not only the next one) contains `/`, the line counts as a hit. Plain names stay usable. | `account.rs` | `test_pfu_pam_scan_include_of_a_path_is_detected`, `test_pfu_pam_scan_plain_name_include_stays_usable`; A14 |
| A12 | The doc comments in `account.rs` contain `over-detect`, `superset`, `Unicode whitespace` and `pam.conf`. They state that the scan must never be narrowed, and they state the name-only limit (a renamed `pam_faillock.so` is not seen). In `mod.rs`, the doc of `DBUS_CALL_TIMEOUT_MS` contains `snapshot` and `round trip`, the doc of `DBUS_CONNECT_TIMEOUT_MS` contains `connect()` and `outside`, and both values stay unchanged. | `account.rs`, `mod.rs` | `presence_followups_contract` doc tests |
| A13 | **Tester addition (blocking).** Add a positive scan case whose policy option sits between two occurrences of the module name, for example `"auth required pam_faillock.so deny=1 note=pam_faillock.so\n"`. libpam passes `deny=1`, so the expected result is detected. This kills the "scan after the last occurrence" mutant, which under-detects. The case passes on the base, so it serves as a characterisation test (the module token is matched by suffix). | `crates/daemon/tests/presence_followups_tests.rs` (for example in `test_pfu_pam_scan_matches_the_module_by_suffix` or a new test) | new test |
| A14 | **Tester addition (blocking).** Add positive include cases where the `/` is not in the guard token right after `include`, for example `"auth include x\u{a0}/y\n"` (libpam target `x\u{a0}/y`, a nested path) and `"auth substack a\u{2003}../b\n"`. This kills the "next token only" mutant, which under-detects. The cases are red on the base, which has no include rule. | same file, `test_pfu_pam_scan_include_of_a_path_is_detected` or a new test | new test |
| A15 | No new log line carries file content, paths of PAM or store files, user names, error `Display` of `BiometricStoreError`, or template data. New skips go through `self.skip()`, which logs at debug level on change. The connect failure reuses the existing `logind_failed` warn (transport error variants only). | `worker.rs`, `logind.rs`, `account.rs` | `presence_unlock_contract::test_pau_presence_logs_no_biometric_field_above_debug`; review |
| A16 | Presence code stays read-only: none of the forbidden write patterns appears in `presence/`, and the probe in `store.rs` neither writes nor locks. | `presence/*`, `store.rs::has_enrolled_template` | `presence_unlock_contract::test_pau_presence_code_never_writes`; `test_pfu_probe_is_read_only` |
| A17 | No test is modified beyond the audited migrations above, and the PFU7 assertion stays exactly as approved by the owner (`Ok` or `Err(Timeout)`, deadline assertion always run). `crates/pam`, the IPC protocol and `main.rs` are untouched, and no dependency is added. | whole diff | `git diff origin/main --stat`; `cargo deny --locked check`; candid review |
| A18 | The full gate is green with the CI command forms: `cargo fmt --all -- --check`, `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`, `cargo test --locked --workspace --all-targets --all-features`. | workspace | CI |

### Pre-existing violations found (not introduced by this change)

- **O3 (evaluator):** the guard assumes `VENDORDIR=/usr/etc`. A libpam built with another
  `VENDORDIR` reads `<VENDORDIR>/pam.d` stacks that the guard never scans. Out of scope; it
  should be documented in `Docs/DAEMON.md` §6.
- `BiometricStore::list_enrolled` (not used by presence) has three problems:
  - its `read_dir` loop has no bound;
  - it accepts non-canonical UIDs (`+5`, `0123`) through `parse::<u32>()`;
  - it does not exclude symlinks or directories.

  It is a diagnostic API, and the new probe must not reuse it (A6, A7).
- When the outer `bounded()` timeout cancels a `ZbusLogind` call, the connection is not reset
  (only the inner per-call timeout resets it). This is harmless: the next call either succeeds
  or resets the connection.

### Clearance: BLOCKED: A13, A14

These are tester additions only. No spec change is needed, and the production design and the
migrations are cleared. When the A13 and A14 cases are added to
`presence_followups_tests.rs`, A13 passing on the base and A14 red on the base, the
developer may start under A1–A18 without another audit round.
