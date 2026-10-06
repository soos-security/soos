# Tester Contract — GitHub #339 follow-up: Failed-Password Alerts in `soos-remote` (feature level 1)

- **Date**: 2026-10-06
- **Branch**: `feat/remote-auth-alerts` (nothing committed, pushed or stashed; O-5)
- **Spec**: `AI/architect_spec_remote_auth_alerts.md` round 2 (plan-evaluator `VALIDATION_VERDICT: APPROVED`)
- **ADR**: "[2026-10-06] Failed-Password Alerts in `soos-remote` From the System Journal" (`AI/DECISIONS.md`)
- **Scope reminder (O-2, spec §0.1)**: every test encodes the safe subset only (time, source class, account class,
  kind, count). No test, fixture or seam carries, stores or displays a typed password; test 33 asserts that no raw
  field ever leaves the parser. The relayed request ("see the passwords that were tested") is still marked
  **open** for the owner's explicit confirmation in the spec and the plan-evaluator report; see "Open points".

## Files

| File | Kind | Tests |
|---|---|---|
| `crates/remote/tests/common/journal.rs` (new) | fixture: `JLine` builder, signal/noise line builders, `ScriptedJournal` (`JournalSource`) | — |
| `crates/remote/tests/journal_tests.rs` (new) | pure | 1–12, 51 (+ proptest companion of 5) |
| `crates/remote/tests/alerts_tests.rs` (new) | pure (+ `TempDir` for 22) | 13–22, 52, constants guard, `Copy` guard |
| `crates/remote/tests/alerts_server_tests.rs` (new) | end-to-end, paused frozen clock, `ScriptedJournal` | 23–36, 53–55 |
| `crates/remote/tests/journal_process_tests.rs` (new) | real child process / duplex pipe | 42, 43 |
| `crates/remote/tests/config_tests.rs` (appended `mod alerts_contract`) | pure | 37, 38, 57 |
| `crates/remote/tests/routes_tests.rs` (appended `mod alerts_contract`) | pure | 39, 40 |
| `crates/remote/tests/http_tests.rs` (appended `mod alerts_contract`) | pure | 41 |
| `tests/invariants/src/remote_alerts_contract.rs` (new) + `mod` line in `tests/invariants/src/lib.rs` | static | 44–50, 56 (RMC-S24–RMC-S31) |

## Tester Contract

Red evidence: in the hand-off state the crate's test targets do not compile because exactly the specified API is
missing (`unresolved import soos_remote::journal`, `soos_remote::alerts`, `config::AlertsConfig`,
`http::encode_sse_alerts_event`, the §3.1 constants, `RemoteConfig` has no field `alerts`). Assertion-level red was
observed with a temporary signature stub (wrong values, removed before hand-off): every new test failed under it
(the two no-panic property tests and the `Copy` guard excepted, by nature). Messages are quoted where they were
captured (server and invariant suites); the pure suites are recorded as "FAILED under the signature stub".
Invariant tests compile today and fail on their assertions.

| Test (path::name) | Acceptance line / matrix ID | Red evidence (failure message) |
|---|---|---|
| `journal_tests::test_rmc_alerts_probe_and_follow_args_are_exact` | A-1, §2.1 / RMC52 | FAILED under the signature stub (assertion on a stub value) |
| `journal_tests::test_rmc_alerts_journal_cursor_bounds` | §2.2 / RMC52 | FAILED under the signature stub (assertion on a stub value) |
| `journal_tests::test_rmc_alerts_parse_entry_accepts_string_and_byte_array_fields` | §2.3 / RMC46 | FAILED under the signature stub (assertion on a stub value) |
| `journal_tests::test_rmc_alerts_parse_entry_refuses_bad_shapes` | §2.3, F-11 / RMC46 | FAILED under the signature stub (assertion on a stub value) |
| `journal_tests::test_rmc_alerts_parse_entry_never_panics` (+ `…_never_panics_on_json_shapes`) | §2.3 / RMC46 | property (no-panic) test: passes against any non-panicking parser; compile-red until `parse_entry` exists |
| `journal_tests::test_rmc_alerts_classify_unix_chkpwd` | §2.4 / RMC47 | FAILED under the signature stub (assertion on a stub value) |
| `journal_tests::test_rmc_alerts_classify_pam_unix_trust_table` | §2.4 trust table, F-2 / RMC47 | FAILED under the signature stub (assertion on a stub value) |
| `journal_tests::test_rmc_alerts_classify_service_classes` | §2.4 service table / RMC47 | FAILED under the signature stub (assertion on a stub value) |
| `journal_tests::test_rmc_alerts_classify_faillock_locked_out_only` | A-7 / RMC50 | FAILED under the signature stub (assertion on a stub value) |
| `journal_tests::test_rmc_alerts_ignored_lines` | §2.4 prefix anchoring / RMC47 | FAILED under the signature stub (assertion on a stub value) |
| `journal_tests::test_rmc_alerts_account_is_mapped_to_three_classes` | A-8 / RMC48 | FAILED under the signature stub (assertion on a stub value) |
| `journal_tests::test_rmc_alerts_owner_login_bounds` | §2.4 `OwnerLogin` / RMC48 | FAILED under the signature stub (assertion on a stub value) |
| `journal_tests::test_rmc_alerts_exe_deleted_suffix_is_stripped` | A-5, F-2 b / RMC47 | FAILED under the signature stub (assertion on a stub value) |
| `alerts_tests::test_rmc_alerts_constants_match_the_spec` | §3.1 / RMC46–RMC55 | FAILED under the signature stub (assertion on a stub value) |
| `alerts_tests::test_rmc_alerts_correlator_scenarios` | §4.2 scenario table / RMC49, RMC50 | FAILED under the signature stub (assertion on a stub value) |
| `alerts_tests::test_rmc_alerts_correlator_pairs_in_both_orders` | §4.2 rules 1, 2, 5, 6 / RMC49 | FAILED under the signature stub (assertion on a stub value) |
| `alerts_tests::test_rmc_alerts_correlator_bounds` | §3.1 `MAX_PENDING_CHECKS`, `MAX_RECENT_FAILURES` / RMC49, RMC51 | FAILED under the signature stub (assertion on a stub value) |
| `alerts_tests::test_rmc_alerts_book_coalescing_and_eviction` | §4.3 coalescing, eviction / RMC51 | FAILED under the signature stub (assertion on a stub value) |
| `alerts_tests::test_rmc_alerts_book_counts_saturate` | §4.3 saturation / RMC51 | FAILED under the signature stub (assertion on a stub value) |
| `alerts_tests::test_rmc_alerts_book_acknowledge_through` | §4.3 rule A / RMC54 | FAILED under the signature stub (assertion on a stub value) |
| `alerts_tests::test_rmc_alerts_book_rebuild_respects_marker` | §4.3 rule R, F-3 b / RMC54 | FAILED under the signature stub (assertion on a stub value) |
| `alerts_tests::test_rmc_alerts_book_seq_overflow` | §4.3, §3.3 overflow / RMC51 | FAILED under the signature stub (assertion on a stub value) |
| `alerts_tests::test_rmc_alerts_book_marker_never_covers_unacknowledged` (test 52) | §4.3 rule M / RMC54 | FAILED under the signature stub (assertion on a stub value) |
| `alerts_tests::test_rmc_alerts_view_json_shape` | §4.4 / RMC48 | FAILED under the signature stub (assertion on a stub value) |
| `alerts_tests::test_rmc_alerts_ack_file_round_trip_and_fail_safe` | §6.1, A-9 / RMC54 | FAILED under the signature stub (assertion on a stub value) |
| `alerts_tests::test_rmc_alerts_signal_and_attempt_are_plain_values` | O-2 (no text in signals) | compile-red only (type guard) |
| `alerts_server_tests::test_rmc_alerts_disabled_by_default` | A-2, G-2 / RMC45 | stub: `AlertsConfig::default().lock_screen_programs` `left: [""]` |
| `alerts_server_tests::test_rmc_alerts_routes_require_authentication` | A-10, F-4 / RMC53, RMC55 | stub: `follower 1 not started within 1000 ms` |
| `alerts_server_tests::test_rmc_alerts_end_to_end_lock_screen_burst` | research §2.2 day, F-2 / RMC47, RMC49, RMC50, RMC52 | stub: `follower 1 not started` |
| `alerts_server_tests::test_rmc_alerts_probe_failure_is_unavailable_never_zero` | A-3, §5 steps 1/4, G-9 / RMC52 | stub: probe count `left: 0, right: 1` |
| `alerts_server_tests::test_rmc_alerts_follower_restart_resumes_after_cursor` | §5 steps 2/4, G-9 / RMC52 | stub: `follower 1 not started` |
| `alerts_server_tests::test_rmc_alerts_first_start_rebuilds_24h` | F-5, §5 step 3 / RMC52 | stub: `follower 1 not started` |
| `alerts_server_tests::test_rmc_alerts_owner_unresolved` | §3.3 / RMC52 | stub: `GET /api/alerts: not_found` `left: 404` |
| `alerts_server_tests::test_rmc_alerts_ack_csrf_headers_and_rate` | A-10, F-1 / RMC54 | stub: `follower 1 not started` |
| `alerts_server_tests::test_rmc_alerts_ack_persists_and_survives_restart` | A-9, G-1 / RMC54 | stub: `follower 1 not started` |
| `alerts_server_tests::test_rmc_alerts_sse_event_order_and_throttle` | §6 SSE / RMC55 | stub: `follower 1 not started` |
| `alerts_server_tests::test_rmc_alerts_never_expose_raw_fields` | O-2, A-8 / RMC48 | stub: `follower 1 not started` |
| `alerts_server_tests::test_rmc_alerts_audit_lines` | A-11 / RMC56 | stub: `assertion failed: j.probes() >= 3` |
| `alerts_server_tests::test_rmc_alerts_unavailable_never_affects_status_or_lock` | §7 / RMC51, RMC52 | stub: `GET /api/alerts: not_found` `left: 404` |
| `alerts_server_tests::test_rmc_alerts_existing_behaviour_unchanged_when_enabled` | A-12 / RMC55 | stub: `follower 1 not started` |
| `alerts_server_tests::test_rmc_alerts_ack_stale_epoch_is_refused` (test 53) | A-13, F-3 a / RMC54 | stub: `follower 1 not started` |
| `alerts_server_tests::test_rmc_alerts_pending_check_older_than_ack_stays_unacknowledged` (test 54) | §4.3 rules R/M, F-3 b / RMC54 | stub: `follower 1 not started` |
| `alerts_server_tests::test_rmc_alerts_rng_failure_is_unavailable` (test 55) | A-13 / RMC52 | stub: `GET /api/alerts: not_found` |
| `config_tests::alerts_contract::test_rmc_alerts_config_keys` | §3.2 / RMC45 | FAILED under the signature stub (assertion on a stub value) |
| `config_tests::alerts_contract::test_rmc_alerts_ack_path_resolution` | §3.2 / RMC54 | FAILED under the signature stub (assertion on a stub value) |
| `config_tests::alerts_contract::test_rmc_alerts_lock_screen_program_validation` (test 57) | F-2 b / RMC45 | FAILED under the signature stub (assertion on a stub value) |
| `routes_tests::alerts_contract::test_rmc_alerts_routes_table` | §6 / RMC53 | FAILED under the signature stub (assertion on a stub value) |
| `routes_tests::alerts_contract::test_rmc_alerts_ack_csrf_rules` | §6.1, F-1 / RMC54 | FAILED under the signature stub (assertion on a stub value) |
| `http_tests::alerts_contract::test_rmc_alerts_sse_alerts_event_encoding` | §6 / RMC55 | FAILED under the signature stub (assertion on a stub value) |
| `journal_process_tests::test_rmc_alerts_bounded_line_reader` | A-14, F-10 / RMC46 | FAILED under the signature stub (assertion on a stub value) |
| `journal_process_tests::test_rmc_alerts_journalctl_source_kills_child_on_drop` | A-1, F-8 / RMC52 | FAILED under the signature stub (assertion on a stub value) |
| `remote_alerts_contract::test_rmc_s24_journal_reader_is_spawned_safely` | RMC-S24 / RMC52 | `only the journal reader spawns a process` (left `[]`) |
| `remote_alerts_contract::test_rmc_s25_alert_modules_never_log` | RMC-S25 / RMC56 | `crates/remote/src/journal.rs must exist (spec §1.1)` |
| `remote_alerts_contract::test_rmc_s26_unit_unchanged_for_alerts` | RMC-S26 / RMC58 | **passes today by design** (regression guard: the unit must stay unchanged) |
| `remote_alerts_contract::test_rmc_s27_process_feature_only_in_remote` | RMC-S27 / RMC58 | `crates/remote takes tokio from the workspace with the process feature` |
| `remote_alerts_contract::test_rmc_s28_alert_types_without_redaction_have_no_debug` | RMC-S28 / RMC48 | `crates/remote/src/journal.rs must exist` |
| `remote_alerts_contract::test_rmc_s29_alerts_are_documented` | RMC-S29 / RMC58 | `Docs/REMOTE_COMPANION.md has a "## 2c." section` |
| `remote_alerts_contract::test_rmc_s30_page_alert_ui` | RMC-S30 / RMC57 | `app.js must contain /api/alerts` |
| `remote_alerts_contract::test_rmc_s31_line_buffers_are_zeroizing` | RMC-S31 (test 56) / RMC48 | `crates/remote/src/journal.rs must exist` |

### Contract choices where the spec is silent (binding for Phase 4)

1. **Test seams** (all `#[doc(hidden)]`): `AlertBook::with_next_seq(self, next_seq: u64) -> Self` (the seq the next
   attempt receives; recording assigns it and needs `next_seq + 1`, otherwise `Err(Overflow)` and nothing is
   recorded) and `AlertBook::force_newest_count(&mut self, count: u32)` (overwrites the newest record's count) —
   tests 17 and 20 cannot reach `u32::MAX` / `u64::MAX` otherwise; `BoundedLineReader::capacity()` (spec test 42).
2. **Ack-file API** (spec §6.1 gives no names): `alerts::read_ack_file(path, owner_uid) -> Result<Option<u64>,
   AckFileError>` (`Ok(None)` = absent, `Err` = invalid → marker 0 + WARN) and `alerts::write_ack_file(path,
   acknowledged_until_us) -> Result<(), AckFileError>`. The file must be a JSON **object** (a JSON array is refused
   even though serde's derived struct visitor would accept `[1,42]`); a FIFO or directory at the path is refused
   without blocking; the write never follows a symlink planted at the path and leaves no temporary file.
3. **Ack-file owner**: checked against `ServerState::file_owner_uid()` (the credential store's seam).
4. **`AlertSettings` has no `Clone` requirement**; `JournalError` must be `Copy` (spec derive list).
5. **Evicted totals are per kind**: an evicted `locked_out` record stays in `unacknowledged_locked_out`, never in
   `unacknowledged_wrong_password` (A-7: lockouts must not inflate the wrong-password count). The spec's single
   `evicted_unacknowledged: u32` field must therefore be split or tracked per kind (test 16).
6. **Probe retries stay `unavailable`**: a failing probe that is retried does not pass through `starting`, so
   repeated failures with the same reason emit **one** `password alerts unavailable` line (test 34); `starting` is
   entered when a probe succeeds.
7. **Ack rate gate**: requests that reach the gate (including those later refused with `400 BeyondNewest` or `409
   stale_view`) consume it; tests always wait `MIN_ALERT_ACK_INTERVAL_MS + 1` after such a request.
8. **CSRF refusal** of the ack route is `403 {"result":"forbidden"}` (lock route rule); the disabled check comes
   before the snapshot-header check (`403 alerts_disabled` even without the headers).
9. **First `alerts` event** of a stream is written right after the first `status` event (no other event in
   between), after the Funnel session re-check.
10. **An empty journal line** (`\n\n`) is handed out as an empty `LineRead::Line` (test 42).

### Plan-evaluator round-2 findings encoded (the spec has not folded them yet)

- **G-1**: `AlertBook::new` keeps the loaded marker as the high water (`marker(None) == loaded` with nothing
  unacknowledged below it, test 19) and test 31 restarts **twice** without a new acknowledgement (file value and
  acknowledged state unchanged).
- **G-2**: test 23 runs the disabled service with a failing CSPRNG and requires the `disabled` view (never
  `rng_failed`) and no probe.
- **G-9**: test 26 checks the backoff reset after a run of `JOURNAL_STABLE_RUN_MS` and the doubling after a short
  run; test 27 creates the configured locker between two follower starts and requires `monitored` after the
  restart; tests 31/53 use a per-call varying CSPRNG (different epoch per start).
- **Not encoded**: G-4 (separate longer bound for a follower that never prints a line) contradicts the spec's
  current rule (b) of §5 step 3; test 28 is written so that both the current rule and a periodic idle tick pass. If
  the architect folds G-4, test 28 needs no change, but a new test for the cold-start case would be added by the
  tester. G-3, G-5, G-6, G-7 (numbering RMC-S24–RMC-S31 already used here), G-8 are documentation items.

## Migrated existing tests

| Test | Old assertion | New assertion | Mandating acceptance line |
|---|---|---|---|
| `crates/remote/tests/common/harness.rs` `Harness::start_with` (`RemoteConfig` literal) | — (setup) | gains `alerts: soos_remote::config::AlertsConfig::default(),` | A-12, §12.7 (setup only) |
| `crates/remote/tests/server_tests.rs` `Harness::start_with` (`RemoteConfig` literal) | — (setup) | gains `alerts: soos_remote::config::AlertsConfig::default(),` | A-12, §12.7 (setup only) |

No assertion of any existing test is changed, no line of an existing test file is removed or modified:
`git diff --numstat` shows additions only (two setup lines; the three appended `mod alerts_contract` blocks; one
`mod` declaration in `tests/invariants/src/lib.rs`). `RequestHead` is untouched (F-1).

## Satisfiability check (validation prototype, not delivered)

To make sure no contract is self-contradictory before it becomes immutable, a throw-away prototype of
`journal.rs`, `alerts.rs`, the config keys, the two routes, the SSE `alerts` event and the follower was written in
the worktree, run, and **removed** (`git checkout -- crates/remote/src crates/remote/Cargo.toml`; the two new source
files deleted). With it, every new test above passed (crate suite: all test binaries green, including every
existing suite with the two setup lines), except the static invariants that need the real documentation, page and
manifest. The prototype is kept only outside the repository (session scratchpad,
`tester_validation_prototype/`) and is not a deliverable; Phase 4 writes the production code from the spec.

## Flakiness check

With the validation prototype, `alerts_server_tests`, `alerts_tests`, `journal_tests` and
`journal_process_tests` were run **10 times in a row**: 10/10 green each time (17 + 13 + 14 + 2 tests per run;
server suite ≈ 0.45 s, paused frozen clock). The full `soos-remote` suite (all 19 test binaries) was green once
more after that.

## Open points

1. **Relayed request (spec §0.1, plan-evaluator §5)**: the owner's explicit confirmation of the safe subset (when,
   where, account class, kind, count — never the typed text) was required "before Phase 2"; this contract was
   written on the orchestrator's instruction while that confirmation is still recorded as open. Nothing here
   depends on the answer except the feature itself: if the owner insists on the typed text, that is a different
   feature that `AGENTS.md` forbids and needs its own ADR; these tests then still describe the safe subset.
2. Test 43 spawns the host's real `/usr/bin/journalctl --follow` (read-only, no configuration touched) when it
   exists; on a host without it the test prints a note and returns.
3. The developer must add the `process` tokio feature in `crates/remote/Cargo.toml` only (test 47) and the §2c
   documentation, page strings and `index.html` `id="alerts"` banner (tests 49, 50).
