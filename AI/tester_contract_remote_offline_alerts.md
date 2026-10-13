# Tester Contract — GitHub #376: Offline History and Back-Online Push (`soos-remote`)

Phase 2 contract of `AI/architect_spec_remote_offline_alerts.md` (§10, revised by §15 and §16).
Red evidence: every new test was run against signature stubs (`offline.rs` returning
`Unknown`/`None`/defaults, unmapped route, no-op scheduler, failing payload). Result: 23 of 23
pure tests, 10 of 10 served tests and 4 of 5 invariants failed on their assertions. ROA-S1 was
green on the base tree; it is a regression guard for the RC-1 property that already held.

Note on the Red run: it used the coordinator's shared cargo target directory, which was later
found to mix binaries of several worktrees. The Green gates were re-run in a private target
directory (walkthrough 202 §8).

| Test (path::name) | Acceptance line / matrix ID |
|---|---|
| `crates/remote/tests/offline_tests.rs::test_roa_constants_match_the_spec` | §3 constants, ROA8 |
| `…::test_roa_classify_states_and_connectivity` | LIMITED/PORTAL not offline, ROA3 |
| `…::test_roa_tracker_records_network_interval_with_boottime_duration` | monotonic + wall time, ROA3 |
| `…::test_roa_tracker_drops_intervals_below_floor` | D-6, ROA3 |
| `…::test_roa_tracker_wifi_disabled_upgrades_open_interval` | reason enum, ROA3 |
| `…::test_roa_tracker_detects_sleep_from_clock_gap` | sleep, ROA3 (includes the 30 000 ms boundary, F-11) |
| `…::test_roa_tracker_releases_notice_only_when_online` | §15 F-2, §16 R2-3, ROA5 |
| `…::test_roa_tracker_unmonitored_release` | D-15, §16 R2-3, NameHasOwner gate helper, ROA2/ROA5 |
| `…::test_roa_tracker_merges_pending_notices` | ROA7 |
| `…::test_roa_tracker_history_is_bounded_to_32` | ≤ 32, ROA3 |
| `…::test_roa_previous_run_classification` | unclean stop → shutdown_or_crash, ROA5 |
| `…::test_roa_tracker_stop_closes_open_interval_without_notice` | ROA5 |
| `…::test_roa_duration_label_vocabulary` | fixed vocabulary, ROA6 |
| `…::test_roa_notice_is_copy_and_merge_rules` | builder takes only enums and times, ROA6 |
| `…::test_roa_store_round_trip_and_mode_0600` | 0600, ROA4 |
| `…::test_roa_store_refuses_symlink_foreign_mode_oversize_and_malformed` | O_NOFOLLOW, owner/mode, ≤ 8 KiB, ROA4 |
| `…::test_roa_store_max_file_fits_bound` | ≤ 8 KiB, ROA4 |
| `…::test_roa_store_save_is_locked_and_atomic` | flock + temp + rename, ROA4 |
| `…::test_roa_offline_alerts_key` | config key, exit 78 without push, ROA8 |
| `…::test_roa_offline_route_table` | ROA9 |
| `…::test_roa_back_online_payload_detailed_and_generic` | fixed vocabulary, `generic` honoured, ROA6 |
| `…::test_roa_scheduler_shares_spacing_and_hourly_cap` | shared coalescing and hourly limit, ROA7 |
| `…::test_roa_scheduler_merges_notices` | ROA7 |
| `crates/remote/tests/offline_server_tests.rs::test_roa_off_installs_nothing` | off → no bus read, no file, ROA8 |
| `…::test_roa_offline_without_push_is_inert` | D-9, ROA8 |
| `…::test_roa_network_outage_sends_one_back_online_push` | one push, topic, previews, ROA1/ROA6 |
| `…::test_roa_unclean_previous_stop_is_shutdown_or_crash` | ROA5 |
| `…::test_roa_clean_stop_writes_marker_and_shutdown_read` | ROA5 |
| `…::test_roa_store_failed_is_unavailable_and_never_rewritten` | ROA4 |
| `…::test_roa_view_over_funnel_requires_session` | ROA9 |
| `…::test_roa_network_monitor_unavailable_without_networkmanager` | D-15, invalid clock (F-5), ROA3 |
| `…::test_roa_alert_and_back_online_share_the_budget` | ROA7 |
| `…::test_roa_heartbeat_saves_at_most_once_per_minute` | F-11, ROA4 |
| `tests/invariants/src/remote_offline_contract.rs::test_roa_s1_no_inet_socket_and_unit_unchanged` | RC-1, ROA1 |
| `…::test_roa_s2_bus_is_read_only` | no NM mutating call, ROA2 |
| `…::test_roa_s3_no_network_identifiers_and_notice_fields` | privacy, ROA6/ROA10 |
| `…::test_roa_s4_page_card_and_service_worker` | web app card, ROA9 |
| `…::test_roa_s5_docs_adr_matrix_walkthrough` | ROA11 |

### Migrated existing tests (setup only, no assertion touched)

The `RemoteConfig` struct literals in the following files gained
`offline: soos_remote::config::OfflineConfig::default(),`, the field this issue adds (spec §9;
the same precedent as Web Push ADR item (10)):

- `crates/remote/tests/common/harness.rs`
- `crates/remote/tests/common/camera.rs`
- `crates/remote/tests/server_tests.rs`
- `crates/remote/tests/alerts_server_tests.rs`
- `crates/remote/tests/push_server_tests.rs`
- `crates/remote/tests/battery_server_tests.rs`
- `crates/remote/tests/android_push_tests.rs`

### Corrections made during Phase 2 (before any test became a contract)

- Plan evaluation round 1 changed four contracts before Green:
  - tests 6 and 17 and the constants test were rewritten for §15 (release only on `Online`; exit 78 without push; budgets of 3000 ms);
  - test 23 became `test_roa_offline_without_push_is_inert`;
  - the heartbeat, invalid-clock and sleep-boundary cases were added.
- Round 2 (R2-3) lowered test 6's `Unknown` loop to 11 steps and added the count-restart case to test 6b.
- Test 6 also expected the held notice alone at the final `Online` step. It overlooked the outage opened at 5 s, which that step closes and merges. The expectation now names the merged notice. No assertion was weakened: the test checks the recorded interval and the exact merged value.

### Corrections found by the first full workspace run (before hand-off)

- ROA-S4 required static markup ids (`#offline`, `#offline-summary`, …) in `index.html`. The
  existing immutable contract RMC-S50 (`test_rmc_s50_ui_contract_ids_labels_and_attributes_are_kept`)
  pins the page's id set exactly, so the new test contradicted it. The card is now built at run time
  without ids, like the camera card and the battery line. ROA-S4 now asserts that `index.html`
  contains no offline markup and that `app.js` builds the card (class, title, insertion point).
  The existing test is untouched.
- ROA-S2's member-literal filter caught the `Debug` texts of `NetworkStore` and
  `SystemOfflineClock`. The code now uses lower-case `Debug` texts; the test is unchanged.
- The matrix citation invariant rejected two new rows. ROA9's unescaped `GET|HEAD` split its
  cells; it is now `GET\|HEAD`. ROA8's brace-grouped citations did not resolve; they are now
  cited one by one. The matrix wording was fixed and no test changed.

### Not driven end to end

A stop whose save misses `OFFLINE_STOP_DEADLINE_MS` cannot be driven: the frozen tokio clock of the
harness cannot advance while `serve` awaits its stop step. The case is bounded by construction (an
outer `timeout`; the marker stays `running`, so the next start reports `shutdown_or_crash`, the
fail-safe direction).

### Flakiness check

The served suite runs on the paused tokio clock with the harness's `FrozenClock`. File checks poll
in bounded real time (`wait_store`, `wait_view`, about 3 s), following the pattern of the
2026-10-07 push migration.
