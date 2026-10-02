# Tester Contract — GitHub #325 (presence auto-unlock review follow-ups of #324)

- **Branch**: `fix/presence-review-followups` (base `origin/main` `35a708a`)
- **Spec**: `AI/architect_spec_presence_followups.md` revision 3 (`AI/plan_evaluator_report.md`)
- **Matrix**: new component `presence-review-followups`, rows PFU1–PFU7 (spec §7)
- **Tests added**: 50 new (47 + 3 from the auditor round), plus the PFU7 owner-approved assertion migration (20 + 5 + 11 daemon integration, 8 biometric-store integration, 6 invariants)
- **Red state**: every PFU1, PFU2, PFU3, PFU6 test and the PFU4 counter-example tests fail
  (assertion or compile error on exactly the specified API); the PFU5 test and the PFU4
  characterisation / negative tests pass on purpose (the behaviour exists, or must stay); their power
  is proven by the mutants listed below.

Run (CI command form): `cargo test --locked -p <package> --all-features --test <file>`;
invariants: `cargo test --locked -p soos-invariants --all-features presence_followups`.

## Contract

### `crates/daemon/tests/presence_followups_tests.rs` (compiles on the base; assertion red)

| Test | Matrix | Red evidence (base `35a708a`) |
|---|---|---|
| `test_pfu_attempt_is_stamped_with_a_fresh_clock_read` | PFU1 | `left: 40, right: 39` (stamp is the step-3 time) |
| `test_pfu_reserve_is_evaluated_at_the_fresh_timestamp` | PFU1 | `left: Skipped(RateLimited), right: Scanned(Unlocked { .. })` |
| `test_pfu_clock_failure_before_the_attempt_skips_without_attempt_or_scan` | PFU1 | `left: Scanned(Aborted(InternalError)), right: Skipped(ClockUnavailable)` |
| `test_pfu_clock_regression_before_the_attempt_skips_the_tick` | PFU1 | `left: Scanned(Unlocked { .. }), right: Skipped(ClockUnavailable)` |
| `test_pfu_attempt_is_stamped_after_the_policy_lock_is_acquired` | PFU1 (F5) | `left: 40, right: 39` |
| `test_pfu_second_account_check_never_runs_while_one_is_in_flight` | PFU5 | passes on the base (behaviour exists); killed by mutant M1 |
| `test_pfu_no_logind_traffic_while_nobody_is_enrolled` | PFU6 | `left: Skipped(InGrace), right: Skipped(NotEnrolled)` |
| `test_pfu_new_enrollment_is_picked_up_on_the_next_tick` | PFU6 | `left: Skipped(InGrace), right: Skipped(NotEnrolled)` |
| `test_pfu_removed_enrollment_stops_polling_and_restarts_the_grace` | PFU6 | `left: 2, right: 1` (snapshot taken with an empty store) |
| `test_pfu_store_listing_error_skips_before_logind` | PFU6 | `left: Skipped(InGrace), right: Skipped(TemplateStoreError)` |
| `test_pfu_pam_scan_detects_options_across_unicode_whitespace` | PFU4 | passes on the base (characterisation, must stay) |
| `test_pfu_pam_scan_backslash_before_comment_continues_the_line` | PFU4 | passes on the base (characterisation, must stay) |
| `test_pfu_pam_scan_unterminated_bracket_after_unicode_space_is_detected` | PFU4 (F1) | assertion failed: `"auth required pam_faillock.so x\u{a0}[a deny=1\n"` not detected |
| `test_pfu_pam_scan_line_after_a_commented_continuation_is_detected` | PFU4 (F1) | assertion failed: second line `deny=1` swallowed by the joined bracket |
| `test_pfu_pam_scan_include_of_a_path_is_detected` | PFU4 (R2-F1) | assertion failed: `auth include /etc/security/site-auth` not detected |
| `test_pfu_pam_scan_plain_name_include_stays_usable` | PFU4 | passes on the base (negatives, must stay) |
| `test_pfu_pam_scan_superset_keeps_negatives_undetected` | PFU4 | passes on the base (negatives, must stay) |
| `test_pfu_pam_scan_matches_the_module_by_suffix` | PFU4 | passes on the base (characterisation, must stay) |

### `crates/daemon/tests/presence_followups_connect_tests.rs` (compile red)

`error[E0407]: method 'connect' is not a member of trait 'PresenceLogind'` — exactly the specified
trait method (spec §2.2).

| Test | Matrix |
|---|---|
| `test_pfu_connect_slower_than_the_call_bound_is_still_awaited` | PFU2 |
| `test_pfu_hung_connect_is_bounded_by_the_connect_timeout` | PFU2 |
| `test_pfu_connect_failure_skips_before_the_snapshot` | PFU2 |
| `test_pfu_connect_failure_restarts_the_grace` | PFU2 |
| `test_pfu_connect_precedes_every_snapshot_and_is_skipped_without_templates` | PFU2, PFU6 (no `connect()` while the store is empty, F4) |

### `crates/daemon/tests/presence_followups_pam_conf_tests.rs` (compile red)

`error[E0432]: unresolved import 'soos_daemon::presence::DEFAULT_PAM_CONF'`,
`error[E0599]: no method named 'with_pam_conf' found for struct 'SystemAccountGuard'`.

| Test | Matrix |
|---|---|
| `test_pfu_default_pam_conf_is_etc_pam_conf` | PFU3 |
| `test_pfu_absent_pam_conf_keeps_the_account_usable` | PFU3 |
| `test_pfu_pam_conf_without_any_pam_directory_is_undeterminable` | PFU3 |
| `test_pfu_non_directory_pam_paths_do_not_hide_pam_conf` | PFU3 |
| `test_pfu_any_pam_directory_switches_to_the_scan` | PFU3 |
| `test_pfu_pam_conf_next_to_pam_d_is_scanned_for_faillock_options` | PFU3 |
| `test_pfu_unscannable_pam_conf_is_undeterminable` | PFU3 |
| `test_pfu_symlinked_pam_conf_is_followed` | PFU3 |
| `test_pfu_root_is_refused_before_pam_conf` | PFU3 |
| `test_pfu_guard_never_writes_pam_conf` | PFU3 |

### `crates/biometric-store/tests/enrollment_probe_tests.rs` (compile red)

`error[E0432]: unresolved import 'soos_biometric_store::store::MAX_ENROLLMENT_PROBE_ENTRIES'`,
`error[E0599]: no method named 'has_enrolled_template' found for struct 'BiometricStore'` (×10).

| Test | Matrix |
|---|---|
| `test_pfu_probe_bound_is_4096_entries` | PFU6 |
| `test_pfu_probe_is_false_for_an_empty_store` | PFU6 |
| `test_pfu_probe_is_true_with_one_template_and_false_after_delete` | PFU6 |
| `test_pfu_probe_ignores_entries_that_are_not_template_files` | PFU6 (F4: temp, `0123`, `+5`, `-1`, overflow, directory, symlink, dangling symlink) |
| `test_pfu_probe_does_not_decrypt` | PFU6 (F4) |
| `test_pfu_probe_is_bounded_by_max_entries` | PFU6 (F4: 4096 ⇒ `false`, 4097 ⇒ `Err`) |
| `test_pfu_probe_missing_directory_is_an_error` | PFU6 (F4) |
| `test_pfu_probe_is_read_only` | PFU6 |

### `tests/invariants/src/presence_followups_contract.rs` (assertion red; registered in `tests/invariants/src/lib.rs`)

| Test | Matrix | Red evidence |
|---|---|---|
| `presence_followups_contract::test_pfu_only_connect_opens_a_bus_connection` | PFU2 (F6) | `async fn connect(&self)` not found |
| `presence_followups_contract::test_pfu_worker_bounds_connect_outside_the_call_bound` | PFU2 | no `DBUS_CONNECT_TIMEOUT_MS` bound around `self.logind.connect()` |
| `presence_followups_contract::test_pfu_worker_probes_the_store_before_any_dbus_call` | PFU6 | `.has_enrolled_template()` not found in `tick` |
| `presence_followups_contract::test_pfu_logind_bounds_are_documented` | PFU2 | `DBUS_CALL_TIMEOUT_MS` doc lacks `snapshot` |
| `presence_followups_contract::test_pfu_account_guard_documents_over_detection_and_pam_conf` | PFU3, PFU4 | `over-detect` missing in `account.rs` docs |
| `presence_followups_contract::test_pfu_daemon_docs_describe_the_followups` | PFU2, PFU3, PFU6 | `/etc/pam.conf` missing in `Docs/DAEMON.md` §6 |

Required phrases (spec §2.2, §2.3, §10): `DBUS_CALL_TIMEOUT_MS` doc: `snapshot`, `round trip`;
`DBUS_CONNECT_TIMEOUT_MS` doc: `connect()`, `outside`; `account.rs` doc comments: `over-detect`,
`superset`, `Unicode whitespace`, `pam.conf`; `Docs/DAEMON.md` §6: `/etc/pam.conf`,
`DBUS_CONNECT_TIMEOUT_MS`, `whole snapshot`, `no template`. `Builder::address(` and `.build()`
exactly once in `logind.rs`, inside `ZbusLogind`'s `async fn connect(&self)`.

## Migrated existing tests (setup only, no assertion changed)

| Test | Old setup | New setup | Mandating line |
|---|---|---|---|
| `presence_worker_tests::test_pau_unusable_templates_cost_no_attempt_and_no_camera` ("not enrolled" case) | `vec![]` (empty store) | `vec![(1001, Enrollment::LiveIdentity)]` | #325 item 5 / PFU6: an empty store now skips before logind, so the first tick is `NotEnrolled` instead of the `InGrace` asserted by `tick_past_grace`; UID 1001 (no session) keeps the per-UID not-enrolled path (F2) |
| `presence_account_tests::Accounts::guard` (fixture of every account test) | — | `.with_pam_conf(self.path("etc/pam.conf"))` (absent) | #325 item 3 / PFU3: the guard now reads `pam.conf`; the fixture must not read the host `/etc/pam.conf` |
| `presence_logging_tests::test_pau_account_guard_never_logs_its_sources` | — | `.with_pam_conf(root.join("pam.conf"))` (absent) | same |
| `presence_candid_review_tests::guard_tree` | — | `.with_pam_conf(root.join("pam.conf"))` (absent) | same |

The three `with_pam_conf` migrations make those test binaries compile-red until the builder
exists (exactly the specified API). `presence_worker_tests` stays green on the base (44 passed).

### PFU7 — owner-approved assertion migration (owner decisions of 2026-10-02)

First decision: option (b), a setup-only readiness channel (server signals right before
`accept()`, client waits for it). Measured with 64 concurrent instances on CPU 0, 3 rounds =
192 runs each: before 4/192, 5/192 (earlier 3/192); after (b) 2/192, 8/192, 18/192, all
`simulated authentication must complete: Timeout`. No measurable effect, so the owner chose
option (a). The (b) channel is **reverted** (it does not help the capture: connect and write go to
the listen backlog and socket buffer whatever the server thread's state).

| Test | Old assertion | New assertion | Mandating line |
|---|---|---|---|
| `cli_deadline_json_tests::test_simulate_pam_auth_zero_timeout_is_clamped_to_pam_minimum` | `simulate_pam_auth(..)` must return `Ok` (`.expect("simulated authentication must complete")`) | `Ok(..)` **or** `Err(AdminCliError::Timeout)`; any other `Err` panics with the same message. Deadline assertion unchanged (`deadline >= before + 10 ms && deadline <= after + 10 ms`) and always run: the value is captured by the mock server and received with `recv_timeout(5 s)`, which fails the test if it never arrives | #325 item 6 / PFU7, owner decision 2026-10-02 (option a) |

Helper change: `capture_deadline(timeout_ms)` keeps its behaviour (`Completion::Required`) for
`test_simulate_pam_auth_deadline_uses_monotonic_clock` and
`test_simulate_pam_auth_huge_timeout_is_clamped_to_pam_maximum` (still require `Ok`, server write
still `.expect`ed); `capture_deadline_with(0, Completion::TimeoutTolerated)` is used only by the
clamp test, where the server's response write may fail (the timed-out client closed its socket).
The captured deadline is now received before the server thread is joined. No other assertion or
expected value changed. `cargo test --locked -p soos-admin-cli --all-features --all-targets`:
148 passed, 0 failed; clippy `-D warnings` and `cargo fmt --check` clean.

Stress after (a), same harness: **2/192, 0/192, 0/192** (250 ms control 0/192). Residual: the
client itself was descheduled past its 10 ms deadline before writing the request (mock server
`read length: UnexpectedEof`, then no captured deadline). That is correct production behaviour.
Under the owner rule "fail if the captured value never arrives" this residual stays; removing
it would mean skipping the deadline check when no request was sent, which needs an owner decision.

## Round (auditor A13/A14)

Cases added after the Phase 3 audit (`AI/auditor_constraints_presence_followups.md`, A13, A14).
They are new tests only: no existing assertion changed.

| Test | Auditor item / matrix | Base `35a708a` | Mutant killed |
|---|---|---|---|
| `presence_followups_tests::test_pfu_pam_scan_searches_after_the_first_module_occurrence` (`"auth required pam_faillock.so deny=1 note=pam_faillock.so\n"` ⇒ detected) | A13 / PFU4 (A10) | passes (the token rule already detects it) | scanning after the **last** occurrence (`rfind`) does not detect it |
| `presence_followups_tests::test_pfu_pam_scan_include_path_after_other_tokens_is_detected` (`"auth include x\u{a0}/y\n"`, `"auth substack a\u{2003}../b\n"` ⇒ detected) | A14 / PFU4 (A11) | **red**: assertion failed at `presence_followups_tests.rs:660` | checking only the token **right after** the directive detects neither case |
| `presence_followups_pam_conf_tests::test_pfu_pam_conf_path_under_a_regular_file_is_undeterminable` (optional; the parent of `pam.conf` is a regular file, `symlink_metadata` ⇒ `NotADirectory`, ⇒ `Undeterminable`) | PFU3 | compile red (file-level, `with_pam_conf`) | treating every `symlink_metadata` error as "absent" |

The scratch copy holding the full reference implementation had already been removed. These cases
were checked with a standalone scratch program instead (since deleted). It reimplements the
revision-3 scan rule exactly (logical-line assembly, substring rule after the first
`pam_faillock.so`, include rule with any later token). Results:
- every PFU4 positive and negative case gives the expected result, A13 and A14 included;
- the `rfind` mutant misses A13;
- the next-token mutant misses both A14 lines;
- `std::fs::symlink_metadata` under a regular file returns `ErrorKind::NotADirectory`, not
  `NotFound`.

`cargo fmt --check` is clean, and `cargo clippy -p soos-daemon --all-features --test
presence_followups_tests -- -D warnings` is clean. The optional `mark_scan_started` check was not
added: the tracker exposes no read of the scan-start stamp to tests, so it would need a new test
hook in production code.

## Validation against a throwaway reference implementation

A reference implementation of spec revision 3 was written in a scratch copy of the tree (never in
the worktree, now removed): all `soos-daemon`, `soos-biometric-store` and `soos-invariants`
targets pass (`--all-targets --all-features`, 1165 passed, 0 failed, every existing presence test
included) and `cargo clippy --locked -p soos-daemon -p soos-biometric-store -p soos-invariants
--all-targets --all-features -- -D warnings` is clean. Mutants of the reference, each killed by
the named test:

| Mutant | Killed by |
|---|---|
| M1 no in-flight flag in `check_account` | `test_pfu_second_account_check_never_runs_while_one_is_in_flight` |
| M2 clock read before the policy write lock | `test_pfu_attempt_is_stamped_after_the_policy_lock_is_acquired` |
| M3 `connect()` bounded by `DBUS_CALL_TIMEOUT_MS` | `test_pfu_connect_slower_than_the_call_bound_is_still_awaited` |
| M4 no `tracker.clear()` on an empty store | `test_pfu_removed_enrollment_stops_polling_and_restarts_the_grace` |
| M5 clock regression accepted | `test_pfu_clock_regression_before_the_attempt_skips_the_tick` |
| M6 probe follows symlinks | `test_pfu_probe_ignores_entries_that_are_not_template_files` |
| M7 non-canonical UID accepted | `test_pfu_probe_ignores_entries_that_are_not_template_files` |
| M8 probe bound off by one | `test_pfu_probe_is_bounded_by_max_entries` |
| M9 `pam.conf`-only system accepted | `test_pfu_pam_conf_without_any_pam_directory_is_undeterminable` |

(A mutant counting any existing PAM path as a directory is behaviour-equivalent: a non-directory
PAM path already fails the directory scan.)

## Flakiness check

On the reference: `presence_followups_tests` + `presence_followups_connect_tests` 10×: 10/10
green; `enrollment_probe_tests` + `presence_followups_pam_conf_tests` (+ the two above) 10×:
10/10 green. Timing margins: PFU2 uses the midpoint between the two bounds (750 ms vs 500 /
1000 ms) and an upper bound of connect + 2000 ms; PFU5 asserts the in-flight refusal is faster
than `ACCOUNT_CHECK_TIMEOUT_MS` and retries the post-release tick (the flag drops a few
instructions after the guard returns); PFU1 lock test holds the lock 300 ms after the step-7 signal
(steps 8–9 are mock calls).

## Notes for the developer

- The shared `target/` dir does not separate two checkouts of this workspace (cargo strips the
  workspace path from metadata): never point a second copy at the same target dir.
- Item 6: no production change; the owner-approved assertion migration is above (PFU7).
- O4: `@include` is matched ASCII case-insensitively like `include` / `substack` (Debian's
  `031_pam_include` uses `strcasecmp`); `test_pfu_pam_scan_include_of_a_path_is_detected` covers
  `@INCLUDE` and `@Include`.
