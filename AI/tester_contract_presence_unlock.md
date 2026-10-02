# Tester Contract — GitHub #323: Presence Auto-Unlock of Locked Local Sessions

- **Phase**: 2 (Tester Agent, TDD Red), branch `feat/presence-auto-unlock`, base `f76a80b`
- **Spec**: `AI/architect_spec_presence_unlock.md` revision 3 (`APPROVED`, `AI/plan_evaluator_report.md` round 4)
- **Criteria**: PAU1–PAU20, PAU22–PAU29 automated. PAU21 is the hardware procedure (`⬜ Pending`); it has no automated test.
- **Total**: 198 contract tests (`test_pau_*` / `prop_pau_*`) in 11 files, plus 1 migrated existing invariant.

All the tests are hermetic. Account sources (`faillock.conf`, `pam.d/`, tally files, `shadow`) and sysfs trees live in tempdirs. The realtime clock is injected. Logind is `MockPresenceLogind`, so no D-Bus is used. The camera is the `mock-camera` `MockCameraManager` behind a counting `SpyCamera`. Vision runs on the `soos-inference-ort` mock backends. The worker clock is a per-test `fn` pointer: `test_clock!`, `CLOCK_MONOTONIC + static offset`.

## Test doubles (`crates/daemon/tests/common/mod.rs`)

| Double | Spec §8 hook | Notes |
|---|---|---|
| `MockPresenceLogind` (cloneable handle over `LogindScript`) | `MockPresenceLogind` | Scripted `seat_sessions` snapshot plus one-shot queue. `session_state` either comes from a one-shot queue or is looked up in the snapshot. Settable `lid_closed` and `unlock_session` results. Every call can be made to never resolve (`hang_*`). Each call has a counter, and the unlocked IDs are recorded. `on_session_state` / `on_unlock` hooks shift the test clock or inject faults. |
| `TestDisplay` | `StaticDisplayProbe(DisplayState)` | Settable at runtime. |
| `StaticAccountGuard`, `ScriptedAccountGuard` | same | Scripted answers per call, with an optional blocking delay, a call counter and the recorded `(name, uid)`. |
| `SpyCamera` | `MockCameraManager` with a `notify_activity` counter | Counts wakes and capture reads. Can be held not ready. Re-stamps every frame with the test clock, so a shifted clock keeps frames fresh. `on_wake` hook. |
| `CountingExtractor` | mock vision backends | Counts inferences and runs a per-inference hook: register an `InteractiveDemandGuard`, or panic. |
| `test_clock!(OFFSET, clock)` | `with_clock_fn` + `static AtomicU64` | Shifted monotonic clock, one per test. |

## API the tests rely on that the spec names but does not fully specify (developer must match)

These choices were made in Phase 2 and are binding for the developer. Each one is listed under "Spec ambiguities resolved" below.

```rust
// presence/mod.rs
pub fn reconnect_backoff(consecutive_failures: u32) -> Duration; // 0 -> ZERO, 1 -> 1 s, x2, saturates at 30 s
pub const DEFAULT_PAM_DIRS: [&str; 3]; // or &[&str]; compared to an array of 3

// presence/logind.rs
impl PresenceLogindError { pub fn call(message: &str) -> Self; } // Call(text <= MAX_LOGIND_ERROR_LEN bytes, char boundary)

// presence/worker.rs
impl SkipReason { pub const fn as_str(self) -> &'static str; } // snake_case of the variant ("no_locked_session", ...)

// presence/account.rs
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaillockPolicy { pub dir: PathBuf, pub deny: u16, pub fail_interval: u32, pub unlock_time: u32,
                            pub root_unlock_time: u32, pub admin_group: Option<String>, pub even_deny_root: bool }
impl Default for FaillockPolicy { /* /run/faillock, 3, 900, 600, 600, None, false */ }
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub struct TallyRecord { pub status: u16, pub time: u64 }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShadowEntry { pub password_locked: bool, pub lastchg: Option<i64>, pub max: Option<i64>,
                         pub inactive: Option<i64>, pub expire: Option<i64> }
pub fn parse_faillock_conf(content: &[u8]) -> Option<FaillockPolicy>;          // None = Undeterminable
pub fn decode_tally_records(bytes: &[u8]) -> Option<Vec<TallyRecord>>;         // None = bad size / > MAX_TALLY_BYTES
pub fn read_tally_records(path: &Path) -> Option<Vec<TallyRecord>>;            // missing -> Some(vec![]); None = Undeterminable
pub fn faillock_denies(policy: &FaillockPolicy, records: &[TallyRecord], now_s: u64) -> bool;
pub fn scan_pam_faillock_options(content: &str) -> bool;                       // true = a policy option is set
pub fn find_shadow_entry(content: &[u8], user: &UserName) -> Option<ShadowEntry>; // None = Undeterminable
pub fn shadow_refusal(entry: &ShadowEntry, now_s: u64) -> Option<AccountRefusal>; // None = usable
impl SystemAccountGuard {
    pub fn new() -> Self; // production paths
    pub fn with_faillock_conf(self, PathBuf) -> Self;
    pub fn with_vendor_faillock_conf(self, PathBuf) -> Self;
    pub fn with_default_faillock_dir(self, PathBuf) -> Self; // replaces DEFAULT_FAILLOCK_DIR wherever a policy leaves `dir` at its default
    pub fn with_pam_dirs(self, Vec<PathBuf>) -> Self;
    pub fn with_shadow(self, PathBuf) -> Self;
    pub fn with_realtime_fn(self, fn() -> Result<u64, DaemonError>) -> Self;
}
```

The spec does fully define these items, and the tests use them as written: `ConsensusContext` / `ConsensusRun` / `run_face_consensus` / `wake_camera` and the `CAMERA_WAKE_*` constants; `InferencePriority`, `InteractiveDemandGuard`, `InferenceGate::{register_interactive, interactive_demand, try_acquire_background}` with `Clone`; `PipelineComponents: Clone`; `PresenceConfig` and its constants; `DaemonConfig::presence`; `SessionId`, `LogindSessionState`, `PresenceLogind` (RPITIT), `session_state_from_properties`; `DisplayState`, `DisplayProbe`, `SysfsDisplayProbe`, `classify_connectors`; `PresenceSwitch`; `LockEntry`, `LockTracker`, `select_candidate`; `UserName`, `AccountRefusal`, `AccountState`, `AccountGuard`; `SkipReason`, `ScanOutcome`, `PresenceTick`, `PresenceWorker::{new, with_expected_embedding_model, with_clock_fn, tick, run}`; `RateLimiter::check_and_record_with_reserve`, `AuthorizationEngine::record_attempt_with_reserve`.

Module paths used: `soos_daemon::consensus::*`; `soos_daemon::presence::{config, logind, display, switch, tracker, account, worker}::*`; the §2.6 constants come from `soos_daemon::presence::*`. `SkipReason`, `ScanOutcome` and `PresenceTick` come from `presence::worker`.

## Tester Contract — GitHub #323

### Counts per criterion

| Criterion | Tests | Files |
|---|---|---|
| PAU1 config | 9 | `presence_config_tests.rs` |
| PAU2 40/60 s default | 5 | `rate_limit_reserve_tests.rs` (2), `presence_config_tests.rs` (1), `inference_priority_tests.rs` (1), invariant C5 (1) |
| PAU3 reserve | 11 | `rate_limit_reserve_tests.rs` (11, 2 of them proptests) |
| D10 reserve in worker | 2 | `presence_worker_tests.rs` |
| PAU4 kill switch | 7 | `presence_display_tests.rs` (5), `presence_worker_tests.rs` (2) |
| PAU5 binding | 2 | `presence_tracker_tests.rs`, `presence_worker_tests.rs` |
| PAU6 grace / interval | 9 | `presence_tracker_tests.rs` (6), `presence_worker_tests.rs` (3) |
| PAU7 one candidate | 3 | `presence_tracker_tests.rs` (1), `presence_worker_tests.rs` (2) |
| PAU8 unusable templates | 1 (3 cases) | `presence_worker_tests.rs` |
| PAU9 lid / screen | 9 | `presence_display_tests.rs` (7), `presence_worker_tests.rs` (2) |
| PAU10 pipeline unchanged | 7 | `presence_consensus_tests.rs` (4), `presence_worker_tests.rs` (2), invariant C4 (1) |
| PAU11 Allow + re-check | 10 | `presence_worker_tests.rs` (9, plus the nominal path), invariant C3 (1) |
| PAU12 PAM priority | 16 | `inference_priority_tests.rs` (9), `presence_consensus_tests.rs` (5), `presence_worker_tests.rs` (2) |
| PAU13 camera standby | 1 (5 scenarios × 20 ticks) | `presence_worker_tests.rs` |
| PAU14 bounds / timeouts / backoff | 9 | `presence_config_tests.rs` (2), `presence_tracker_tests.rs` (1), `presence_logind_mapping_tests.rs` (1), `presence_worker_tests.rs` (4), `presence_logging_tests.rs` (1) |
| PAU15 locker ignored | 3 | `presence_tracker_tests.rs` (2), `presence_worker_tests.rs` (1) |
| PAU16 logging | 5 | `presence_logging_tests.rs` (4), invariant (1) |
| PAU17 dependency / bus | 6 | invariants C1 (3), C2 (2), `presence_config_tests.rs` constants (1) |
| PAU18 GetAll mapping | 8 | `presence_logind_mapping_tests.rs` |
| PAU19 start / stop / no bus | 3 | `presence_worker_tests.rs` (1), `presence_logging_tests.rs` (1), invariant (1) |
| PAU20 documentation | 4 | invariants C7 (2), operator docs (1), migrated `daemon_docs_contract` (1) |
| PAU22 faillock.conf | 9 | `presence_account_tests.rs` |
| PAU23 tally reading | 9 | `presence_account_tests.rs` |
| PAU24 check_tally replica | 10 | `presence_account_tests.rs` (1 of them a proptest against a reference port) |
| PAU25 PAM option scan | 5 | `presence_account_tests.rs` (1 of them a proptest) |
| PAU26 shadow | 9 | `presence_account_tests.rs` |
| PAU27 user names / root / `Name` | 6 | `presence_account_tests.rs` (2), `presence_logind_mapping_tests.rs` (1), `presence_tracker_tests.rs` (1), `presence_worker_tests.rs` (2) |
| PAU28 guard in worker | 4 | `presence_worker_tests.rs` |
| PAU29 no writes / sandbox | 4 | invariants C8 (1), C9 (1), `presence_account_tests.rs` (1), plus C7 / docs keywords |
| C6 kill-switch dir | 2 | `presence_config_tests.rs` (1), invariant (1) |
| PAU21 hardware | 0 | ⬜ Pending (`tests/physical/screensaver_test.md`, procedure written in Phase 6) |

Several tests cover more than one criterion; each is counted under its main one.

### Contract table

Red evidence codes:
- **E-API**: compile error on the specified, not yet existing API only (`soos_daemon::presence`, `soos_daemon::consensus`, `DaemonConfig::presence`, `InferencePriority`, `InteractiveDemandGuard`, `InferenceGate::{register_interactive, interactive_demand, try_acquire_background}`, `RateLimiter::check_and_record_with_reserve`, `AuthorizationEngine::record_attempt_with_reserve`).
- **A**: assertion failure (message quoted in the red evidence section).

| Test (path::name) | Criterion | Red |
|---|---|---|
| `crates/policy/tests/rate_limit_reserve_tests.rs::test_pau_default_max_attempts_is_forty_per_minute` | PAU2 | E-API |
| `…::test_pau_default_limiter_refuses_the_forty_first_attempt` | PAU2 | E-API |
| `…::test_pau_reserve_records_while_more_than_reserve_remain` | PAU3 | E-API |
| `…::test_pau_reserve_refuses_without_recording_at_the_reserve_boundary` | PAU3 | E-API |
| `…::test_pau_reserve_accepts_when_exactly_one_above_the_reserve` | PAU3 | E-API |
| `…::test_pau_reserve_zero_equals_check_and_record` | PAU3 | E-API |
| `…::test_pau_reserve_at_or_above_max_attempts_always_refuses` | PAU3 | E-API |
| `…::test_pau_reserve_refuses_whenever_check_and_record_refuses` | PAU3 | E-API |
| `…::test_pau_reserve_prunes_expired_attempts_before_evaluating` | PAU3 | E-API |
| `…::test_pau_reserve_is_evaluated_per_uid` | PAU3 | E-API |
| `…::test_pau_engine_record_attempt_with_reserve` | PAU3 | E-API |
| `…::prop_pau_presence_never_consumes_the_reserve` | PAU3 (property) | E-API |
| `…::prop_pau_reserve_zero_matches_check_and_record` | PAU3 (property) | E-API |
| `crates/daemon/tests/presence_config_tests.rs::test_pau_presence_defaults_and_bounds_match_spec` | PAU1 | E-API |
| `…::test_pau_absent_empty_file_and_empty_table_give_the_same_presence_config` | PAU1 | E-API |
| `…::test_pau_presence_keys_are_parsed` | PAU1 | E-API |
| `…::test_pau_out_of_range_presence_intervals_are_startup_errors` | PAU1 (0, 999, 60001) | E-API |
| `…::test_pau_zero_interval_is_rejected_even_when_disabled` | PAU1 (0 never a sentinel) | E-API |
| `…::test_pau_presence_interval_bounds_are_accepted` | PAU1 (1000, 60000) | E-API |
| `…::test_pau_presence_wrong_type_is_a_startup_error` | PAU1 | E-API |
| `…::test_pau_presence_validate_names_the_key_and_is_called_by_daemon_validate` | PAU1 | E-API |
| `…::test_pau_presence_warns_when_max_attempts_leaves_no_room_for_scans` | §2.6 warnings / D10 | E-API |
| `…::test_pau_presence_warns_when_scans_will_be_throttled` | §2.6 warnings | E-API |
| `…::test_pau_presence_defaults_and_disabled_feature_do_not_warn` | §2.6 warnings | E-API |
| `…::test_pau_presence_rate_limit_warning_is_not_an_error` | §2.6 warnings | E-API |
| `…::test_pau_daemon_default_rate_limit_is_forty_per_minute` | PAU2 | E-API |
| `…::test_pau_presence_constants_match_spec` | PAU14, PAU17, §2.6 | E-API |
| `…::test_pau_kill_switch_dir_is_the_pam_flag_dir` | C6 | E-API |
| `…::test_pau_reconnect_backoff_doubles_and_saturates` | PAU14 | E-API |
| `crates/daemon/tests/presence_tracker_tests.rs::test_pau_session_id_parse_accepts_only_logind_ids` | PAU5 (invalid IDs) | E-API |
| `…::test_pau_only_bound_and_locked_sessions_are_tracked` | PAU5 | E-API |
| `…::test_pau_grace_boundary_is_inclusive` | PAU6 (`>=`) | E-API |
| `…::test_pau_still_locked_session_keeps_its_lock_period` | PAU6 | E-API |
| `…::test_pau_relock_after_unlock_absence_or_inactivity_restarts_grace` | PAU6 | E-API |
| `…::test_pau_uid_change_is_a_new_lock_period` | PAU6 | E-API |
| `…::test_pau_lock_after_presence_unlock_starts_a_fresh_grace` | PAU6 | E-API |
| `…::test_pau_scan_interval_spaces_scans_of_one_session` | PAU6 / scan interval | E-API |
| `…::test_pau_due_is_saturating_on_a_clock_going_backwards` | PAU6 | E-API |
| `…::test_pau_tracker_overflow_clears_and_fails_closed` | PAU14 | E-API |
| `…::test_pau_unconfirmed_unlock_marks_the_locker_ignored` | PAU15 | E-API |
| `…::test_pau_no_unlock_request_never_expires` | PAU15 | E-API |
| `…::test_pau_select_candidate_requires_exactly_one` | PAU7 | E-API |
| `crates/daemon/tests/presence_display_tests.rs::test_pau_classify_connectors_table` | PAU9 | E-API |
| `…::test_pau_sysfs_probe_reads_a_connected_panel` | PAU9 | E-API |
| `…::test_pau_sysfs_probe_reads_a_blanked_screen_as_off` | PAU9 | E-API |
| `…::test_pau_sysfs_probe_ignores_non_connector_entries` | PAU9 (`renderD*`, `cardN`) | E-API |
| `…::test_pau_sysfs_probe_ignores_oversized_attributes` | PAU9 | E-API |
| `…::test_pau_sysfs_probe_follows_class_symlinks` | PAU9 | E-API |
| `…::test_pau_sysfs_probe_unknown_cases` | PAU9 (unreadable, > 64, none connected) | E-API |
| `…::test_pau_sysfs_probe_examines_up_to_the_entry_bound` | PAU9 | E-API |
| `…::test_pau_kill_switch_is_off_without_flags` | PAU4 | E-API |
| `…::test_pau_kill_switch_engages_on_any_entry_kind` | PAU4 (file, dir, dangling symlink) | E-API |
| `…::test_pau_kill_switch_fails_closed_on_stat_errors` | PAU4 | E-API |
| `…::test_pau_gdm_disable_does_not_engage_the_kill_switch` | D9 / Q3 | E-API |
| `…::test_pau_kill_switch_is_re_evaluated_on_every_call` | PAU4 | E-API |
| `crates/daemon/tests/presence_logind_mapping_tests.rs::test_pau_get_all_reply_maps_to_a_bound_locked_state` | PAU18 | E-API |
| `…::test_pau_id_mismatch_is_malformed` | PAU18 | E-API |
| `…::test_pau_active_follows_the_session_record_rule` | PAU18 | E-API |
| `…::test_pau_remote_is_only_trusted_as_a_boolean` | PAU18 / PAU5 | E-API |
| `…::test_pau_empty_seat_and_class_map_to_none` | PAU18 | E-API |
| `…::test_pau_missing_or_ill_typed_locked_hint_is_not_locked` | PAU18 | E-API |
| `…::test_pau_missing_or_ill_typed_user_leaves_uid_unset` | PAU18 | E-API |
| `…::test_pau_missing_or_invalid_name_maps_to_none` | PAU27 | E-API |
| `…::test_pau_logind_call_error_text_is_bounded` | PAU14 | E-API |
| `crates/daemon/tests/inference_priority_tests.rs::test_pau_inference_priority_variants` | PAU12 | E-API |
| `…::test_pau_interactive_demand_counts_live_guards` | PAU12 | E-API |
| `…::test_pau_gate_clones_share_permits_and_demand` | PAU12 | E-API |
| `…::test_pau_background_acquisition_yields_to_interactive_demand` | PAU12 | E-API |
| `…::test_pau_background_acquisition_never_waits` | PAU12 | E-API |
| `…::test_pau_interactive_acquires_after_a_background_job` | PAU12 | E-API |
| `…::test_pau_dispatcher_holds_an_interactive_guard_for_every_auth_request` | PAU12 | E-API |
| `…::test_pau_dispatcher_drops_the_guard_on_every_return_path` | PAU12 | E-API |
| `…::test_pau_auth_during_a_presence_job_still_allows_within_its_deadline` | PAU12 | E-API |
| `…::test_pau_forty_first_auth_attempt_is_rate_limited_by_default` | PAU2 | E-API |
| `crates/daemon/tests/presence_consensus_tests.rs::test_pau_camera_wake_constants_keep_the_dispatcher_values` | PAU10 | E-API |
| `…::test_pau_consensus_allows_after_exactly_three_passing_captures` | PAU10 (k = 3) | E-API |
| `…::test_pau_consensus_one_spoof_capture_vetoes` | PAU10 | E-API |
| `…::test_pau_consensus_applies_the_caller_thresholds` | PAU10 (raised `match_threshold`) | E-API |
| `…::test_pau_background_consensus_is_preempted_before_its_first_inference` | PAU12 | E-API |
| `…::test_pau_background_consensus_is_preempted_between_captures` | PAU12 | E-API |
| `…::test_pau_interactive_consensus_is_never_preempted` | PAU12 | E-API |
| `…::test_pau_background_consensus_polls_a_busy_slot_without_queueing` | PAU12 | E-API |
| `…::test_pau_consensus_aborts_on_vision_inference_error` | PAU11 | E-API |
| `…::test_pau_consensus_aborts_on_inference_job_panic` | PAU11 | E-API |
| `…::test_pau_consensus_aborts_on_clock_failure` | PAU11 | E-API |
| `…::test_pau_wake_camera_is_bounded` | PAU11 / PAU14 | E-API |
| `crates/daemon/tests/presence_worker_tests.rs::test_pau_worker_unlocks_an_eligible_session_after_the_grace` | PAU10, PAU11, PAU28 (nominal) | E-API |
| `…::test_pau_kill_switch_stops_every_side_effect` | PAU4 | E-API |
| `…::test_pau_kill_switch_removal_resumes_with_grace_restarted` | PAU4 | E-API |
| `…::test_pau_ineligible_sessions_are_never_scanned_or_unlocked` | PAU5 | E-API |
| `…::test_pau_worker_respects_the_lock_grace` | PAU6 | E-API |
| `…::test_pau_worker_relock_restarts_the_grace` | PAU6 | E-API |
| `…::test_pau_worker_spaces_scans_by_the_scan_interval` | PAU6 / scan interval | E-API |
| `…::test_pau_two_eligible_sessions_are_ambiguous` | PAU7 | E-API |
| `…::test_pau_enrolled_session_is_scanned_next_to_a_not_enrolled_one` | PAU7 | E-API |
| `…::test_pau_unusable_templates_cost_no_attempt_and_no_camera` | PAU8 | E-API |
| `…::test_pau_closed_lid_or_blank_screen_gates_the_scan` | PAU9 | E-API |
| `…::test_pau_undetectable_lid_or_screen_does_not_gate` | PAU9 | E-API |
| `…::test_pau_spoof_capture_vetoes_the_presence_scan` | PAU10 | E-API |
| `…::test_pau_policy_thresholds_apply_to_presence` | PAU10 | E-API |
| `…::test_pau_recheck_refusals_never_unlock` | PAU11 | E-API |
| `…::test_pau_logind_error_at_recheck_never_unlocks` | PAU11 (every variant) | E-API |
| `…::test_pau_logind_error_at_snapshot_never_unlocks` | PAU11 (every variant) | E-API |
| `…::test_pau_unlock_call_error_is_reported_as_unlock_failed` | PAU11 (every variant) | E-API |
| `…::test_pau_expired_allow_is_discarded` | PAU11 (`MAX_ALLOW_TO_UNLOCK_MS`) | E-API |
| `…::test_pau_clock_failure_skips_the_tick` | PAU11 | E-API |
| `…::test_pau_clock_failure_after_allow_never_unlocks` | PAU11 | E-API |
| `…::test_pau_inference_failures_never_unlock` | PAU11 (panic, `VisionError::Inference`) | E-API |
| `…::test_pau_camera_not_ready_never_unlocks` | PAU11 | E-API |
| `…::test_pau_interactive_demand_skips_the_presence_tick` | PAU12 | E-API |
| `…::test_pau_interactive_request_preempts_a_running_scan` | PAU12 | E-API |
| `…::test_pau_camera_stays_in_standby_without_an_eligible_scan` | PAU13 | E-API |
| `…::test_pau_too_many_seat_sessions_skip_the_tick` | PAU14 | E-API |
| `…::test_pau_hung_snapshot_call_is_bounded` | PAU14 | E-API |
| `…::test_pau_hung_scan_calls_are_bounded` | PAU14 | E-API |
| `…::test_pau_logind_failure_backoff_doubles_and_resets` | PAU14 | E-API |
| `…::test_pau_ignored_unlock_is_not_retried_until_the_next_lock_period` | PAU15 | E-API |
| `…::test_pau_presence_keeps_the_pam_reserve` | D10 / PAU3 | E-API |
| `…::test_pau_presence_never_scans_when_the_budget_is_the_reserve` | D10 | E-API |
| `…::test_pau_refused_account_before_the_scan_costs_nothing` | PAU28 (every refusal) | E-API |
| `…::test_pau_account_is_rechecked_after_the_allow` | PAU28 | E-API |
| `…::test_pau_slow_account_guard_is_undeterminable` | PAU28 | E-API |
| `…::test_pau_changed_owner_name_at_recheck_never_unlocks` | PAU28 | E-API |
| `…::test_pau_session_without_owner_name_is_never_unlocked` | PAU27 | E-API |
| `…::test_pau_root_session_is_never_scanned_or_unlocked` | PAU27 | E-API |
| `…::test_pau_run_stops_on_shutdown_and_never_unlocks_after_stop` | PAU19 | E-API |
| `crates/daemon/tests/presence_logging_tests.rs::test_pau_skip_reason_codes_are_stable_snake_case` | PAU16 | E-API |
| `…::test_pau_unlock_logs_one_info_line_without_biometric_values` | PAU16 | E-API |
| `…::test_pau_spoof_veto_is_warned_without_biometric_values` | PAU16 / D7 | E-API |
| `…::test_pau_repeated_skip_reason_is_logged_once` | PAU16 | E-API |
| `…::test_pau_missing_system_bus_warns_once_per_transition` | PAU19 / PAU14 | E-API |
| `…::test_pau_gate_transitions_are_logged_at_info_once` | F6 / PAU9 | E-API |
| `crates/daemon/tests/presence_account_tests.rs::test_pau_user_name_parse_table` | PAU27 | E-API |
| `…::test_pau_root_account_is_refused_first` | PAU27 | E-API |
| `…::test_pau_healthy_account_is_usable` | PAU22–26 nominal | E-API |
| `…::test_pau_realtime_failure_is_undeterminable` | §2.6 step 2 | E-API |
| `…::test_pau_guard_never_writes_its_sources` | PAU29 | E-API |
| `…::test_pau_faillock_defaults` | PAU22 | E-API |
| `…::test_pau_faillock_conf_grammar` | PAU22 | E-API |
| `…::test_pau_root_unlock_time_defaults_to_unlock_time` | PAU22 | E-API |
| `…::test_pau_faillock_time_bound_is_accepted` | PAU22 | E-API |
| `…::test_pau_faillock_conf_invalid_inputs_are_undeterminable` | PAU22 | E-API |
| `…::test_pau_etc_faillock_conf_wins_over_vendor` | PAU22 | E-API |
| `…::test_pau_vendor_conf_is_evaluated_with_the_defaults_strictest_wins` | PAU22 (F9) | E-API |
| `…::test_pau_absent_conf_files_use_the_defaults` | PAU22 | E-API |
| `…::test_pau_unreadable_or_invalid_conf_is_undeterminable` | PAU22 | E-API |
| `…::test_pau_tally_records_decode_like_struct_tally` | PAU23 | E-API |
| `…::test_pau_tally_size_bounds` | PAU23 | E-API |
| `…::test_pau_missing_tally_file_is_zero_records` | PAU23 | E-API |
| `…::test_pau_regular_tally_file_is_read` | PAU23 | E-API |
| `…::test_pau_non_regular_tally_paths_are_undeterminable` | PAU23 (symlink, FIFO, dir, size) | E-API |
| `…::test_pau_permission_denied_tally_is_undeterminable` | PAU23 (`EACCES`; returns early when run as root) | E-API |
| `…::test_pau_exclusively_locked_tally_is_undeterminable` | PAU23 (`flock`) | E-API |
| `…::test_pau_guard_refuses_a_symlinked_tally` | PAU23 | E-API |
| `…::test_pau_faillock_threshold` | PAU24 | E-API |
| `…::test_pau_faillock_deny_zero_never_locks` | PAU24 | E-API |
| `…::test_pau_faillock_fail_interval_boundary` | PAU24 | E-API |
| `…::test_pau_faillock_unlock_time_boundary` | PAU24 | E-API |
| `…::test_pau_faillock_unlock_time_zero_is_permanent` | PAU24 | E-API |
| `…::test_pau_faillock_invalid_status_records_are_ignored` | PAU24 | E-API |
| `…::test_pau_faillock_future_dated_records_count` | PAU24 | E-API |
| `…::test_pau_faillock_overflow_is_locked` | PAU24 | E-API |
| `…::test_pau_faillock_admin_group_uses_the_stricter_unlock_time` | PAU24 | E-API |
| `…::prop_pau_faillock_denies_matches_check_tally` | PAU24 (reference port) | E-API |
| `…::prop_pau_decode_tally_never_panics` | PAU23 | E-API |
| `…::prop_pau_parse_faillock_conf_never_panics` | PAU22 | E-API |
| `…::prop_pau_scan_pam_options_never_panics` | PAU25 | E-API |
| `…::test_pau_pam_lines_with_faillock_policy_options_are_detected` | PAU25 (F8 syntaxes) | E-API |
| `…::test_pau_pam_lines_without_policy_options_are_ignored` | PAU25 | E-API |
| `…::test_pau_guard_scans_every_pam_directory` | PAU25 | E-API |
| `…::test_pau_guard_skips_missing_pam_directories_and_subdirectories` | PAU25 | E-API |
| `…::test_pau_guard_pam_scan_bounds_are_undeterminable` | PAU25 | E-API |
| `…::test_pau_shadow_account_expiry` | PAU26 | E-API |
| `…::test_pau_shadow_forced_password_change` | PAU26 | E-API |
| `…::test_pau_shadow_password_expiry_and_inactivity` | PAU26 | E-API |
| `…::test_pau_shadow_locked_password` | PAU26 | E-API |
| `…::test_pau_shadow_empty_fields_are_not_set` | PAU26 | E-API |
| `…::test_pau_shadow_malformed_inputs_are_undeterminable` | PAU26 | E-API |
| `…::test_pau_shadow_hash_never_leaves_the_parser` | PAU26 | E-API |
| `…::test_pau_guard_shadow_refusals` | PAU26 | E-API |
| `…::test_pau_guard_faillock_and_shadow_order` | PAU24 / PAU26 / §2.6 order | E-API |
| `tests/invariants/src/presence_unlock_contract.rs::test_pau_zbus_is_declared_once_in_the_workspace` | C1 / PAU17 | passes (Cargo scaffolding added in Phase 2, see below) |
| `…::test_pau_zbus_is_used_only_by_the_daemon` | C1 / PAU17 | passes (scaffolding) |
| `…::test_pau_lockfile_pins_zbus_5_without_libdbus` | PAU17 | passes (scaffolding) |
| `…::test_pau_new_modules_forbid_unsafe_code` | C2 | A |
| `…::test_pau_new_modules_never_unwrap_or_expect` | C2 | A |
| `…::test_pau_presence_connects_only_to_the_pinned_system_bus` | C2 / PAU17 | A |
| `…::test_pau_unlock_session_has_exactly_one_production_call_site` | C3 / PAU11 | A |
| `…::test_pau_pad_consensus_is_built_only_in_consensus_rs` | C4 / PAU10 / PAU12 | A |
| `…::test_pau_rate_limit_default_is_forty_and_documented` | C5 / PAU2 | A |
| `…::test_pau_kill_switch_dir_equals_the_pam_flag_dir` | C6 | A |
| `…::test_pau_adr_records_the_owner_decisions_and_the_account_guard` | C7 / PAU20 / PAU29 | A |
| `…::test_pau_architecture_states_invariant_six` | C7 / PAU20 | A |
| `…::test_pau_operator_docs_describe_presence` | PAU20 / PAU29 | A |
| `…::test_pau_presence_logs_no_biometric_field_above_debug` | PAU16 | A |
| `…::test_pau_main_spawns_presence_after_ready_and_stops_it_at_shutdown` | PAU19 | A |
| `…::test_pau_presence_code_never_writes` | C8 / PAU29 / PAU23 flags | A |
| `…::test_pau_unit_keeps_the_account_guard_sandbox` | C9 / PAU29 | passes (regression guard: the unit already satisfies it and must not change) |

### Red evidence (observed 2026-10-02, toolchain 1.98.1)

- `cargo test --locked -p soos-policy --all-features --test rate_limit_reserve_tests` fails with 20 errors, all on the missing API: 17× `E0599 no method named check_and_record_with_reserve found for struct RateLimiter`, 3× `E0599 no method named record_attempt_with_reserve found for struct AuthorizationEngine`.
- `cargo build --locked -p soos-daemon --all-features --tests --keep-going`: all 9 new daemon test targets fail to compile, and only on the specified API: 24× `E0433 cannot find presence in soos_daemon`, 13× `E0609 no field presence on type DaemonConfig`, 6× `E0432 unresolved import soos_daemon::presence`, 1× `E0560 struct DaemonConfig has no field named presence`, 3× `E0432` on `InferencePriority` / `InteractiveDemandGuard`, 1× `E0432 unresolved import soos_daemon::consensus`. No other error. Every existing daemon test target still compiles.
- `cargo test --locked -p soos-invariants --all-features`: 381 passed, 14 failed, all on assertions:
  - `daemon_docs_contract::test_daemon_doc_documents_every_daemon_toml_key` (migrated): "Expected at least 10 *ConfigFile structs in crates/daemon/src/config.rs, found 9"
  - `test_pau_adr_records_the_owner_decisions_and_the_account_guard`: "AI/DECISIONS.md must contain the ADR \"Presence Auto-Unlock Through logind\""
  - `test_pau_architecture_states_invariant_six`: "AI/ARCHITECTURE.md §2 \"Mandatory Security Invariants\" must list invariant 6"
  - `test_pau_main_spawns_presence_after_ready_and_stops_it_at_shutdown`: "main.rs builds the presence worker"
  - `test_pau_operator_docs_describe_presence`: "Docs/DAEMON.md must document `[presence]`"
  - `test_pau_rate_limit_default_is_forty_and_documented`: "RateLimitConfig::DEFAULT_MAX_ATTEMPTS must be 40"
  - `test_pau_pad_consensus_is_built_only_in_consensus_rs`: "PadAggregator::with_defaults only in consensus.rs"
  - `test_pau_unlock_session_has_exactly_one_production_call_site`: "exactly one production `.unlock_session(` call, in presence/worker.rs"
  - `test_pau_new_modules_never_unwrap_or_expect`, `test_pau_presence_connects_only_to_the_pinned_system_bus`, `test_pau_presence_logs_no_biometric_field_above_debug`: "crates/daemon/src/presence must hold mod, config, logind, display, switch, tracker, account and worker (found 0)"
  - `test_pau_new_modules_forbid_unsafe_code`, `test_pau_kill_switch_dir_equals_the_pam_flag_dir`, `test_pau_presence_code_never_writes`: "Cannot read crates/daemon/src/presence/{mod,account}.rs"

### Validation against a throwaway reference implementation

The tests are immutable contracts, so they were checked against a reference implementation before hand-off. It was written in scratch space only: a temporary overlay of `consensus.rs`, `presence/**`, the gate demand counter, the reserve call, the `[presence]` parsing and one `register_interactive()` line in the dispatcher. The overlay has been removed: no production file differs from `f76a80b`. Results:

- with the reference, every contract test passes: policy 13/13, config 16/16, tracker 13/13, display 13/13, mapping 9/9, priority 10/10, consensus 12/12, worker 40/40, logging 6/6, account 49/49;
- with signature-only stubs that return wrong values, the tests fail on assertions (e.g. policy 11/13 failing);
- `cargo clippy --locked -p soos-daemon -p soos-policy -p soos-invariants --all-targets --all-features -- -D warnings` is clean on the test code, and `cargo fmt --all -- --check` is clean.

The invariant tests about docs, `main.rs` wiring and module layout stay red under the reference overlay. They are satisfied only by the Phase 4 / Phase 6 deliverables.

### Migrated existing tests

| Test | Old assertion | New assertion | Mandating acceptance line |
|---|---|---|---|
| `tests/invariants/src/daemon_docs_contract.rs::test_daemon_doc_documents_every_daemon_toml_key` | `CONFIG_FILE_TABLES: [(&str, &str); 9]` (no `PresenceConfigFile`) | `[(&str, &str); 10]` with `("PresenceConfigFile", "presence")` appended. No assertion, loop or message changed; the test is stricter: it now also requires `[presence]` and its keys in `Docs/DAEMON.md` | Spec §1.1 row *[R2: F1]*, PAU20 |

No other existing test was touched.

### Phase 2 scaffolding (no production logic)

- `Cargo.toml` `[workspace.dependencies]`: `zbus = { version = "5", default-features = false, features = ["tokio"] }` (spec §1.1). `Cargo.lock` gains exactly the 22 crates of spec §0 (resolved offline; zbus 5.19.0, zvariant 5.15.0).
- `crates/daemon/Cargo.toml`: `zbus = { workspace = true }` in `[dependencies]` (needed by the PAU18 tests, which build `zbus::zvariant::OwnedValue` maps), and `proptest = "1"` in `[dev-dependencies]` (PAU24 / PAU22 / PAU23 / PAU25 properties).
- `tests/invariants/src/lib.rs`: `mod presence_unlock_contract;` (`cfg(all(test, unix))`).

### Flakiness check

The timing-sensitive targets `presence_worker_tests`, `presence_logging_tests`, `presence_consensus_tests` and `inference_priority_tests` were run 10 times in a row against the reference overlay: 10/10 green, with stable durations (worker 4.4 s, logging 3.2 s, consensus 2.1 s, priority 2.1 s). No wall-clock assertion is tighter than a product budget. Hung-call bounds allow `DBUS_CALL_TIMEOUT_MS + 1 s`. The PAM deadline test asserts the 1000 ms client budget. The backoff tests shift both the test clock and real time, so they hold whichever clock the backoff window uses.

### Spec ambiguities resolved (binding for the developer)

1. **Pure account helpers.** The spec names `parse_faillock_conf`, `decode_tally_records`, `read_tally_records`, `faillock_denies`, `scan_pam_faillock_options`, `find_shadow_entry` and `shadow_refusal` without signatures. The signatures above are the contract. `None` / `true` mean `Undeterminable`.
2. **`SystemAccountGuard` injection.** The guard uses a builder (`new()` plus `with_*`). It adds `with_default_faillock_dir`, which replaces `DEFAULT_FAILLOCK_DIR` for the built-in defaults and for any parsed policy whose `dir` is still `DEFAULT_FAILLOCK_DIR`. Without it, the "no `/etc` conf" readings (F9) would touch the real `/run/faillock`.
3. **`FaillockPolicy::root_unlock_time`** is a resolved `u32`: the parser copies `unlock_time` into it when the key is absent, whatever the key order.
4. **A PAM directory path that exists but is not a directory** (`ENOTDIR`) counts as "unreadable" and gives `Undeterminable`. Sub-directories inside a PAM directory are skipped (only regular files are scanned).
5. **Backoff** is pinned through a pure `presence::reconnect_backoff(consecutive_failures) -> Duration`, plus a worker test that shifts both clocks. The window may be measured on either clock.
6. **`PresenceLogindError::call(&str)`** is the truncating constructor (PAU14 "Call text ≤ 256 bytes"). `ZbusLogind` must build every `Call` through it.
7. **`SkipReason::as_str`** codes are the snake_case variant names. The transition-only debug log line must contain that code (PAU16).
8. **Step-7 skip reason for a single dropped candidate.** The tests accept either the specific reason (`NotEnrolled`, `ForeignTemplate`, `TemplateStoreError`, `AccountRefused`) or `NoCandidate`. Both are allowed by spec step 7. In every case they assert that no attempt, camera wake, inference or unlock happened.
9. **Re-check errors.** Any `session_state` error or timeout ends the scan with `SessionChanged`. A clock failure after the `Allow` may end with any non-`Unlocked` outcome. A hung `lid_closed` is a gating error, so the scan proceeds (spec §2.6 "Lid … `Err(_)` ⇒ scan allowed").
10. **A `Name` that disappears or changes between step 7 and the re-check** gives `SessionChanged` (spec step 13, "its `user_name` must equal the one checked in step 7").
11. **Unlock log fields.** The unlock `info!` line must carry a field named `session_id` and `uid` (`uid=1000`). Gate transitions must use the field names `lid_closed` and `display_state`, logged once per transition. Logind unavailability must be logged at `warn`, once per transition.
12. **Wording the invariants pin** (Phase 6 must use these words): the ADR line contains `Presence Auto-Unlock Through logind` and, case-insensitively, `default-on`, `40`, `sudo`, `LED`, `presentation`, `pam_faillock`, `/etc/shadow`, `no tally reset`, `UnlockSession`, `zbus`. `Docs/DAEMON.md` contains `[presence]`, `` `scan_interval_ms` ``, `` `lock_grace_ms` ``, `presence.disable`, `/etc/soos/disabled`, `gdm.disable`, `pam_faillock`, `faillock.conf`, `/etc/shadow`, `no tally reset`, and its `max_attempts` table row says `` `40` ``. `Docs/POLICY_CRATE.md` has a line with `40` and "attempts", and names `check_and_record_with_reserve`. `Docs/DISTRIBUTION_DEPLOYMENT.md` names `presence.disable`, `swayidle`, `SetLockedHint`, `GNOME` and `KDE`. `AI/MOCK_STRATEGY.md` names `MockPresenceLogind`. `Docs/SECURITY_AND_QUALITY_GUIDELINES.md` and `AI/ARCHITECTURE.md` name `zbus`. `Docs/PACKAGING_AND_PROVISIONING.md` names `CAP_DAC_OVERRIDE`, `system bus` and `/run/faillock`. Invariant 6 is a line starting with `6. ` in the "### Mandatory Security Invariants" list, and it contains `logind`, `Allow`, `grace`, `REMOTE=0`, `pam_faillock` and `locked`.
13. **C2 `.unwrap()` / `.expect(`** applies to the whole of every file in `presence/**` and `consensus.rs`, `#[cfg(test)]` modules included (spec wording). Unit tests belong in `crates/daemon/tests/`.
14. **C3** counts `.unlock_session(` in production code (before the first `#[cfg(test)]`). `ZbusLogind` must therefore call the D-Bus method by its name, `"UnlockSession"` (which the invariant requires), and must not call a generated `.unlock_session(` proxy method.
15. **C8 forbidden write APIs** in `presence/**`: `.write(true)`, `.append(true)`, `.create(true)`, `.create_new(true)`, `.truncate(true)`, `fs::write(`, `File::create(`, `--reset`, `remove_file(`, `set_permissions(`, `O_WRONLY`, `O_RDWR`, `O_TRUNC`, `O_CREAT`. `account.rs` must use `O_NOFOLLOW`, `O_NONBLOCK`, `O_CLOEXEC`, `LockSharedNonblock` and `Zeroizing`.
16. **`main.rs` wiring (PAU19)** is pinned statically. `PresenceWorker::new(` must appear after `notify_ready()`. The file must use `ZbusLogind::new()`, `SysfsDisplayProbe::new(` with `DEFAULT_DRM_SYSFS_DIR`, `PresenceSwitch::new(` with `DEFAULT_KILL_SWITCH_DIR`, `SystemAccountGuard::new()`, `presence.enabled` and `enforce_active_session`. After `accept_until_shutdown(`, it must call `.send(true)` and `.abort()`. The systemd acceptance script (`tests/docker/systemd_unit_acceptance_test.sh`) is the existing Docker evidence for READY ordering and clean stop, and must keep passing unchanged.
17. **`ConsensusRun` field types.** `frames_evaluated` and `consecutive_passing` are `usize` (spec). `PadAggregator` returns `u32`, so the consensus converts.

## Round 2 (auditor T1–T8)

Input: `AI/auditor_constraints_presence_unlock.md` (Phase 3, `BLOCKED` via T1–T5). This round only adds tests and tightens assertions; no assertion was weakened or removed. The total is now **209 contract tests**: 11 new, plus the existing tests that were tightened. No production file differs from `f76a80b`.

### New tests

| Test (path::name) | Finding / constraint | Red |
|---|---|---|
| `crates/daemon/tests/presence_account_tests.rs::test_pau_guard_follows_symlinked_pam_stack_files` | T1 / constraint 12 / A5 | E-API |
| `…::test_pau_guard_refuses_dangling_or_special_pam_symlinks` | T1 (dangling, symlink to FIFO ⇒ `Undeterminable`; symlink to a directory is skipped) | E-API |
| `…::test_pau_guard_refuses_an_unsafe_tally_directory` | constraint 13 (symlinked dir; modes 0777/0775/0757/1777 when not root-owned; `dir=` that is a regular file) | E-API |
| `…::test_pau_tally_under_a_regular_file_is_undeterminable_even_as_root` | T8 (root-proof `ENOTDIR`) | E-API |
| `…::test_pau_guard_rereads_every_source_on_every_check` | T2 (one guard instance; tally, shadow, PAM stack and `faillock.conf` each changed in both directions) | E-API |
| `crates/daemon/tests/presence_worker_tests.rs::test_pau_recheck_is_bound_to_the_candidate_session_id` | T3 / constraint 22 | E-API |
| `…::test_pau_session_swap_during_the_scan_never_unlocks` | T4 (candidate replaced by session 5; UID changed; name changed; unlocked; turned remote; all sessions gone) | E-API |
| `…::test_pau_interactive_demand_at_camera_wake_preempts_before_inference` | T8 | E-API |
| `crates/daemon/tests/presence_logging_tests.rs::test_pau_account_guard_never_logs_its_sources` | T6 / constraints 18–20 (hash, tally source and conf/PAM content at no level; user name not above debug) | E-API |
| `crates/policy/tests/rate_limit_reserve_tests.rs::prop_pau_reserve_holds_across_window_expiry` | T8 (gaps up to 40 s cross the 60 s window) | E-API |
| `tests/invariants/src/presence_unlock_contract.rs::test_pau_production_part_only_strips_a_trailing_test_module` | T5 (self-test of the helper) | passes (pure helper) |

### Tightened existing tests

| Test | Change | Finding |
|---|---|---|
| `presence_worker_tests::test_pau_worker_unlocks_an_eligible_session_after_the_grace` | + `state_requests() == ["2"]` | T3 |
| `presence_worker_tests::test_pau_hung_snapshot_call_is_bounded` | bound `DBUS_CALL_TIMEOUT_MS + 1000` → `+ 700` ms | T8 |
| `presence_worker_tests::test_pau_hung_scan_calls_are_bounded` | + each hung re-check / unlock call is cut within `DBUS_CALL_TIMEOUT_MS + 700` ms, measured from the call; + hung unlock: `unlocked_ids == ["2"]` (one attempt, no retry) | T8 |
| `presence_worker_tests::test_pau_presence_keeps_the_pam_reserve` | + `unlock_calls == 0` | T8 |
| `presence_logging_tests::test_pau_spoof_veto_is_warned_without_biometric_values` | + `unlock_calls == 0`; + no user name above debug | T8 / constraint 20 |
| `presence_logging_tests::test_pau_unlock_logs_one_info_line_without_biometric_values` | + no user name above debug | constraint 20 |
| `presence_unlock_contract::test_pau_unlock_session_has_exactly_one_production_call_site` | `production_part` now strips only a **trailing** `#[cfg(test)] mod {…}` block. + `unlock_session(` counted in every syntax (method, UFCS, fully qualified, turbofish), except `fn unlock_session` declarations. + exactly one `"UnlockSession"` literal in production code, in `presence/logind.rs`. + forbidden anywhere in `crates/daemon/src`: `"UnlockSessions"`, `"Unlock"`, `"LockSession"`, `"LockSessions"`, `"SetLockedHint"`, `"TerminateSession"`, `"KillSession"`, `"ActivateSession"`, `loginctl`, `unlock-session` | T5 |
| `presence_unlock_contract::test_pau_new_modules_never_unwrap_or_expect` | + no `panic!(`/`unreachable!(`/`todo!(`/`unimplemented!(`; + no `allow(` of `clippy::{panic,indexing_slicing,arithmetic_side_effects,unwrap_used,expect_used}` in `presence/**` and `consensus.rs` | T7 / constraint 1 |
| `presence_unlock_contract::test_pau_presence_connects_only_to_the_pinned_system_bus` | + forbidden in `presence/**`: `::system(`, `::session(`, `env::var`, `Address::system`, `zbus::blocking`, `request_name`, `serve_at`, `object_server`, `#[proxy`, `zbus::proxy`, `#[interface`, `MessageStream`, `SignalStream`, `CacheProperties::Yes`/`Lazily`. + `Builder::system`/`session` forbidden across `crates/daemon/src` | T7 / constraints 4–6 |
| `presence_unlock_contract::test_pau_presence_code_never_writes` | + `rename(`, `fs::copy(`, `create_dir`, `remove_dir`, `hard_link`, `Command::new`, `LockExclusive` | T7 / constraints 14–15 |
| `presence_unlock_contract::test_pau_presence_logs_no_biometric_field_above_debug` | + inline format captures (`{score…`, `{embedding…`, …) forbidden. + in `presence/**`, the field keys `name`/`user_name`/`username`/`user`/`login` and inline `{name`/`{user_name`/`{user}` are forbidden in `info!`/`warn!`/`error!` | T7 / constraint 20 |

Test-double and setup changes (no assertion touched):
- `MockPresenceLogind` records the session ID of every `session_state` call (`state_requests()`); a queued reply may carry any ID.
- `presence_account_tests::Accounts::new` sets both tally directories to `0755`, so the constraint-13 rule does not depend on the runner's umask.

### Red evidence (round 2)

- `soos-policy`: 21× `E0599` (`check_and_record_with_reserve` 18, `record_attempt_with_reserve` 3). Only the specified API is missing.
- `soos-daemon --tests --keep-going`: the same 9 targets fail, only on the specified API: 25× `E0433 presence`, 13× `E0609 DaemonConfig::presence`, 6× `E0432 soos_daemon::presence`, 1× `E0560`, 3× `E0432` `InferencePriority` / `InteractiveDemandGuard`, 1× `E0432 soos_daemon::consensus`.
- `soos-invariants`: 382 passed, 14 failed. The failures are the same assertions as in round 1, plus the new C3/C2/C8 conditions.

### Fail-open variants proven to fail (on the adapted reference overlay)

| Deliberate defect in the reference | Failing new test(s) |
|---|---|
| PAM scan classifies entries with `symlink_metadata` and skips anything that is not a regular file | `test_pau_guard_follows_symlinked_pam_stack_files`, `test_pau_guard_refuses_dangling_or_special_pam_symlinks` |
| Guard memoises its first result | `test_pau_guard_rereads_every_source_on_every_check` |
| Tally-directory check removed | `test_pau_guard_refuses_an_unsafe_tally_directory` |
| Re-check does not compare the returned `id` with the candidate | `test_pau_recheck_is_bound_to_the_candidate_session_id` |
| Re-check reuses the step-4 snapshot instead of a fresh `session_state` | `test_pau_session_swap_during_the_scan_never_unlocks` |
| A second call via UFCS (`PresenceLogind::unlock_session(l, id)`) placed after a non-module `#[cfg(test)]` | `test_pau_unlock_session_has_exactly_one_production_call_site` ("exactly one production `unlock_session(` call in any syntax") |
| A second `"UnlockSession"` literal | same test ("exactly one production \"UnlockSession\" method literal") |
| A `"Unlock"` method literal | same test ("logind method literal \"Unlock\" is forbidden") |

The compliant reference overlay passes every new and tightened runtime test: account 54/54, worker 43/43, logging 7/7, policy 14/14. With one `"UnlockSession"` literal in `logind.rs`, it also passes the new C3 test. The other new invariant conditions raised no false positive on the reference. Clippy `-D warnings` is clean on all test code, and `cargo fmt --check` is clean. Flakiness: worker, logging and account ran 10× in a row: 10/10 green.

### Reference-overlay adaptations (scratch only, removed)

- PAM scan uses `fs::metadata` (follows symlinks): a directory is skipped; anything else that is not a regular file, or dangling, gives `Undeterminable`.
- The tally directory is checked with `symlink_metadata`: it must be a real directory with `mode & 0o022 == 0`, except a root-owned sticky directory.
- The worker re-check requires `fresh.id == candidate`.

### Open point for the coordinator / architect (not encoded)

The coordinator's T4 wording also lists "a second eligible session appears during the scan ⇒ no unlock". The spec (D5 at selection, step 13 re-check of the candidate only) and the auditor's T4 do not require it. Enforcing it needs a second `seat_sessions` snapshot before `UnlockSession`, which is a new behaviour, so I did not add it to the contract. If the architect adopts it, it becomes one additional worker test.

## Round 3 (auditor T9, spec revision 4)

Spec revision 4 adds two things:
- the outcome `ScanOutcome::KillSwitchEngaged`;
- a PAU28 clause: the kill switch (`/etc/soos/disabled`, `/etc/soos/presence.disable`, or a stat error on either) is checked a second time in the step-13 re-check, immediately before `UnlockSession`. If it is engaged, nothing is unlocked, the outcome is `KillSwitchEngaged` and the tracker is cleared.

The contract now has **210 tests**.

| Test (path::name) | Criterion | Red |
|---|---|---|
| `crates/daemon/tests/presence_worker_tests.rs::test_pau_kill_switch_engaged_before_unlock_never_unlocks` | T9 / PAU28 rev 4 | E-API (missing `presence` module; `ScanOutcome::KillSwitchEngaged` is new specified API) |

What the test checks, for each of three variants (`disabled`, `presence.disable`, and a stat error where the flag directory becomes a regular file, so every stat fails with `ENOTDIR`):
1. The switch is engaged inside the `session_state` hook, i.e. after the consensus `Allow` and before `UnlockSession`. The tick must return `Scanned(KillSwitchEngaged)`, with `unlock_calls == 0`, no unlocked ID and exactly one re-check.
2. Three further ticks while the switch stays engaged must each return `Skipped(KillSwitch)`. There must be no new camera wake, no logind call and no unlock (no retry).
3. Once the switch is removed, the next tick must return `Skipped(InGrace)`: the tracker was cleared, so the grace restarts.

Validation:
- The reference overlay with the rev-4 re-check passes it (worker 44/44, 10/10 consecutive runs).
- A fail-open variant that does not re-check the switch before unlocking fails it.
- Clippy and fmt are clean.
- The red state is unchanged: compile errors only on the specified API. No production code was changed.

## Round 4 (candid review `CHANGES_REQUESTED`)

This round adds tests only. All the new runtime tests are in a new file, `crates/daemon/tests/presence_candid_review_tests.rs`, so the existing contract files keep compiling against the current implementation. No existing test was edited. The contract now has **220 tests** (9 runtime tests and 1 invariant added).

| Test (path::name) | Finding | Red on the current implementation |
|---|---|---|
| `presence_candid_review_tests::test_pau_bracketed_faillock_arguments_are_detected` | MAJOR §4-1, PAU25 (`[deny=2]`, `[dir=/x y]`, every option, continuation) | A: "`auth [default=die] pam_faillock.so authfail [deny=2]` sets faillock policy …" |
| `…::test_pau_bracketed_control_field_is_not_an_argument` | MAJOR §4-1 (no false positive: `[success=1 default=bad]` control, `[deny=1]` on other modules) | passes (regression guard) |
| `…::test_pau_guard_detects_bracketed_faillock_arguments` | MAJOR §4-1, guard level | A: `Usable` instead of `Refused(Undeterminable)` |
| `…::test_pau_shadow_star_and_bang_markers_are_locked` | MINOR §4-3, PAU26 (`*LK*`, `*NP*`, `*`, `**`, `*$6$hash`, `!`, `!$6$hash`, `!*LK*`; plus guard level) | A: "`*LK*` is a locked marker" |
| `…::test_pau_suspend_between_allow_and_unlock_discards_the_allow` | MINOR §4-2 (the boot clock jumps past `MAX_ALLOW_TO_UNLOCK_MS` while the monotonic clock does not) | E-API (`with_boot_clock_fn`); with a no-op stub, A: "suspend time counts against MAX_ALLOW_TO_UNLOCK_MS" |
| `…::test_pau_boot_clock_without_suspend_still_unlocks` | MINOR §4-2 (no false refusal) | E-API |
| `…::test_pau_boot_clock_failure_after_allow_never_unlocks` | MINOR §4-2 (boot clock error ⇒ `AllowExpired`) | E-API; with a stub, A |
| `…::test_pau_lid_closed_at_recheck_never_unlocks` | MINOR §4-2 (lid closed after the gate ⇒ `ScanOutcome::LidClosed`; lid read at least twice) | E-API (`ScanOutcome::LidClosed`); with a stub, A |
| `…::test_pau_lid_error_at_recheck_does_not_refuse` | MINOR §4-2 (only `Ok(true)` refuses, same rule as the gate) | E-API |
| `tests/invariants/src/presence_unlock_contract.rs::test_pau_allow_window_uses_the_boot_clock` | MINOR §4-2 (static) | A: "presence/worker.rs (or the presence clock helper in mod.rs) must read CLOCK_BOOTTIME" |

The existing `presence_worker_tests::test_pau_expired_allow_is_discarded` already covers a monotonic-clock jump between the `Allow` and the unlock. It must keep passing.

### Binding API and behaviour (developer)

- **Boot clock.** `PresenceWorker::with_boot_clock_fn(self, fn() -> Result<u64, DaemonError>) -> Self` is a test hook; production uses `current_monotonic_nanos_from_clock(ClockId::CLOCK_BOOTTIME)`.
- **Allow window.** The `Allow`-to-unlock window is checked on **both** clocks: `clock_fn` (monotonic) and the boot clock. Exceeding `MAX_ALLOW_TO_UNLOCK_MS` on either, or any error reading the boot clock, gives `AllowExpired`. Checking both keeps `test_pau_expired_allow_is_discarded` valid.
- **No clock overrides in production.** `main.rs` calls neither `with_boot_clock_fn` nor `with_clock_fn` (invariant).
- **Lid re-check.** New outcome `ScanOutcome::LidClosed`. No existing outcome fits: `SessionChanged` is about the session record, and `SkipReason::LidClosed` is a skip, not a scan outcome. In step 13 the lid is read again after the `Allow`, before `UnlockSession`. Only `Ok(true)` refuses; an error or timeout does not (same rule as the gate). The existing hung-lid test (`test_pau_hung_scan_calls_are_bounded`, case "lid_closed" ⇒ `Unlocked`) still passes with two lid timeouts.
- **Bracketed arguments.** Every argument token after the module token is normalised by stripping leading `[` (and a trailing `]` for the bare flag `even_deny_root`) before matching.
- **Locked password markers.** A password field starting with `!` **or** `*` is `PasswordLocked`.

### Validation

1. **Red on the current implementation:**
   - The new test file fails to compile only on the new API: `E0599 with_boot_clock_fn`, and `ScanOutcome::LidClosed` not found.
   - With a temporary no-op stub of those two items, 6 of the 9 tests fail on assertions, each for the reason under review. The 3 that pass are regression guards (no false positives).
   - The new invariant fails on its CLOCK_BOOTTIME assertion.
2. **Green with a temporary fix:** I applied a scratch fix on top of the current implementation (bracket stripping, `*` prefix, boot clock checked on both clocks, lid re-check). Results:
   - round-4 tests 9/9, 10/10 consecutive runs;
   - existing worker 44/44, account 54/54, logging 7/7;
   - presence invariants 19/19.
3. **Clean-up:** the scratch changes were then removed; `crates/daemon/src/presence/{worker,account}.rs` are byte-identical to before (sha256 verified).
4. **Lint:** clippy `-D warnings` (test code, with a temporary signature stub) and `cargo fmt --check` are clean.
