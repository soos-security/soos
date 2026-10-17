//! Contract tests of the per-user coordination of lock-status clients: one daemon exchange at a
//! time, a private shared cache and a shared back-off (GitHub #401; matrix LKA2; spec §9 R1, §10,
//! §11).

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

mod common;

use std::cell::Cell;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{my_uid, runtime_dir, Behavior, MockDaemon};
use soos_lockstatus::shared::{
    boottime_now_ms, decode_record, encode_record, freshness_ms, is_reusable, shared_fetch,
    socket_hash, Record, Source, CACHE_FRESH_MS, LOCK_FILE_NAME, MAX_CACHE_BYTES,
    MAX_RECORDED_FAILURES, MIN_EXCHANGE_BUDGET_MS,
};
use soos_lockstatus::view::{LockView, ViewState};
use soos_lockstatus::{ERROR_BACKOFF_MS, EXCHANGE_BUDGET_MS};
use soos_protocol::LockWidgetState;

const BUDGET: Duration = Duration::from_millis(EXCHANGE_BUDGET_MS);
const SLACK: Duration = Duration::from_millis(700);

fn scanning(sequence: u64) -> LockView {
    LockView {
        state: ViewState::Scanning,
        sequence,
    }
}

fn fetch_at(dir: &Path, daemon: &MockDaemon, clock: &Cell<Option<u64>>) -> (LockView, Source) {
    let out = shared_fetch(Some(dir), &daemon.path, my_uid(), BUDGET, &|| clock.get());
    (out.view, out.source)
}

#[test]
fn test_lka2_constants() {
    assert_eq!(LOCK_FILE_NAME, "soos-lockstatus.lock");
    assert_eq!(CACHE_FRESH_MS, 250);
    assert_eq!(CACHE_FRESH_MS, soos_lockstatus::MIN_POLL_INTERVAL_MS);
    assert_eq!(MAX_CACHE_BYTES, 128);
    assert_eq!(MIN_EXCHANGE_BUDGET_MS, 50);
    assert_eq!(MAX_RECORDED_FAILURES, 3);
}

#[test]
fn test_lka2_socket_hash_is_fnv1a_64() {
    assert_eq!(socket_hash(Path::new("")), 0xcbf2_9ce4_8422_2325);
    assert_eq!(
        socket_hash(Path::new("/run/soos/daemon.sock")),
        0x6db2_7226_93fa_5425
    );
    assert_ne!(
        socket_hash(Path::new("/run/soos/daemon.sock")),
        socket_hash(Path::new("/tmp/daemon.sock"))
    );
}

#[test]
fn test_lka2_record_round_trip_and_maximum_length() {
    let max = Record {
        view: LockView::UNAVAILABLE,
        failures: MAX_RECORDED_FAILURES,
        stamp_ms: u64::MAX,
        socket_hash: u64::MAX,
    };
    let encoded = encode_record(&max);
    assert_eq!(
        encoded,
        "v1 unavailable 0 3 18446744073709551615 ffffffffffffffff\n"
    );
    assert_eq!(decode_record(encoded.as_bytes()), Some(max));
    let longest = Record {
        view: LockView {
            state: ViewState::NotRecognized,
            sequence: u64::MAX,
        },
        failures: 0,
        stamp_ms: u64::MAX,
        socket_hash: u64::MAX,
    };
    let encoded = encode_record(&longest);
    assert!(encoded.len() <= MAX_CACHE_BYTES, "{}", encoded.len());
    assert_eq!(decode_record(encoded.as_bytes()), Some(longest));
    for (state, word) in [
        (ViewState::Idle, "idle"),
        (ViewState::Scanning, "scanning"),
        (ViewState::NotRecognized, "not_recognized"),
    ] {
        let r = Record {
            view: LockView { state, sequence: 9 },
            failures: 0,
            stamp_ms: 1234,
            socket_hash: 0xab,
        };
        let e = encode_record(&r);
        assert_eq!(e, format!("v1 {word} 9 0 1234 00000000000000ab\n"));
        assert_eq!(decode_record(e.as_bytes()), Some(r));
    }
}

#[test]
fn test_lka2_malformed_records_are_misses() {
    for bad in [
        "",
        "\n",
        "v2 idle 1 0 5 00000000000000ab\n",
        "v1 recognized 1 0 5 00000000000000ab\n",
        "v1 idle x 0 5 00000000000000ab\n",
        "v1 idle 1 0 5 ab\n",
        "v1 idle 1 0 5 00000000000000ab extra\n",
        "v1 idle 1 0 5 00000000000000ab",
        "v1 idle 1 0 -5 00000000000000ab\n",
        "v1  idle 1 0 5 00000000000000ab\n",
        // A success record never carries failures; a failure record is always unavailable
        // with sequence 0 and 1..=3 failures.
        "v1 idle 1 1 5 00000000000000ab\n",
        "v1 unavailable 0 0 5 00000000000000ab\n",
        "v1 unavailable 4 1 5 00000000000000ab\n",
        "v1 unavailable 0 4 5 00000000000000ab\n",
        "v1 scanning 18446744073709551616 0 5 00000000000000ab\n",
    ] {
        assert_eq!(decode_record(bad.as_bytes()), None, "{bad:?}");
    }
    let too_long = format!("v1 idle 1 0 5 00000000000000ab\n{}", "x".repeat(200));
    assert_eq!(decode_record(too_long.as_bytes()), None);
    assert_eq!(decode_record(&[0xFF, 0xFE, b'\n']), None);
}

#[test]
fn test_lka2_freshness_and_reuse_rules() {
    let ok = Record {
        view: scanning(3),
        failures: 0,
        stamp_ms: 10_000,
        socket_hash: 7,
    };
    assert_eq!(freshness_ms(&ok), CACHE_FRESH_MS);
    for (failures, expected) in [
        (1, ERROR_BACKOFF_MS[0]),
        (2, ERROR_BACKOFF_MS[1]),
        (3, ERROR_BACKOFF_MS[2]),
    ] {
        let failed = Record {
            view: LockView::UNAVAILABLE,
            failures,
            stamp_ms: 10_000,
            socket_hash: 7,
        };
        assert_eq!(freshness_ms(&failed), expected);
    }
    assert!(is_reusable(&ok, 10_000, 7));
    assert!(is_reusable(&ok, 10_250, 7));
    assert!(!is_reusable(&ok, 10_251, 7));
    assert!(
        !is_reusable(&ok, 9_999, 7),
        "a stamp in the future is a miss"
    );
    assert!(!is_reusable(&ok, 10_100, 8), "another socket is a miss");
    assert!(!is_reusable(&ok, u64::MAX, 7));
}

#[test]
fn test_lka2_boottime_clock_reads() {
    let a = boottime_now_ms().expect("CLOCK_BOOTTIME");
    std::thread::sleep(Duration::from_millis(20));
    let b = boottime_now_ms().unwrap();
    assert!(b >= a + 15, "{a} {b}");
}

#[test]
fn test_lka2_nominal_fetch_creates_a_private_cache_and_reuses_it() {
    let daemon = MockDaemon::start(Behavior::Answer(LockWidgetState::Scanning, 3));
    let dir = runtime_dir();
    let clock = Cell::new(Some(50_000));
    assert_eq!(
        fetch_at(dir.path(), &daemon, &clock),
        (scanning(3), Source::Daemon)
    );
    let file = dir.path().join(LOCK_FILE_NAME);
    let meta = fs::symlink_metadata(&file).unwrap();
    assert!(meta.is_file());
    assert_eq!(meta.mode() & 0o777, 0o600);
    let record = decode_record(&fs::read(&file).unwrap()).unwrap();
    assert_eq!(record.view, scanning(3));
    assert_eq!(record.failures, 0);
    assert_eq!(record.stamp_ms, 50_000);
    assert_eq!(record.socket_hash, socket_hash(&daemon.path));

    daemon.set_behavior(Behavior::Answer(LockWidgetState::NotRecognized, 4));
    clock.set(Some(50_250));
    assert_eq!(
        fetch_at(dir.path(), &daemon, &clock),
        (scanning(3), Source::Cache)
    );
    assert_eq!(daemon.requests(), 1);
    clock.set(Some(50_251));
    let (view, source) = fetch_at(dir.path(), &daemon, &clock);
    assert_eq!(source, Source::Daemon);
    assert_eq!(view.state, ViewState::NotRecognized);
    assert_eq!(daemon.requests(), 2);
}

#[test]
fn test_lka2_missing_clock_disables_the_cache() {
    let daemon = MockDaemon::start(Behavior::Answer(LockWidgetState::Idle, 1));
    let dir = runtime_dir();
    let clock = Cell::new(None);
    for expected in 1..=3 {
        let (_, source) = fetch_at(dir.path(), &daemon, &clock);
        assert_eq!(source, Source::Daemon);
        assert_eq!(daemon.requests(), expected);
    }
    let content = fs::read(dir.path().join(LOCK_FILE_NAME)).unwrap_or_default();
    assert!(content.is_empty(), "no record is written without a clock");
}

#[test]
fn test_lka2_future_stamp_and_other_socket_are_misses() {
    let daemon = MockDaemon::start(Behavior::Answer(LockWidgetState::Idle, 1));
    let dir = runtime_dir();
    let file = dir.path().join(LOCK_FILE_NAME);
    let future = Record {
        view: scanning(9),
        failures: 0,
        stamp_ms: 90_000,
        socket_hash: socket_hash(&daemon.path),
    };
    fs::write(&file, encode_record(&future)).unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    let clock = Cell::new(Some(80_000));
    assert_eq!(fetch_at(dir.path(), &daemon, &clock).1, Source::Daemon);
    let other = Record {
        view: scanning(9),
        failures: 0,
        stamp_ms: 80_000,
        socket_hash: socket_hash(Path::new("/elsewhere.sock")),
    };
    fs::write(&file, encode_record(&other)).unwrap();
    assert_eq!(fetch_at(dir.path(), &daemon, &clock).1, Source::Daemon);
    assert_eq!(daemon.requests(), 2);
}

#[test]
fn test_lka2_failures_back_off_through_the_cache() {
    let daemon = MockDaemon::start(Behavior::Close);
    let dir = runtime_dir();
    let clock = Cell::new(Some(100_000));
    assert_eq!(
        fetch_at(dir.path(), &daemon, &clock),
        (LockView::UNAVAILABLE, Source::Daemon)
    );
    let file = dir.path().join(LOCK_FILE_NAME);
    assert_eq!(
        decode_record(&fs::read(&file).unwrap()).unwrap().failures,
        1
    );
    clock.set(Some(100_000 + ERROR_BACKOFF_MS[0]));
    assert_eq!(
        fetch_at(dir.path(), &daemon, &clock),
        (LockView::UNAVAILABLE, Source::Cache)
    );
    assert_eq!(daemon.requests(), 1);
    let mut now = 100_000 + ERROR_BACKOFF_MS[0] + 1;
    clock.set(Some(now));
    assert_eq!(fetch_at(dir.path(), &daemon, &clock).1, Source::Daemon);
    assert_eq!(
        decode_record(&fs::read(&file).unwrap()).unwrap().failures,
        2
    );
    now += ERROR_BACKOFF_MS[1] + 1;
    clock.set(Some(now));
    assert_eq!(fetch_at(dir.path(), &daemon, &clock).1, Source::Daemon);
    assert_eq!(
        decode_record(&fs::read(&file).unwrap()).unwrap().failures,
        3
    );
    now += ERROR_BACKOFF_MS[2] + 1;
    clock.set(Some(now));
    assert_eq!(fetch_at(dir.path(), &daemon, &clock).1, Source::Daemon);
    assert_eq!(
        decode_record(&fs::read(&file).unwrap()).unwrap().failures,
        MAX_RECORDED_FAILURES,
        "saturates"
    );
    assert_eq!(daemon.requests(), 4);
    // Recovery resets the count.
    daemon.set_behavior(Behavior::Answer(LockWidgetState::Idle, 2));
    now += ERROR_BACKOFF_MS[2] + 1;
    clock.set(Some(now));
    assert_eq!(fetch_at(dir.path(), &daemon, &clock).1, Source::Daemon);
    assert_eq!(
        decode_record(&fs::read(&file).unwrap()).unwrap().failures,
        0
    );
}

fn assert_no_runtime_dir(dir: Option<&Path>, uid: u32) {
    let daemon = MockDaemon::start(Behavior::Answer(LockWidgetState::Scanning, 1));
    let out = shared_fetch(dir, &daemon.path, uid, BUDGET, &|| Some(1));
    assert_eq!(out.view, LockView::UNAVAILABLE);
    assert_eq!(out.source, Source::NoRuntimeDir);
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!(daemon.connections(), 0, "the daemon must not be contacted");
}

#[test]
fn test_lka2_absent_or_relative_runtime_dir_never_contacts_the_daemon() {
    assert_no_runtime_dir(None, my_uid());
    assert_no_runtime_dir(Some(Path::new("relative/dir")), my_uid());
    let dir = runtime_dir();
    assert_no_runtime_dir(Some(&dir.path().join("missing")), my_uid());
}

#[test]
fn test_lka2_shared_or_foreign_runtime_dir_never_contacts_the_daemon() {
    let dir = runtime_dir();
    for mode in [0o755, 0o750, 0o701, 0o710] {
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(mode)).unwrap();
        assert_no_runtime_dir(Some(dir.path()), my_uid());
    }
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    // A directory of another owner (here: asking for another UID).
    assert_no_runtime_dir(Some(dir.path()), my_uid().wrapping_add(1));
}

#[test]
fn test_lka2_symlinked_runtime_dir_never_contacts_the_daemon() {
    let dir = runtime_dir();
    let real = dir.path().join("real");
    fs::create_dir(&real).unwrap();
    fs::set_permissions(&real, fs::Permissions::from_mode(0o700)).unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert_no_runtime_dir(Some(&link), my_uid());
}

#[test]
fn test_lka2_unsafe_lock_file_never_contacts_the_daemon() {
    // Readable by others.
    let dir = runtime_dir();
    let file = dir.path().join(LOCK_FILE_NAME);
    fs::write(&file, b"").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
    assert_no_runtime_dir(Some(dir.path()), my_uid());

    // Hard-linked.
    let dir = runtime_dir();
    let file = dir.path().join(LOCK_FILE_NAME);
    fs::write(&file, b"").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    fs::hard_link(&file, dir.path().join("second-name")).unwrap();
    assert_no_runtime_dir(Some(dir.path()), my_uid());

    // A symlink in place of the lock file.
    let dir = runtime_dir();
    let target = dir.path().join("target");
    fs::write(&target, b"").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    std::os::unix::fs::symlink(&target, dir.path().join(LOCK_FILE_NAME)).unwrap();
    assert_no_runtime_dir(Some(dir.path()), my_uid());

    // A directory in place of the lock file.
    let dir = runtime_dir();
    fs::create_dir(dir.path().join(LOCK_FILE_NAME)).unwrap();
    assert_no_runtime_dir(Some(dir.path()), my_uid());
}

fn hold_lock(dir: &Path, hold: Duration) -> std::thread::JoinHandle<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(dir.join(LOCK_FILE_NAME))
        .unwrap();
    let locked = nix::fcntl::Flock::lock(file, nix::fcntl::FlockArg::LockExclusiveNonblock)
        .map_err(|(_, e)| e)
        .unwrap();
    std::thread::spawn(move || {
        std::thread::sleep(hold);
        drop(locked);
    })
}

#[test]
fn test_lka2_lock_held_all_budget_is_lock_busy_without_contact() {
    let daemon = MockDaemon::start(Behavior::Answer(LockWidgetState::Scanning, 1));
    let dir = runtime_dir();
    let holder = hold_lock(dir.path(), Duration::from_millis(1500));
    let start = Instant::now();
    let out = shared_fetch(Some(dir.path()), &daemon.path, my_uid(), BUDGET, &|| {
        Some(5)
    });
    let elapsed = start.elapsed();
    assert_eq!(out.source, Source::LockBusy);
    assert_eq!(out.view, LockView::UNAVAILABLE);
    assert!(elapsed < BUDGET + SLACK, "{elapsed:?}");
    assert_eq!(daemon.connections(), 0);
    holder.join().unwrap();
}

#[test]
fn test_lka2_starved_exchange_is_lock_busy_and_not_a_failure() {
    let daemon = MockDaemon::start(Behavior::Answer(LockWidgetState::Scanning, 1));
    let dir = runtime_dir();
    // Released after 240 ms of the 250 ms budget: less than MIN_EXCHANGE_BUDGET_MS is left.
    let holder = hold_lock(dir.path(), Duration::from_millis(240));
    let out = shared_fetch(Some(dir.path()), &daemon.path, my_uid(), BUDGET, &|| {
        Some(5)
    });
    holder.join().unwrap();
    assert_eq!(out.source, Source::LockBusy);
    assert_eq!(daemon.connections(), 0);
    let content = fs::read(dir.path().join(LOCK_FILE_NAME)).unwrap();
    assert!(
        content.is_empty(),
        "no failure record is written: {content:?}"
    );
}

#[test]
fn test_lka2_concurrent_clients_make_one_connection_at_a_time_and_share_the_answer() {
    let daemon = Arc::new(MockDaemon::start(Behavior::DelayedAnswer(
        Duration::from_millis(30),
        LockWidgetState::Scanning,
        6,
    )));
    let dir = Arc::new(runtime_dir());
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let daemon = Arc::clone(&daemon);
            let dir = Arc::clone(&dir);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                shared_fetch(
                    Some(dir.path()),
                    &daemon.path,
                    my_uid(),
                    Duration::from_millis(1000),
                    &boottime_now_ms,
                )
            })
        })
        .collect();
    let outcomes: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
    assert_eq!(
        daemon.max_active(),
        1,
        "at most one daemon connection at a time"
    );
    assert_eq!(
        daemon.requests(),
        1,
        "one exchange serves every client for 250 ms"
    );
    for out in &outcomes {
        assert_eq!(out.view, scanning(6));
    }
    assert_eq!(
        outcomes
            .iter()
            .filter(|o| o.source == Source::Daemon)
            .count(),
        1
    );
}
