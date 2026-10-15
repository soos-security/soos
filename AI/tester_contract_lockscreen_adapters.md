# Tester Contract — GitHub #401: `soos-lockstatus` bridge and swaylock-plugin / hyprlock adapters

- **Date**: 2026-10-15 — **Branch**: `feat/lockscreen-adapters`
- **Spec**: `AI/architect_spec_lockscreen_adapters.md` (with §9–§11 revisions; plan evaluation
  APPROVED in round 3, `AI/plan_evaluator_report.md`).
- All tests are new; red was proven against signature stubs returning wrong values (the stubs
  compile; failures are assertion failures, not missing items).

| Test file | Tests | Matrix | Red evidence |
|---|---|---|---|
| `crates/lockstatus/tests/lockstatus_client_tests.rs` | `test_lka1_*` (request fields, one request per connection, each state, refusal of any verdict incl. `Allow`, other version, lengths 0/4097/`u32::MAX` refused before allocation, garbage/truncated/unknown state byte, old-daemon EOF, stall and late answer within the budget, missing socket, zero budget, overlong path) | LKA1 | 8 of 13 failed (stub `query_lock_state` returned `Timeout`, `fetch_view` `UNAVAILABLE`); the 5 passing ones are error-path guards that a fail-open implementation would break |
| `crates/lockstatus/tests/lockstatus_shared_tests.rs` | `test_lka2_*` (FNV-1a hash, record format and maximum length, malformed records, freshness and reuse incl. future stamp and other socket, `CLOCK_BOOTTIME`, private 0600 cache reused for 250 ms, missing clock disables the cache, failure back-off through the cache and saturation, absent/relative/missing/shared/foreign/symlinked runtime directory, 0644/hard-linked/symlinked/directory lock file, lock held → `LockBusy`, starved exchange (lock released at 240 ms) → `LockBusy` without failure record, 8 concurrent clients → one connection at a time and one request) | LKA2 | 15 of 17 failed |
| `crates/lockstatus/tests/lockstatus_view_format_tests.rs` | `test_lka3_*`, `test_lka4_*`, `test_lka5_json_*`, `test_lka7_*`, `test_lka8_cubic_bezier_*` | LKA3, LKA4, LKA5, LKA7, LKA8 | 13 of 14 failed |
| `crates/lockstatus/tests/lockstatus_poll_tests.rs` | `test_lka5_*` (interval clamp, back-off steps and reset, poller through the shared path, back-off on daemon failures only, `--watch` emits first view and changes only, ≤ 4 Hz, stops when asked; `test_lka5_poller_keeps_the_last_view_while_the_lock_is_busy` added after candid review 6) | LKA5 | 6 of 7 failed; the added test failed against the pre-fix poller |
| `crates/lockstatus/tests/lockstatus_cli_tests.rs` | `test_lka6_*` (each state, reduced-motion env, daemon down / stalled / refusing / old → Idle, exit 0, empty stderr, within `ONCE_EXIT_BUDGET_MS` + CI slack, no runtime dir → no contact, closed stdout, silent usage error with `--once`, reported usage error without it, `--watch` JSON line, `--watch` exits 0 on closed stdout; `test_lka6_watch_exits_when_stdout_closes_while_the_state_is_stable` added after candid review 1) | LKA6 | 6 of 12 failed (empty `main`); the added test failed against the pre-fix `--watch` (kept polling) |
| `crates/lock-widget/tests/lock_widget_scene_tests.rs` | `test_lka8_*`, `test_lka3_unavailable_draws_exactly_like_idle` (label first, cross-fade, eased quarter turns in the 180° domain, bounded 4.8 s loop, half-turn finish, 30 s presence-cadence replay without a jump, single shake per sequence, reduced motion, clock skew; `test_lka8_scan_joining_a_running_turn_stops_within_the_loop_limit` added after candid review 2, `test_lka8_daemon_restart_epoch_shakes_a_reused_sequence` after review 4, `test_lka8_scan_seen_again_after_a_transient_error_keeps_its_start` after review 6, `test_lka8_neutral_idle_answer_is_not_a_new_epoch` after review 7) | LKA3, LKA8 | 10 of 13 failed; the added test failed against the round-up extension |
| `crates/lock-widget/tests/lock_widget_render_tests.rs` | `test_lka9_*`, `test_lka10_*`, `test_lka11_palette_values_and_roles` | LKA9, LKA10, LKA11 | 16 of 16 failed |
| `crates/lock-widget/tests/lock_widget_cli_tests.rs` | `test_lka13_*` (no compositor → exit 1 after 1000 ms, one stderr line, no state, no panic; invalid `WAYLAND_SOCKET`; usage error; `test_lka13_usage_error_waits_the_restart_delay` added after candid review 2) | LKA13 | 3 of 4 failed; the added test failed against the immediate exit 2 |
| `tests/invariants/src/lockscreen_adapters_contract.rs` | `test_lka11_*`, `test_lka12_*` | LKA11, LKA12 | 5 of 11 failed (palette stub zeros, no docs/snippet/ADR, no Wayland code, no coordination code) |

### Migrated existing tests

None. The only edit to an existing test file is additive: `tests/invariants/src/lib.rs` registers
the new module and appends `"lockstatus"` and `"lock-widget"` to the
`test_business_crates_forbid_unsafe_code` list (strengthening; no assertion changed).

### Flakiness check

Timing-sensitive tests (`test_lka1_stalled_daemon_*`, `test_lka1_late_answer_*`,
`test_lka2_*lock*`, `test_lka2_concurrent_*`, `test_lka5_watch_*`, `test_lka6_*`,
`test_lka13_*`) assert only the product budget plus CI slack and are run 10 times in a row once
green (developer phase).
