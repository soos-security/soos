//! Pure and tempdir contract tests of the offline history and the back-online push of
//! `soos-remote` (GitHub #376, tier 1 of #359 Part B; ADR 2026-10-13 "Offline History and
//! Back-Online Push Notifications in `soos-remote`"; architect spec
//! `AI/architect_spec_remote_offline_alerts.md` §10.1–§10.4; matrix ROA3–ROA8).
//!
//! Every clock is a fake [`ClockSample`] built by the test (wall, `CLOCK_BOOTTIME` and
//! `CLOCK_MONOTONIC` in ms); no bus, no network, no real clock is read here. Store tests run
//! in a `TempDir`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::fs;
use std::os::unix::fs::{symlink, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use soos_remote::alerts::{Attempt, AttemptKind};
use soos_remote::auth::system_random;
use soos_remote::config::{
    parse_config, resolve_network_store_path, ConfigError, OfflineConfig, PushConfig, PushPreviews,
};
use soos_remote::http::Method;
use soos_remote::journal::{AccountClass, SourceClass};
use soos_remote::offline::{
    classify, duration_label, nm_reads_allowed, previous_run_interval, ClockSample,
    NetworkObservation, NetworkStore, NetworkStoreError, NetworkStoreFile, NmReading,
    OfflineInterval, OfflineNotice, OfflineReason, OfflineTracker, RunMarker,
};
use soos_remote::push::{back_online_payload, DueMessage, PushScheduler};
use soos_remote::routes::{allow_header, is_funnel_public, route, Route, OFFLINE_PATH};
use soos_remote::{
    DBUS_CALL_TIMEOUT_MS, DBUS_CONNECT_TIMEOUT_MS, MAX_CREDENTIALS_PATH_LEN,
    MAX_NETWORK_STORE_BYTES, MAX_OFFLINE_INTERVALS, MAX_PUSH_PLAINTEXT_BYTES,
    NETWORK_READ_DEADLINE_MS, NETWORK_STORE_FILE_NAME, OFFLINE_HEARTBEAT_MS,
    OFFLINE_MIN_INTERVAL_MS, OFFLINE_RETRY_DELAYS_MS, OFFLINE_SHUTDOWN_READ_MS,
    OFFLINE_STOP_DEADLINE_MS, OFFLINE_TICK_MS, OFFLINE_UNMONITORED_RELEASE_TICKS, PUSH_COALESCE_MS,
    PUSH_MAX_PER_HOUR, PUSH_MIN_INTERVAL_MS, PUSH_OFFLINE_TOPIC, PUSH_TEST_TOPIC, PUSH_TOPIC,
};

const BASE_UNIX_MS: u64 = 1_760_000_000_000;
const BOOT_OFFSET_MS: u64 = 40_000;
const MONO_OFFSET_MS: u64 = 7_000;
const HOUR_MS: u64 = 3_600_000;
const RP_ID: &str = "pc.tail1234.ts.net";
const RUNTIME_DIR: &str = "/run/user/1000";

// ---------------------------------------------------------------------------------------
// Fake clock helpers
// ---------------------------------------------------------------------------------------

/// An awake sample `t` ms after the test origin (all three clocks advance together).
fn at(t: u64) -> ClockSample {
    ClockSample {
        unix_ms: BASE_UNIX_MS + t,
        boot_ms: BOOT_OFFSET_MS + t,
        mono_ms: MONO_OFFSET_MS + t,
    }
}

/// A sample `t` ms of awake time after the origin, after `slept` ms of suspend in total
/// (wall and boot clocks include the suspend, the monotonic clock does not).
fn after_sleep(t: u64, slept: u64) -> ClockSample {
    ClockSample {
        unix_ms: BASE_UNIX_MS + t + slept,
        boot_ms: BOOT_OFFSET_MS + t + slept,
        mono_ms: MONO_OFFSET_MS + t,
    }
}

const ONLINE: NetworkObservation = NetworkObservation::Online;
const LOST: NetworkObservation = NetworkObservation::Offline(OfflineReason::NetworkLost);
const WIFI_OFF: NetworkObservation = NetworkObservation::Offline(OfflineReason::WifiDisabled);
const UNKNOWN: NetworkObservation = NetworkObservation::Unknown;

fn interval(reason: OfflineReason, start: u64, end: u64, duration: u64) -> OfflineInterval {
    OfflineInterval {
        reason,
        start_unix_ms: start,
        end_unix_ms: end,
        duration_ms: duration,
    }
}

fn reading(state: u32, connectivity: u32, wireless_enabled: Option<bool>) -> NmReading {
    NmReading {
        state,
        connectivity,
        wireless_enabled,
    }
}

fn started() -> OfflineTracker {
    let mut tracker = OfflineTracker::new(Vec::new());
    let outcome = tracker.start(None, at(0));
    assert!(outcome.recorded.is_empty() && outcome.notice.is_none());
    tracker
}

// ---------------------------------------------------------------------------------------
// Constants (spec §3)
// ---------------------------------------------------------------------------------------

/// Spec §3: every constant has the specified value and the compile-time relations hold.
#[test]
fn test_roa_constants_match_the_spec() {
    assert_eq!(NETWORK_STORE_FILE_NAME, "remote-network.json");
    assert_eq!(MAX_NETWORK_STORE_BYTES, 8192);
    assert_eq!(MAX_OFFLINE_INTERVALS, 32);
    assert_eq!(OFFLINE_TICK_MS, 5000);
    assert_eq!(OFFLINE_MIN_INTERVAL_MS, 30_000);
    assert_eq!(OFFLINE_HEARTBEAT_MS, 60_000);
    assert_eq!(NETWORK_READ_DEADLINE_MS, 3000);
    assert_eq!(OFFLINE_STOP_DEADLINE_MS, 3000);
    assert_eq!(OFFLINE_SHUTDOWN_READ_MS, 1500);
    assert_eq!(OFFLINE_UNMONITORED_RELEASE_TICKS, 12);
    assert_eq!(OFFLINE_RETRY_DELAYS_MS, [5_000, 30_000, 120_000, 600_000]);
    assert_eq!(PUSH_OFFLINE_TOPIC, "soosonline");
    assert_ne!(PUSH_OFFLINE_TOPIC, PUSH_TOPIC);
    assert_ne!(PUSH_OFFLINE_TOPIC, PUSH_TEST_TOPIC);
    assert!(PUSH_OFFLINE_TOPIC
        .bytes()
        .all(|b| b.is_ascii_alphanumeric()));
    const { assert!(NETWORK_READ_DEADLINE_MS < OFFLINE_TICK_MS) };
    const { assert!(OFFLINE_TICK_MS < OFFLINE_MIN_INTERVAL_MS) };
    const { assert!(OFFLINE_TICK_MS <= OFFLINE_HEARTBEAT_MS) };
    const { assert!(DBUS_CONNECT_TIMEOUT_MS + 4 * DBUS_CALL_TIMEOUT_MS <= NETWORK_READ_DEADLINE_MS) };
    const { assert!(DBUS_CONNECT_TIMEOUT_MS + DBUS_CALL_TIMEOUT_MS <= OFFLINE_SHUTDOWN_READ_MS) };
    const { assert!(OFFLINE_SHUTDOWN_READ_MS < OFFLINE_STOP_DEADLINE_MS) };
}

// ---------------------------------------------------------------------------------------
// Test 1 — classification (D-4)
// ---------------------------------------------------------------------------------------

/// Test 1 (ROA3, D-4): NetworkManager states 10–40 are offline; 50–70 are online unless the
/// connectivity is `NONE`; `PORTAL` and `LIMITED` are online; unknown states are `Unknown`;
/// `WirelessEnabled = false` names the reason.
#[test]
fn test_roa_classify_states_and_connectivity() {
    // Connected states: online for every connectivity but NONE (1).
    for state in [50, 60, 70] {
        for connectivity in [0, 2, 3, 4, 9, u32::MAX] {
            for wireless in [Some(true), Some(false), None] {
                assert_eq!(
                    classify(&reading(state, connectivity, wireless)),
                    ONLINE,
                    "state {state} connectivity {connectivity} wireless {wireless:?}"
                );
            }
        }
        assert_eq!(classify(&reading(state, 1, Some(true))), LOST, "{state}");
        assert_eq!(classify(&reading(state, 1, None)), LOST, "{state}");
        assert_eq!(
            classify(&reading(state, 1, Some(false))),
            WIFI_OFF,
            "{state}"
        );
    }
    // PORTAL and LIMITED are never offline (issue risk list).
    assert_eq!(classify(&reading(70, 2, Some(true))), ONLINE);
    assert_eq!(classify(&reading(70, 3, Some(true))), ONLINE);
    assert_eq!(classify(&reading(60, 3, Some(false))), ONLINE);
    // Asleep, disconnected, disconnecting, connecting: offline whatever the connectivity.
    for state in [10, 20, 30, 40] {
        for connectivity in [0, 1, 2, 3, 4] {
            assert_eq!(
                classify(&reading(state, connectivity, Some(true))),
                LOST,
                "{state}/{connectivity}"
            );
            assert_eq!(
                classify(&reading(state, connectivity, None)),
                LOST,
                "{state}/{connectivity}"
            );
            assert_eq!(
                classify(&reading(state, connectivity, Some(false))),
                WIFI_OFF,
                "{state}/{connectivity}"
            );
        }
    }
    // Unknown (0) and undocumented states never cause a transition.
    for state in [0, 1, 9, 11, 15, 25, 45, 55, 69, 71, 80, 100, u32::MAX] {
        for connectivity in [0, 1, 4] {
            assert_eq!(
                classify(&reading(state, connectivity, Some(false))),
                UNKNOWN,
                "{state}/{connectivity}"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------
// Tests 2–10 — tracker
// ---------------------------------------------------------------------------------------

/// Test 2 (ROA3, §5.2): an outage is recorded with wall start and end and a duration from
/// `CLOCK_BOOTTIME`, never from the wall clock (an NTP jump while offline does not change it),
/// and the closing step releases its notice.
#[test]
fn test_roa_tracker_records_network_interval_with_boottime_duration() {
    let mut tracker = started();
    assert_eq!(tracker.tick(at(5_000), ONLINE), Default::default());
    assert_eq!(tracker.tick(at(10_000), LOST), Default::default());
    assert_eq!(tracker.tick(at(70_000), LOST), Default::default());
    // The wall clock is corrected by one hour when the network comes back.
    let mut back = at(75_000);
    back.unix_ms += HOUR_MS;
    let outcome = tracker.tick(back, ONLINE);
    let expected = interval(
        OfflineReason::NetworkLost,
        BASE_UNIX_MS + 10_000,
        BASE_UNIX_MS + 75_000 + HOUR_MS,
        65_000,
    );
    assert_eq!(outcome.recorded, vec![expected]);
    assert_eq!(
        outcome.notice,
        Some(OfflineNotice {
            reason: OfflineReason::NetworkLost,
            intervals: 1,
            first_start_unix_ms: BASE_UNIX_MS + 10_000,
            last_end_unix_ms: BASE_UNIX_MS + 75_000 + HOUR_MS,
            last_duration_ms: 65_000,
        })
    );
    assert_eq!(tracker.history(), vec![expected]);
    assert_eq!(tracker.history_newest_first(), vec![expected]);
    // Online again: nothing more.
    assert_eq!(tracker.tick(at(80_000), ONLINE), Default::default());
    assert_eq!(tracker.history().len(), 1);
}

/// Test 3 (ROA3, D-6): an interval shorter than `OFFLINE_MIN_INTERVAL_MS` is dropped (no
/// history entry, no notice); exactly the floor is recorded.
#[test]
fn test_roa_tracker_drops_intervals_below_floor() {
    let mut tracker = started();
    tracker.tick(at(10_000), LOST);
    let short = tracker.tick(at(10_000 + OFFLINE_MIN_INTERVAL_MS - 1), ONLINE);
    assert!(short.recorded.is_empty(), "{short:?}");
    assert_eq!(short.notice, None);
    assert!(tracker.history().is_empty());

    tracker.tick(at(100_000), WIFI_OFF);
    let exact = tracker.tick(at(100_000 + OFFLINE_MIN_INTERVAL_MS), ONLINE);
    assert_eq!(
        exact.recorded,
        vec![interval(
            OfflineReason::WifiDisabled,
            BASE_UNIX_MS + 100_000,
            BASE_UNIX_MS + 100_000 + OFFLINE_MIN_INTERVAL_MS,
            OFFLINE_MIN_INTERVAL_MS
        )]
    );
    assert!(exact.notice.is_some());
}

/// Test 4 (ROA3, D-4): Wi-Fi turned off during a `network_lost` outage names the outage
/// `wifi_disabled`; the reverse never downgrades it.
#[test]
fn test_roa_tracker_wifi_disabled_upgrades_open_interval() {
    let mut tracker = started();
    tracker.tick(at(10_000), LOST);
    tracker.tick(at(15_000), WIFI_OFF);
    tracker.tick(at(20_000), LOST);
    let outcome = tracker.tick(at(60_000), ONLINE);
    assert_eq!(outcome.recorded.len(), 1);
    assert_eq!(outcome.recorded[0].reason, OfflineReason::WifiDisabled);
    assert_eq!(outcome.recorded[0].start_unix_ms, BASE_UNIX_MS + 10_000);
    assert_eq!(
        outcome.notice.map(|n| n.reason),
        Some(OfflineReason::WifiDisabled)
    );

    tracker.tick(at(100_000), WIFI_OFF);
    tracker.tick(at(105_000), LOST);
    let second = tracker.tick(at(140_000), ONLINE);
    assert_eq!(second.recorded[0].reason, OfflineReason::WifiDisabled);
}

/// Test 5 (ROA3, D-1, D-5): a gap between `CLOCK_BOOTTIME` and `CLOCK_MONOTONIC` of at least
/// the floor is a `sleep` interval ending at the resume sample, never pushed; an outage open
/// before the suspend is closed at the last sample before it (its notice waits for the
/// network); a shorter gap is no sleep.
#[test]
fn test_roa_tracker_detects_sleep_from_clock_gap() {
    // Plain suspend of one hour while online.
    let mut tracker = started();
    tracker.tick(at(5_000), ONLINE);
    let resumed = after_sleep(10_000, HOUR_MS);
    let outcome = tracker.tick(resumed, ONLINE);
    assert_eq!(
        outcome.recorded,
        vec![interval(
            OfflineReason::Sleep,
            resumed.unix_ms - HOUR_MS,
            resumed.unix_ms,
            HOUR_MS
        )]
    );
    assert_eq!(outcome.notice, None, "a sleep is never pushed");

    // A gap just below the floor is not a sleep.
    let mut tracker = started();
    tracker.tick(at(5_000), ONLINE);
    let outcome = tracker.tick(after_sleep(10_000, OFFLINE_MIN_INTERVAL_MS - 1), ONLINE);
    assert!(outcome.recorded.is_empty(), "{outcome:?}");
    // Exactly the floor is a sleep.
    let mut tracker = started();
    tracker.tick(at(5_000), ONLINE);
    let outcome = tracker.tick(after_sleep(10_000, OFFLINE_MIN_INTERVAL_MS), ONLINE);
    assert_eq!(outcome.recorded.len(), 1, "{outcome:?}");
    assert_eq!(outcome.recorded[0].duration_ms, OFFLINE_MIN_INTERVAL_MS);

    // Offline before the suspend, still offline after the resume, online 10 s later.
    let mut tracker = started();
    tracker.tick(at(10_000), LOST);
    tracker.tick(at(60_000), LOST);
    let resumed = after_sleep(65_000, 2 * HOUR_MS);
    let outcome = tracker.tick(resumed, LOST);
    let network = interval(
        OfflineReason::NetworkLost,
        BASE_UNIX_MS + 10_000,
        BASE_UNIX_MS + 60_000,
        50_000,
    );
    let sleep = interval(
        OfflineReason::Sleep,
        resumed.unix_ms - 2 * HOUR_MS,
        resumed.unix_ms,
        2 * HOUR_MS,
    );
    assert_eq!(outcome.recorded, vec![network, sleep]);
    assert_eq!(outcome.notice, None, "still offline: the notice waits");
    let back = tracker.tick(after_sleep(75_000, 2 * HOUR_MS), ONLINE);
    assert!(
        back.recorded.is_empty(),
        "10 s after the resume: below the floor"
    );
    assert_eq!(back.notice, Some(OfflineNotice::of(&network)));
    assert_eq!(tracker.history(), vec![network, sleep]);
}

/// Test 6 (ROA5, §15 F-2, §16 R2-3): a held notice is released by an `Online` step; fewer than
/// `OFFLINE_UNMONITORED_RELEASE_TICKS` consecutive `Unknown` steps release nothing.
#[test]
fn test_roa_tracker_releases_notice_only_when_online() {
    let previous = interval(
        OfflineReason::ShutdownOrCrash,
        BASE_UNIX_MS - HOUR_MS,
        BASE_UNIX_MS,
        HOUR_MS,
    );
    let mut tracker = OfflineTracker::new(Vec::new());
    let start = tracker.start(Some(previous), at(0));
    assert_eq!(start.recorded, vec![previous]);
    assert_eq!(start.notice, None, "the start step never releases");
    assert_eq!(tracker.tick(at(5_000), LOST).notice, None);
    assert_eq!(tracker.tick(at(10_000), WIFI_OFF).notice, None);
    for i in 0..(u64::from(OFFLINE_UNMONITORED_RELEASE_TICKS) - 1) {
        assert_eq!(
            tracker.tick(at(15_000 + i * 5_000), UNKNOWN).notice,
            None,
            "fewer than OFFLINE_UNMONITORED_RELEASE_TICKS Unknown steps never release ({i})"
        );
    }
    let released = tracker.tick(at(500_000), ONLINE);
    // The outage open since 5 s closes now and joins the held notice.
    let outage = interval(
        OfflineReason::WifiDisabled,
        BASE_UNIX_MS + 5_000,
        BASE_UNIX_MS + 500_000,
        495_000,
    );
    assert_eq!(released.recorded, vec![outage]);
    assert_eq!(
        released.notice,
        Some(OfflineNotice::merge(
            &OfflineNotice::of(&previous),
            &OfflineNotice::of(&outage)
        ))
    );
    assert_eq!(
        tracker.tick(at(505_000), ONLINE).notice,
        None,
        "released once"
    );
}

/// Test 6b (ROA5, D-15, §16 R2-3): `OFFLINE_UNMONITORED_RELEASE_TICKS` consecutive `Unknown`
/// steps release a held notice (NetworkManager absent or stopped); an `Online` or `Offline`
/// step restarts the count.
#[test]
fn test_roa_tracker_unmonitored_release() {
    let previous = interval(
        OfflineReason::ShutdownOrCrash,
        BASE_UNIX_MS - HOUR_MS,
        BASE_UNIX_MS,
        HOUR_MS,
    );
    let mut tracker = OfflineTracker::new(Vec::new());
    tracker.start(Some(previous), at(0));
    let ticks = u64::from(OFFLINE_UNMONITORED_RELEASE_TICKS);
    for i in 1..ticks {
        assert_eq!(tracker.tick(at(i * 5_000), UNKNOWN).notice, None, "{i}");
    }
    assert_eq!(
        tracker.tick(at(ticks * 5_000), UNKNOWN).notice,
        Some(OfflineNotice::of(&previous))
    );
    assert_eq!(tracker.tick(at((ticks + 1) * 5_000), UNKNOWN).notice, None);
    // An Offline step restarts the count.
    let mut tracker = OfflineTracker::new(Vec::new());
    tracker.start(Some(previous), at(0));
    for i in 1..ticks {
        tracker.tick(at(i * 5_000), UNKNOWN);
    }
    assert_eq!(tracker.tick(at(ticks * 5_000), LOST).notice, None);
    for i in 1..ticks {
        assert_eq!(
            tracker.tick(at((ticks + i) * 5_000), UNKNOWN).notice,
            None,
            "{i}"
        );
    }
    assert!(tracker
        .tick(at(2 * ticks * 5_000), UNKNOWN)
        .notice
        .is_some());
    // The NameHasOwner gate: no NetworkManager read without an owner.
    assert!(nm_reads_allowed(true));
    assert!(!nm_reads_allowed(false));
}

/// Test 7 (ROA7, §5.1): notices held together merge (count, earliest start, latest end,
/// newest reason and duration).
#[test]
fn test_roa_tracker_merges_pending_notices() {
    let previous = interval(
        OfflineReason::ShutdownOrCrash,
        BASE_UNIX_MS - HOUR_MS,
        BASE_UNIX_MS,
        HOUR_MS,
    );
    let mut tracker = OfflineTracker::new(Vec::new());
    tracker.start(Some(previous), at(0));
    tracker.tick(at(5_000), LOST);
    let outcome = tracker.tick(at(45_000), ONLINE);
    assert_eq!(
        outcome.notice,
        Some(OfflineNotice {
            reason: OfflineReason::NetworkLost,
            intervals: 2,
            first_start_unix_ms: BASE_UNIX_MS - HOUR_MS,
            last_end_unix_ms: BASE_UNIX_MS + 45_000,
            last_duration_ms: 40_000,
        })
    );
}

/// Test 8 (ROA3): the history holds at most `MAX_OFFLINE_INTERVALS` entries; the oldest is
/// evicted; a longer loaded history keeps its newest entries.
#[test]
fn test_roa_tracker_history_is_bounded_to_32() {
    let many: Vec<OfflineInterval> = (0..40u64)
        .map(|i| interval(OfflineReason::NetworkLost, i * 100, i * 100 + 50, 50))
        .collect();
    let tracker = OfflineTracker::new(many.clone());
    assert_eq!(tracker.history(), many[8..].to_vec());

    let full: Vec<OfflineInterval> = many[..MAX_OFFLINE_INTERVALS].to_vec();
    let mut tracker = OfflineTracker::new(full.clone());
    tracker.start(None, at(0));
    tracker.tick(at(10_000), LOST);
    let outcome = tracker.tick(at(50_000), ONLINE);
    assert_eq!(outcome.recorded.len(), 1);
    let history = tracker.history();
    assert_eq!(history.len(), MAX_OFFLINE_INTERVALS);
    assert_eq!(history[0], full[1], "the oldest was evicted");
    assert_eq!(history[MAX_OFFLINE_INTERVALS - 1], outcome.recorded[0]);
    let newest_first = tracker.history_newest_first();
    assert_eq!(newest_first[0], outcome.recorded[0]);
    assert_eq!(newest_first[MAX_OFFLINE_INTERVALS - 1], full[1]);
}

/// Test 9 (ROA5, D-7): `running` and `shutdown` give one `shutdown_or_crash` interval from
/// the last sign of life to now (floor applied); `stopped` gives none; a clock that went
/// backwards gives none; extreme values never panic.
#[test]
fn test_roa_previous_run_classification() {
    let last = BASE_UNIX_MS;
    for run in [RunMarker::Running, RunMarker::Shutdown] {
        assert_eq!(
            previous_run_interval(run, last, last + OFFLINE_MIN_INTERVAL_MS),
            Some(interval(
                OfflineReason::ShutdownOrCrash,
                last,
                last + OFFLINE_MIN_INTERVAL_MS,
                OFFLINE_MIN_INTERVAL_MS
            )),
            "{run:?}"
        );
        assert_eq!(
            previous_run_interval(run, last, last + OFFLINE_MIN_INTERVAL_MS - 1),
            None,
            "{run:?}: below the floor"
        );
        assert_eq!(previous_run_interval(run, last, last - 1), None, "{run:?}");
        assert_eq!(previous_run_interval(run, last, last), None, "{run:?}");
        assert_eq!(
            previous_run_interval(run, 0, u64::MAX),
            Some(interval(
                OfflineReason::ShutdownOrCrash,
                0,
                u64::MAX,
                u64::MAX
            ))
        );
    }
    assert_eq!(
        previous_run_interval(RunMarker::Stopped, last, last + 10 * HOUR_MS),
        None
    );
}

/// Test 10 (ROA5, §5.2): a clean stop closes an open outage at the stop sample (recorded
/// when long enough) without releasing any notice: the PC is not back online.
#[test]
fn test_roa_tracker_stop_closes_open_interval_without_notice() {
    let mut tracker = started();
    tracker.tick(at(10_000), LOST);
    let outcome = tracker.stop(at(50_000));
    assert_eq!(
        outcome.recorded,
        vec![interval(
            OfflineReason::NetworkLost,
            BASE_UNIX_MS + 10_000,
            BASE_UNIX_MS + 50_000,
            40_000
        )]
    );
    assert_eq!(outcome.notice, None);

    let mut tracker = started();
    tracker.tick(at(10_000), LOST);
    let short = tracker.stop(at(20_000));
    assert!(short.recorded.is_empty());
    let mut idle = started();
    assert_eq!(idle.stop(at(20_000)), Default::default());
}

// ---------------------------------------------------------------------------------------
// Tests 11–12 — vocabulary and notice
// ---------------------------------------------------------------------------------------

/// Test 11 (ROA6, D-11): the duration label vocabulary and its boundaries.
#[test]
fn test_roa_duration_label_vocabulary() {
    for (ms, label) in [
        (0, "less than a minute"),
        (30_000, "less than a minute"),
        (59_999, "less than a minute"),
        (60_000, "1 min"),
        (119_999, "1 min"),
        (120_000, "2 min"),
        (3_599_999, "59 min"),
        (3_600_000, "1 h"),
        (3_659_999, "1 h"),
        (3_660_000, "1 h 1 min"),
        (7_200_000, "2 h"),
        (172_799_999, "47 h 59 min"),
        (172_800_000, "2 days"),
        (259_199_999, "2 days"),
        (259_200_000, "3 days"),
    ] {
        assert_eq!(duration_label(ms), label, "{ms}");
    }
    let huge = duration_label(u64::MAX);
    assert!(huge.ends_with(" days"), "{huge}");
    assert!(huge.bytes().all(|b| b.is_ascii_alphanumeric() || b == b' '));
}

const fn assert_copy<T: Copy>() {}

/// Test 12 (ROA6, §5.1): a notice is `Copy` (no `String` can hide in it) and merges with
/// saturating counts; the newer operand names reason and duration.
#[test]
fn test_roa_notice_is_copy_and_merge_rules() {
    assert_copy::<OfflineNotice>();
    assert_copy::<OfflineInterval>();
    assert_copy::<OfflineReason>();
    let a = OfflineNotice::of(&interval(OfflineReason::ShutdownOrCrash, 100, 900, 800));
    assert_eq!(
        a,
        OfflineNotice {
            reason: OfflineReason::ShutdownOrCrash,
            intervals: 1,
            first_start_unix_ms: 100,
            last_end_unix_ms: 900,
            last_duration_ms: 800,
        }
    );
    let b = OfflineNotice::of(&interval(OfflineReason::WifiDisabled, 1_000, 5_000, 4_000));
    let merged = OfflineNotice {
        reason: OfflineReason::WifiDisabled,
        intervals: 2,
        first_start_unix_ms: 100,
        last_end_unix_ms: 5_000,
        last_duration_ms: 4_000,
    };
    assert_eq!(OfflineNotice::merge(&a, &b), merged);
    assert_eq!(
        OfflineNotice::merge(&b, &a),
        merged,
        "order of the arguments"
    );
    let saturated = OfflineNotice {
        intervals: u32::MAX,
        ..a
    };
    assert_eq!(OfflineNotice::merge(&saturated, &b).intervals, u32::MAX);
    assert!(OfflineReason::NetworkLost.pushes());
    assert!(OfflineReason::WifiDisabled.pushes());
    assert!(OfflineReason::ShutdownOrCrash.pushes());
    assert!(!OfflineReason::Sleep.pushes());
}

// ---------------------------------------------------------------------------------------
// Tests 13–16 — store
// ---------------------------------------------------------------------------------------

fn own_uid() -> u32 {
    nix::unistd::getuid().as_raw()
}

fn store_in(dir: &Path) -> (NetworkStore, PathBuf) {
    let path = dir.join(NETWORK_STORE_FILE_NAME);
    (NetworkStore::new(path.clone(), own_uid()), path)
}

fn private_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

fn write_mode(path: &Path, bytes: &[u8], mode: u32) {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)
        .unwrap();
    std::io::Write::write_all(&mut file, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

fn sample_file() -> NetworkStoreFile {
    NetworkStoreFile {
        run: RunMarker::Running,
        last_alive_unix_ms: BASE_UNIX_MS,
        intervals: vec![
            interval(OfflineReason::Sleep, 10, 20, 10),
            interval(OfflineReason::ShutdownOrCrash, 30, 90, 60),
        ],
    }
}

/// Test 13 (ROA4, §5.3): save then load round-trips; the file is `0600`, has exactly the
/// four specified keys and reason names in snake case; an absent file loads as `None`.
#[test]
fn test_roa_store_round_trip_and_mode_0600() {
    let dir = private_dir();
    let (store, path) = store_in(dir.path());
    assert_eq!(store.load(), Ok(None));
    let file = sample_file();
    store.save(&file, &system_random()).unwrap();
    let meta = fs::symlink_metadata(&path).unwrap();
    assert!(meta.file_type().is_file());
    assert_eq!(meta.mode() & 0o777, 0o600);
    assert_eq!(meta.uid(), own_uid());
    let loaded = store.load().unwrap().unwrap();
    assert_eq!(loaded, file);
    let json: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        json,
        json!({
            "version": 1,
            "run": "running",
            "last_alive_unix_ms": BASE_UNIX_MS,
            "intervals": [
                {"reason": "sleep", "start_unix_ms": 10, "end_unix_ms": 20, "duration_ms": 10},
                {"reason": "shutdown_or_crash", "start_unix_ms": 30, "end_unix_ms": 90, "duration_ms": 60},
            ],
        })
    );
    for run in [RunMarker::Stopped, RunMarker::Shutdown, RunMarker::Running] {
        let file = NetworkStoreFile {
            run,
            ..sample_file()
        };
        store.save(&file, &system_random()).unwrap();
        assert_eq!(store.load().unwrap().unwrap().run, run);
    }
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"running\""), "{text}");
    let names: Vec<String> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        names.iter().all(|n| !n.contains(".tmp-")),
        "no temp file left: {names:?}"
    );
}

/// Test 14 (ROA4, D-14): a symlink, a group- or world-accessible file, an oversized file,
/// and malformed content (bad JSON, unknown key, wrong version, too many intervals,
/// `start > end`, unknown reason or run marker) are refused, never repaired.
#[test]
fn test_roa_store_refuses_symlink_foreign_mode_oversize_and_malformed() {
    let dir = private_dir();
    let (store, path) = store_in(dir.path());

    // Symlink to a valid file.
    let target = dir.path().join("target.json");
    write_mode(
        &target,
        br#"{"version":1,"run":"stopped","last_alive_unix_ms":1,"intervals":[]}"#,
        0o600,
    );
    symlink(&target, &path).unwrap();
    assert_eq!(store.load(), Err(NetworkStoreError::Refused));
    fs::remove_file(&path).unwrap();

    // Mode.
    for mode in [0o640, 0o604, 0o660, 0o644] {
        write_mode(
            &path,
            br#"{"version":1,"run":"stopped","last_alive_unix_ms":1,"intervals":[]}"#,
            mode,
        );
        assert_eq!(store.load(), Err(NetworkStoreError::Refused), "{mode:o}");
        fs::remove_file(&path).unwrap();
    }

    // Oversize: one byte above the bound (valid JSON padded with spaces).
    let mut big =
        br#"{"version":1,"run":"stopped","last_alive_unix_ms":1,"intervals":[]}"#.to_vec();
    big.resize(MAX_NETWORK_STORE_BYTES + 1, b' ');
    write_mode(&path, &big, 0o600);
    assert_eq!(store.load(), Err(NetworkStoreError::Refused));
    fs::remove_file(&path).unwrap();
    // Exactly the bound is accepted.
    big.truncate(MAX_NETWORK_STORE_BYTES);
    write_mode(&path, &big, 0o600);
    assert!(store.load().unwrap().is_some());
    fs::remove_file(&path).unwrap();

    let entry = r#"{"reason":"network_lost","start_unix_ms":1,"end_unix_ms":2,"duration_ms":1}"#;
    let too_many = format!(
        r#"{{"version":1,"run":"stopped","last_alive_unix_ms":1,"intervals":[{}]}}"#,
        vec![entry; MAX_OFFLINE_INTERVALS + 1].join(",")
    );
    let just_enough = format!(
        r#"{{"version":1,"run":"stopped","last_alive_unix_ms":1,"intervals":[{}]}}"#,
        vec![entry; MAX_OFFLINE_INTERVALS].join(",")
    );
    write_mode(&path, just_enough.as_bytes(), 0o600);
    assert_eq!(
        store.load().unwrap().unwrap().intervals.len(),
        MAX_OFFLINE_INTERVALS
    );
    fs::remove_file(&path).unwrap();
    for bad in [
        "{".to_string(),
        String::new(),
        "[]".to_string(),
        too_many,
        r#"{"version":2,"run":"stopped","last_alive_unix_ms":1,"intervals":[]}"#.to_string(),
        r#"{"version":1,"run":"stopped","last_alive_unix_ms":1,"intervals":[],"ssid":"x"}"#
            .to_string(),
        r#"{"version":1,"run":"paused","last_alive_unix_ms":1,"intervals":[]}"#.to_string(),
        r#"{"version":1,"run":"stopped","intervals":[]}"#.to_string(),
        r#"{"version":1,"run":"stopped","last_alive_unix_ms":-1,"intervals":[]}"#.to_string(),
        r#"{"version":1,"run":"stopped","last_alive_unix_ms":1,"intervals":[{"reason":"network_lost","start_unix_ms":5,"end_unix_ms":2,"duration_ms":1}]}"#.to_string(),
        r#"{"version":1,"run":"stopped","last_alive_unix_ms":1,"intervals":[{"reason":"roaming","start_unix_ms":1,"end_unix_ms":2,"duration_ms":1}]}"#.to_string(),
        r#"{"version":1,"run":"stopped","last_alive_unix_ms":1,"intervals":[{"reason":"sleep","start_unix_ms":1,"end_unix_ms":2,"duration_ms":1,"bssid":"aa"}]}"#.to_string(),
    ] {
        write_mode(&path, bad.as_bytes(), 0o600);
        assert_eq!(store.load(), Err(NetworkStoreError::Malformed), "{bad}");
        assert_eq!(fs::read(&path).unwrap(), bad.as_bytes(), "never repaired");
        fs::remove_file(&path).unwrap();
    }

    // A directory or a FIFO at the path is refused.
    fs::create_dir(&path).unwrap();
    assert_eq!(store.load(), Err(NetworkStoreError::Refused));
    fs::remove_dir(&path).unwrap();
    nix::unistd::mkfifo(&path, nix::sys::stat::Mode::from_bits_truncate(0o600)).unwrap();
    assert_eq!(store.load(), Err(NetworkStoreError::Refused));
}

/// Test 15 (ROA4, §3): 32 intervals with the largest numbers still fit the 8 KiB bound.
#[test]
fn test_roa_store_max_file_fits_bound() {
    let dir = private_dir();
    let (store, path) = store_in(dir.path());
    let file = NetworkStoreFile {
        run: RunMarker::Shutdown,
        last_alive_unix_ms: u64::MAX,
        intervals: vec![
            interval(OfflineReason::ShutdownOrCrash, u64::MAX, u64::MAX, u64::MAX);
            MAX_OFFLINE_INTERVALS
        ],
    };
    store.save(&file, &system_random()).unwrap();
    let len = fs::metadata(&path).unwrap().len() as usize;
    assert!(len <= MAX_NETWORK_STORE_BYTES, "{len}");
    assert_eq!(store.load().unwrap().unwrap(), file);
}

/// Test 16 (ROA4, §5.3): a save while another holder owns `<path>.lock` is `Locked` and
/// writes nothing; the save itself replaces the file atomically (no temp file left, the
/// inode changes).
#[test]
fn test_roa_store_save_is_locked_and_atomic() {
    let dir = private_dir();
    let (store, path) = store_in(dir.path());
    store.save(&sample_file(), &system_random()).unwrap();
    let first_inode = fs::metadata(&path).unwrap().ino();
    let before = fs::read(&path).unwrap();

    let mut lock_name = path.as_os_str().to_os_string();
    lock_name.push(".lock");
    let lock_file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(PathBuf::from(lock_name))
        .unwrap();
    let held = nix::fcntl::Flock::lock(lock_file, nix::fcntl::FlockArg::LockExclusiveNonblock)
        .map_err(|(_, e)| e)
        .unwrap();
    let other = NetworkStoreFile {
        run: RunMarker::Stopped,
        ..sample_file()
    };
    assert_eq!(
        store.save(&other, &system_random()),
        Err(NetworkStoreError::Locked)
    );
    assert_eq!(
        fs::read(&path).unwrap(),
        before,
        "nothing written while locked"
    );
    drop(held);

    store.save(&other, &system_random()).unwrap();
    assert_eq!(store.load().unwrap().unwrap().run, RunMarker::Stopped);
    assert_ne!(
        fs::metadata(&path).unwrap().ino(),
        first_inode,
        "rename, not rewrite"
    );
    let names: Vec<String> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(names.iter().all(|n| !n.contains(".tmp-")), "{names:?}");

    // A parent directory writable by others is refused (never written).
    let open_dir = tempfile::tempdir().unwrap();
    fs::set_permissions(open_dir.path(), fs::Permissions::from_mode(0o777)).unwrap();
    let (open_store, open_path) = store_in(open_dir.path());
    assert!(open_store.save(&sample_file(), &system_random()).is_err());
    assert!(!open_path.exists());
}

// ---------------------------------------------------------------------------------------
// Test 17 — configuration (D-8, D-9)
// ---------------------------------------------------------------------------------------

fn parse(extra: &str) -> Result<soos_remote::config::RemoteConfig, ConfigError> {
    parse_config(
        &format!("allowed_logins = [\"owner@example.com\"]\n{extra}"),
        Some(Path::new(RUNTIME_DIR)),
    )
}

const PUSH_ON: &str =
    "password_alerts = true\nrp_id = \"pc.tail1234.ts.net\"\npush_notifications = true\n";

/// Test 17 (ROA8, D-8, D-9): `offline_alerts` defaults to `false`; `true` requires
/// `push_notifications = true` (configuration error, exit 78, otherwise); any other TOML type
/// is a syntax error; the store is the sibling `remote-network.json`.
#[test]
fn test_roa_offline_alerts_key() {
    assert_eq!(OfflineConfig::default(), OfflineConfig { enabled: false });
    let absent = parse("").unwrap();
    assert_eq!(absent.offline, OfflineConfig { enabled: false });
    assert!(!parse("offline_alerts = false\n").unwrap().offline.enabled);
    assert_eq!(
        parse("offline_alerts = true\n").map(|c| c.offline),
        Err(ConfigError::OfflineRequiresPush)
    );
    assert_eq!(
        parse("password_alerts = true\nrp_id = \"pc.tail1234.ts.net\"\noffline_alerts = true\n")
            .map(|c| c.offline),
        Err(ConfigError::OfflineRequiresPush)
    );
    assert!(!ConfigError::OfflineRequiresPush.to_string().is_empty());
    let with_push = parse(&format!("{PUSH_ON}offline_alerts = true\n")).unwrap();
    assert!(with_push.offline.enabled && with_push.push.enabled);
    assert!(!parse(PUSH_ON).unwrap().offline.enabled);
    let _ = PushConfig::default();

    for bad in [
        "offline_alerts = 1\n",
        "offline_alerts = \"true\"\n",
        "offline_alerts = []\n",
        "offline_alert = true\n",
    ] {
        assert_eq!(
            parse(bad).map(|c| c.offline),
            Err(ConfigError::Syntax),
            "{bad}"
        );
    }

    assert_eq!(
        resolve_network_store_path(Path::new("/home/me/.config/soos/remote-passkeys.json")),
        Ok(PathBuf::from("/home/me/.config/soos/remote-network.json"))
    );
    let invalid = Err(ConfigError::InvalidCredentialsPath {
        max: MAX_CREDENTIALS_PATH_LEN,
    });
    assert_eq!(resolve_network_store_path(Path::new("/")), invalid);
    assert_eq!(
        resolve_network_store_path(Path::new("remote-passkeys.json")),
        invalid
    );
    let deep = format!("/{}/p.json", "d".repeat(MAX_CREDENTIALS_PATH_LEN - 10));
    assert_eq!(resolve_network_store_path(Path::new(&deep)), invalid);
}

// ---------------------------------------------------------------------------------------
// Test 18 — route
// ---------------------------------------------------------------------------------------

/// Test 18 (ROA9, §6): `GET|HEAD /api/offline`, `405` otherwise, never Funnel-public.
#[test]
fn test_roa_offline_route_table() {
    assert_eq!(OFFLINE_PATH, "/api/offline");
    assert_eq!(route(Method::Get, "/api/offline"), Route::Offline);
    assert_eq!(route(Method::Head, "/api/offline"), Route::Offline);
    assert_eq!(route(Method::Get, "/api/offline?x=1"), Route::Offline);
    assert_eq!(route(Method::Post, "/api/offline"), Route::MethodNotAllowed);
    assert_eq!(
        route(Method::Other, "/api/offline"),
        Route::MethodNotAllowed
    );
    assert_eq!(allow_header("/api/offline"), "GET, HEAD");
    assert!(!is_funnel_public(Route::Offline));
    for near in [
        "/api/offline/",
        "/api/Offline",
        "/api/offline/x",
        "/api/offlinex",
    ] {
        assert_eq!(route(Method::Get, near), Route::NotFound, "{near}");
    }
}

// ---------------------------------------------------------------------------------------
// Tests 19–21 — payload and shared scheduler
// ---------------------------------------------------------------------------------------

fn payload(notice: &OfflineNotice, previews: PushPreviews) -> Value {
    let bytes = back_online_payload(notice, previews, RP_ID).unwrap();
    assert!(bytes.len() <= MAX_PUSH_PLAINTEXT_BYTES);
    serde_json::from_slice(&bytes).unwrap()
}

/// Test 19 (ROA6, D-11, §5.6): the exact Declarative Web Push JSON of a back-online notice,
/// detailed (reason label, duration, plural form) and generic (fixed text, no reason).
#[test]
fn test_roa_back_online_payload_detailed_and_generic() {
    let single = OfflineNotice {
        reason: OfflineReason::NetworkLost,
        intervals: 1,
        first_start_unix_ms: BASE_UNIX_MS,
        last_end_unix_ms: BASE_UNIX_MS + 720_000,
        last_duration_ms: 720_000,
    };
    assert_eq!(
        payload(&single, PushPreviews::Detailed),
        json!({
            "web_push": 8030,
            "notification": {
                "title": "Your PC is back online",
                "body": "Offline for 12 min (network lost)",
                "navigate": "https://pc.tail1234.ts.net/",
                "lang": "en",
            },
            "soos": {
                "v": 1,
                "kind": "back_online",
                "reason": "network_lost",
                "intervals": 1,
                "first_start_unix_ms": BASE_UNIX_MS,
                "last_end_unix_ms": BASE_UNIX_MS + 720_000,
            },
        })
    );
    for (reason, label, name) in [
        (
            OfflineReason::WifiDisabled,
            "Wi-Fi turned off",
            "wifi_disabled",
        ),
        (
            OfflineReason::ShutdownOrCrash,
            "shutdown or crash",
            "shutdown_or_crash",
        ),
        (OfflineReason::Sleep, "sleep", "sleep"),
        (OfflineReason::NetworkLost, "network lost", "network_lost"),
    ] {
        let notice = OfflineNotice {
            reason,
            last_duration_ms: 2 * HOUR_MS + 300_000,
            ..single
        };
        let v = payload(&notice, PushPreviews::Detailed);
        assert_eq!(
            v["notification"]["body"],
            format!("Offline for 2 h 5 min ({label})")
        );
        assert_eq!(v["soos"]["reason"], name);
    }
    let several = OfflineNotice {
        reason: OfflineReason::WifiDisabled,
        intervals: 3,
        last_duration_ms: 45_000,
        ..single
    };
    let v = payload(&several, PushPreviews::Detailed);
    assert_eq!(
        v["notification"]["body"],
        "Offline 3 times, latest for less than a minute (Wi-Fi turned off)"
    );
    assert_eq!(v["soos"]["intervals"], 3);

    let generic = payload(&several, PushPreviews::Generic);
    assert_eq!(
        generic,
        json!({
            "web_push": 8030,
            "notification": {
                "title": "Update from your PC",
                "body": "Open soos for details",
                "navigate": "https://pc.tail1234.ts.net/",
                "lang": "en",
            },
            "soos": {
                "v": 1,
                "kind": "back_online",
                "reason": null,
                "intervals": 3,
                "first_start_unix_ms": BASE_UNIX_MS,
                "last_end_unix_ms": BASE_UNIX_MS + 720_000,
            },
        })
    );
    let text = generic["notification"].to_string().to_ascii_lowercase();
    for word in ["wi-fi", "network", "offline", "shutdown", "sleep", "min"] {
        assert!(!text.contains(word), "generic text names {word}: {text}");
    }
    let extreme = OfflineNotice {
        reason: OfflineReason::ShutdownOrCrash,
        intervals: u32::MAX,
        first_start_unix_ms: u64::MAX,
        last_end_unix_ms: u64::MAX,
        last_duration_ms: u64::MAX,
    };
    for previews in [PushPreviews::Detailed, PushPreviews::Generic] {
        let bytes = back_online_payload(&extreme, previews, RP_ID).unwrap();
        assert!(bytes.len() <= MAX_PUSH_PLAINTEXT_BYTES);
    }
}

fn wrong(at_us: u64) -> Attempt {
    Attempt {
        class: SourceClass::LockScreen,
        account: AccountClass::Owner,
        kind: AttemptKind::WrongPassword,
        at_us,
    }
}

fn notice(end: u64) -> OfflineNotice {
    OfflineNotice::of(&interval(
        OfflineReason::NetworkLost,
        end - 60_000,
        end,
        60_000,
    ))
}

/// Test 20 (ROA7, D-10): alerts and back-online notices share the 30 s spacing and the
/// 20-per-hour budget; the alert summary goes first when both are due; without a notice the
/// alert API behaves exactly as before.
#[test]
fn test_roa_scheduler_shares_spacing_and_hourly_cap() {
    // Both noted at 0: alert first at 3 s, the notice 30 s later.
    let mut s = PushScheduler::new();
    s.note(wrong(BASE_UNIX_MS * 1000), 0);
    let n = notice(BASE_UNIX_MS);
    s.note_offline(n, 0);
    assert_eq!(s.due_at(), Some(PUSH_COALESCE_MS));
    assert_eq!(s.offline_due_at(), Some(PUSH_COALESCE_MS));
    assert_eq!(s.take_due(PUSH_COALESCE_MS - 1), None);
    assert!(matches!(
        s.take_due(PUSH_COALESCE_MS),
        Some(DueMessage::Alert(summary)) if summary.wrong_password == 1
    ));
    assert_eq!(s.due_at(), None);
    assert_eq!(
        s.offline_due_at(),
        Some(PUSH_COALESCE_MS + PUSH_MIN_INTERVAL_MS)
    );
    assert_eq!(
        s.take_due(PUSH_COALESCE_MS + PUSH_MIN_INTERVAL_MS - 1),
        None
    );
    assert_eq!(
        s.take_due(PUSH_COALESCE_MS + PUSH_MIN_INTERVAL_MS),
        Some(DueMessage::BackOnline(n))
    );
    assert_eq!(s.offline_due_at(), None);
    assert_eq!(s.take_due(10 * HOUR_MS), None);

    // A notice sent first delays the next alert through the old API as well.
    let mut s = PushScheduler::new();
    s.note_offline(n, 0);
    assert_eq!(
        s.take_due(PUSH_COALESCE_MS),
        Some(DueMessage::BackOnline(n))
    );
    s.note(wrong(BASE_UNIX_MS * 1000), PUSH_COALESCE_MS + 1);
    assert_eq!(s.due_at(), Some(PUSH_COALESCE_MS + PUSH_MIN_INTERVAL_MS));
    assert!(s
        .take(PUSH_COALESCE_MS + PUSH_MIN_INTERVAL_MS - 1)
        .is_none());
    assert!(s.take(PUSH_COALESCE_MS + PUSH_MIN_INTERVAL_MS).is_some());
    // ...and an alert sent through `take` spaces the next notice.
    s.note_offline(n, PUSH_COALESCE_MS + PUSH_MIN_INTERVAL_MS);
    assert_eq!(
        s.offline_due_at(),
        Some(PUSH_COALESCE_MS + 2 * PUSH_MIN_INTERVAL_MS)
    );

    // The hourly cap counts both kinds: 20 sends alternating, the 21st waits for the hour.
    let mut s = PushScheduler::new();
    let mut now = 0;
    let mut first_sent = None;
    for i in 0..PUSH_MAX_PER_HOUR {
        if i % 2 == 0 {
            s.note(wrong((BASE_UNIX_MS + now) * 1000), now);
        } else {
            s.note_offline(notice(BASE_UNIX_MS + now), now);
        }
        let due = s.due_at().or(s.offline_due_at()).unwrap();
        now = due;
        assert!(s.take_due(now).is_some(), "send {i}");
        first_sent.get_or_insert(now);
    }
    s.note_offline(notice(BASE_UNIX_MS + now), now);
    let capped = s.offline_due_at().unwrap();
    assert_eq!(capped, first_sent.unwrap() + HOUR_MS);
    assert_eq!(s.take_due(capped - 1), None);
    assert!(matches!(
        s.take_due(capped),
        Some(DueMessage::BackOnline(_))
    ));

    // Without a notice, `take_due` is `take`.
    let mut a = PushScheduler::new();
    let mut b = PushScheduler::new();
    for (i, t) in [0u64, 1_000, 40_000, 41_000].into_iter().enumerate() {
        a.note(wrong((BASE_UNIX_MS + i as u64) * 1000), t);
        b.note(wrong((BASE_UNIX_MS + i as u64) * 1000), t);
        assert_eq!(a.due_at(), b.due_at());
        assert_eq!(a.offline_due_at(), None);
        let x = a.take(t + PUSH_COALESCE_MS);
        let y = b.take_due(t + PUSH_COALESCE_MS);
        assert_eq!(x.map(DueMessage::Alert), y, "{t}");
    }
}

/// Test 21 (ROA7, §5.7): notices noted before the send merge into one; the coalescing delay
/// counts from the first one.
#[test]
fn test_roa_scheduler_merges_notices() {
    let mut s = PushScheduler::new();
    let a = OfflineNotice::of(&interval(OfflineReason::ShutdownOrCrash, 100, 900, 800));
    let b = OfflineNotice::of(&interval(OfflineReason::NetworkLost, 2_000, 70_000, 68_000));
    s.note_offline(a, 0);
    s.note_offline(b, 2_500);
    assert_eq!(s.offline_due_at(), Some(PUSH_COALESCE_MS));
    assert_eq!(
        s.take_due(PUSH_COALESCE_MS),
        Some(DueMessage::BackOnline(OfflineNotice::merge(&a, &b)))
    );
    assert_eq!(s.take_due(HOUR_MS), None);
}
