//! Contract tests of GitHub #331 for the presence settle keyed on the camera stream start
//! (spec `AI/architect_spec_install_gdm_followups.md` §3, matrix IGF6–IGF7).
//!
//! The settle lower bound of a presence scan is
//! `max(woke ? start + settle : 0, stream ? stream + settle : 0)`, so a camera woken by a PAM
//! request less than `PRESENCE_WAKE_SETTLE_MS` before a scan is covered too, while a camera
//! streaming for longer than the settle is evaluated at once (IWP10 unchanged).
//!
//! IGF7 uses the spy camera's `stream_started_ns` (test-clock domain, 0 = `None`), because the
//! spy restamps every capture with the test clock.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

mod common;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use common::{
    build_worker, fast_presence_config, locked_session, sid, PipelineOptions, ScriptedAccountGuard,
    WorkerFixture, MS_NS,
};
use soos_camera_v4l::CameraManager;
use soos_daemon::presence::account::AccountState;
use soos_daemon::presence::worker::{PresenceTick, ScanOutcome, SkipReason};
use soos_daemon::presence::{presence_settle_window, PresenceSettle, PRESENCE_WAKE_SETTLE_MS};
use soos_daemon::DaemonError;
use soos_inference_ort::MockPadDetector;

const UID: u32 = 1000;
const SETTLE_MS: u64 = 1000;
const SETTLE_NS: u64 = SETTLE_MS * MS_NS;
/// An arbitrary booted-machine CLOCK_MONOTONIC instant (100 s).
const START: u64 = 100_000 * MS_NS;

const NO_SETTLE: PresenceSettle = PresenceSettle {
    not_before_ns: 0,
    wait_ms: 0,
};

// ---------------------------------------------------------------------------------------
// IGF6 — `presence_settle_window` properties
// ---------------------------------------------------------------------------------------

/// IGF6: the shipped settle constant is the one the window is computed with.
#[test]
fn test_igf_settle_window_constant_is_1000_ms() {
    assert_eq!(PRESENCE_WAKE_SETTLE_MS, SETTLE_MS);
}

/// IGF6: a zero settle never bounds a scan, whatever the inputs.
#[test]
fn test_igf_settle_window_zero_settle_is_no_bound() {
    assert_eq!(presence_settle_window(false, START, None, 0), NO_SETTLE);
    assert_eq!(presence_settle_window(true, START, None, 0), NO_SETTLE);
    assert_eq!(
        presence_settle_window(true, START, Some(START), 0),
        NO_SETTLE
    );
    assert_eq!(
        presence_settle_window(false, START, Some(START - MS_NS), 0),
        NO_SETTLE
    );
}

/// IGF6: not woken and no stream stamp → no bound (today's streaming camera).
#[test]
fn test_igf_settle_window_not_woken_without_stamp_is_no_bound() {
    assert_eq!(
        presence_settle_window(false, START, None, SETTLE_MS),
        NO_SETTLE
    );
}

/// IGF6: woken without a stream stamp → exactly the #329 bound `start + settle`.
#[test]
fn test_igf_settle_window_woken_without_stamp_keeps_the_329_bound() {
    assert_eq!(
        presence_settle_window(true, START, None, SETTLE_MS),
        PresenceSettle {
            not_before_ns: START + SETTLE_NS,
            wait_ms: SETTLE_MS,
        }
    );
}

/// IGF6: stream age exactly the settle → no bound (boundary).
#[test]
fn test_igf_settle_window_stream_age_equal_to_settle_is_no_bound() {
    assert_eq!(
        presence_settle_window(false, START, Some(START - SETTLE_NS), SETTLE_MS),
        NO_SETTLE
    );
}

/// IGF6: stream age one millisecond below the settle → one millisecond to wait.
#[test]
fn test_igf_settle_window_stream_age_999_ms_waits_1_ms() {
    let stream = START - 999 * MS_NS;
    assert_eq!(
        presence_settle_window(false, START, Some(stream), SETTLE_MS),
        PresenceSettle {
            not_before_ns: stream + SETTLE_NS,
            wait_ms: 1,
        }
    );
}

/// IGF6: a stream that has just started → the whole settle.
#[test]
fn test_igf_settle_window_stream_age_zero_waits_the_whole_settle() {
    assert_eq!(
        presence_settle_window(false, START, Some(START), SETTLE_MS),
        PresenceSettle {
            not_before_ns: START + SETTLE_NS,
            wait_ms: SETTLE_MS,
        }
    );
}

/// IGF6: a stream started by a PAM request 300 ms before the scan → bound `stream + settle`,
/// 700 ms to wait.
#[test]
fn test_igf_settle_window_stream_age_300_ms_waits_700_ms() {
    let stream = START - 300 * MS_NS;
    assert_eq!(
        presence_settle_window(false, START, Some(stream), SETTLE_MS),
        PresenceSettle {
            not_before_ns: stream + SETTLE_NS,
            wait_ms: 700,
        }
    );
}

/// IGF6: `wait_ms` is rounded up to whole milliseconds.
#[test]
fn test_igf_settle_window_rounds_the_wait_up() {
    // Stream age 999.5 ms → 0.5 ms left → 1 ms.
    let stream = START - 999 * MS_NS - MS_NS / 2;
    let window = presence_settle_window(false, START, Some(stream), SETTLE_MS);
    assert_eq!(window.not_before_ns, stream + SETTLE_NS);
    assert_eq!(window.wait_ms, 1);
    // Stream age 299.999999 ms → 700.000001 ms left → 701 ms.
    let stream = START - 300 * MS_NS + 1;
    let window = presence_settle_window(false, START, Some(stream), SETTLE_MS);
    assert_eq!(window.not_before_ns, stream + SETTLE_NS);
    assert_eq!(window.wait_ms, 701);
}

/// IGF6: a stream stamp later than the scan start is clamped to the scan start.
#[test]
fn test_igf_settle_window_future_stream_stamp_is_clamped_to_start() {
    assert_eq!(
        presence_settle_window(false, START, Some(START + 5 * SETTLE_NS), SETTLE_MS),
        PresenceSettle {
            not_before_ns: START + SETTLE_NS,
            wait_ms: SETTLE_MS,
        }
    );
}

/// IGF6: `Some(0)` is treated as `None`.
#[test]
fn test_igf_settle_window_zero_stamp_is_none() {
    assert_eq!(
        presence_settle_window(false, START, Some(0), SETTLE_MS),
        presence_settle_window(false, START, None, SETTLE_MS)
    );
    assert_eq!(
        presence_settle_window(true, START, Some(0), SETTLE_MS),
        presence_settle_window(true, START, None, SETTLE_MS)
    );
    // `Some(0)` must not be read as "stream started at boot, long ago" either way: with
    // `start < settle` it would otherwise produce a bound.
    assert_eq!(
        presence_settle_window(false, 10 * MS_NS, Some(0), SETTLE_MS),
        NO_SETTLE
    );
}

/// IGF6: woken + stream older than the settle → today's `start + settle`.
#[test]
fn test_igf_settle_window_woken_with_old_stream_keeps_start_bound() {
    assert_eq!(
        presence_settle_window(true, START, Some(START - 5 * SETTLE_NS), SETTLE_MS),
        PresenceSettle {
            not_before_ns: START + SETTLE_NS,
            wait_ms: SETTLE_MS,
        }
    );
}

/// IGF6: woken + stream 300 ms old → the maximum, `start + settle`.
#[test]
fn test_igf_settle_window_woken_with_young_stream_takes_the_maximum() {
    assert_eq!(
        presence_settle_window(true, START, Some(START - 300 * MS_NS), SETTLE_MS),
        PresenceSettle {
            not_before_ns: START + SETTLE_NS,
            wait_ms: SETTLE_MS,
        }
    );
}

/// IGF6: saturating arithmetic at `u64::MAX`, `wait_ms <= settle_ms` and
/// `not_before_ns > start_ns` iff `wait_ms > 0`.
#[test]
fn test_igf_settle_window_saturates_without_panic() {
    let cases = [
        (true, u64::MAX, None, SETTLE_MS),
        (true, u64::MAX, Some(u64::MAX), SETTLE_MS),
        (false, u64::MAX, Some(u64::MAX), SETTLE_MS),
        (false, u64::MAX - 1, Some(u64::MAX), SETTLE_MS),
        (true, START, Some(u64::MAX), u64::MAX),
        (false, START, Some(START - 1), u64::MAX),
        (true, 0, None, u64::MAX),
        (false, 0, Some(1), SETTLE_MS),
        (
            true,
            u64::MAX - SETTLE_NS / 2,
            Some(u64::MAX - 1),
            SETTLE_MS,
        ),
    ];
    for (woke, start, stream, settle) in cases {
        let w = presence_settle_window(woke, start, stream, settle);
        assert!(
            w.wait_ms <= settle,
            "wait_ms {} > settle {settle} for {woke} {start} {stream:?}",
            w.wait_ms
        );
        assert_eq!(
            w.not_before_ns > start,
            w.wait_ms > 0,
            "not_before > start iff wait > 0 ({w:?}, start {start})"
        );
        if w.wait_ms == 0 {
            assert_eq!(w, NO_SETTLE, "no wait means no bound ({w:?})");
        }
    }
}

/// IGF6: exhaustive property over a grid of ages, wake flags and settles.
#[test]
fn test_igf_settle_window_properties_over_a_grid() {
    for settle in [0_u64, 1, 250, 999, 1000, 2500] {
        for woke in [false, true] {
            for age_ms in [0_u64, 1, 299, 300, 700, 998, 999, 1000, 1001, 5000] {
                let stream = START - age_ms * MS_NS;
                let w = presence_settle_window(woke, START, Some(stream), settle);
                let stream_bound = stream.saturating_add(settle * MS_NS);
                let wake_bound = if woke {
                    START.saturating_add(settle * MS_NS)
                } else {
                    0
                };
                let bound = stream_bound.max(wake_bound);
                if bound <= START {
                    assert_eq!(w, NO_SETTLE, "settle {settle} woke {woke} age {age_ms}");
                } else {
                    assert_eq!(
                        w.not_before_ns, bound,
                        "settle {settle} woke {woke} age {age_ms}"
                    );
                    assert_eq!(
                        w.wait_ms,
                        (bound - START).div_ceil(MS_NS),
                        "settle {settle} woke {woke} age {age_ms}"
                    );
                }
                assert!(w.wait_ms <= settle);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// IGF7 — worker: a camera started by a PAM request shortly before a scan is settled
// ---------------------------------------------------------------------------------------

/// Upper bound of how long the PAD observer waits for a first evaluation.
const OBSERVE_LIMIT: Duration = Duration::from_secs(6);
const NOT_SEEN: u64 = u64::MAX;

type Fixture = WorkerFixture<ScriptedAccountGuard>;

fn advance(offset: &AtomicU64, ms: u64) {
    offset.fetch_add(ms * MS_NS, Ordering::SeqCst);
}

fn unlocked() -> PresenceTick {
    PresenceTick::Scanned(ScanOutcome::Unlocked {
        session: sid("2"),
        uid: UID,
    })
}

async fn eligible(clock: fn() -> Result<u64, DaemonError>) -> Fixture {
    let fx = build_worker(
        PipelineOptions::new(clock),
        fast_presence_config(),
        ScriptedAccountGuard::always(AccountState::Usable),
    )
    .await;
    fx.logind
        .set_sessions(vec![locked_session("2", UID, "alice")]);
    fx
}

async fn pass_grace(fx: &mut Fixture, offset: &AtomicU64) {
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::InGrace)
    );
    advance(offset, 1000);
}

/// Records, in ms since `notify_activity`, the first instant a PAD evaluation is seen
/// (observation can only be late, never early).
#[derive(Clone)]
struct PadObserver {
    first_seen_ms: Arc<AtomicU64>,
    handle: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl PadObserver {
    fn new() -> Self {
        Self {
            first_seen_ms: Arc::new(AtomicU64::new(NOT_SEEN)),
            handle: Arc::new(Mutex::new(None)),
        }
    }

    fn start(&self, pad: Arc<MockPadDetector>) {
        let mut slot = self.handle.lock().unwrap();
        if slot.is_some() {
            return;
        }
        let first_seen = Arc::clone(&self.first_seen_ms);
        let started = Instant::now();
        let baseline = pad.call_count();
        *slot = Some(std::thread::spawn(move || {
            while started.elapsed() < OBSERVE_LIMIT {
                if pad.call_count() > baseline {
                    let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                    first_seen.store(ms, Ordering::SeqCst);
                    return;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        }));
    }

    fn first_seen_ms(&self) -> u64 {
        if let Some(handle) = self.handle.lock().unwrap().take() {
            handle.join().unwrap();
        }
        self.first_seen_ms.load(Ordering::SeqCst)
    }
}

/// Installs a `notify_activity` hook that stamps the spy camera's stream start `age_ms` before
/// the current test-clock instant (the camera stays ready: no presence wake) and starts the
/// PAD observer at the same instant.
fn stamp_stream_on_scan(
    fx: &Fixture,
    observer: &PadObserver,
    clock: fn() -> Result<u64, DaemonError>,
    age_ms: u64,
) {
    let camera = Arc::downgrade(&fx.parts.camera);
    let pad = Arc::clone(&fx.parts.pad);
    let observer = observer.clone();
    *fx.parts.camera.on_wake.lock().unwrap() = Some(Box::new(move || {
        if let Some(camera) = camera.upgrade() {
            let now = clock().unwrap();
            camera
                .stream_started_ns
                .store(now.saturating_sub(age_ms * MS_NS), Ordering::SeqCst);
        }
        observer.start(Arc::clone(&pad));
    }));
}

/// IGF7: the camera is streaming (no presence wake) but its stream started 300 ms before the
/// scan (a PAM request woke it): no PAD evaluation before the remaining ≈ 700 ms, and the scan
/// still unlocks with the unchanged `k = 3` consensus.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_igf_scan_settles_a_stream_started_shortly_before() {
    test_clock!(OFFSET, clock);
    let mut fx = eligible(clock).await;
    pass_grace(&mut fx, &OFFSET).await;
    assert!(
        fx.parts.camera.is_ready(),
        "the camera is streaming before the scan (no presence wake)"
    );
    let observer = PadObserver::new();
    stamp_stream_on_scan(&fx, &observer, clock, 300);

    let tick = fx.worker.tick().await;
    let first_pad_ms = observer.first_seen_ms();
    assert_eq!(tick, unlocked(), "the settled scan still unlocks the owner");
    assert!(
        first_pad_ms != NOT_SEEN && first_pad_ms >= 690,
        "no PAD evaluation may happen before stream start + {SETTLE_MS} ms (≈ 700 ms after \
         the scan start); first PAD call seen {first_pad_ms} ms after the scan start"
    );
    assert!(
        first_pad_ms < SETTLE_MS,
        "the settle is keyed on the stream start, not on the scan start (first PAD call \
         {first_pad_ms} ms after the scan start)"
    );
    assert_eq!(
        fx.parts.inferences(),
        3,
        "k = 3 passing captures after the settle"
    );
    assert_eq!(fx.logind.unlock_calls(), 1);
    assert_eq!(fx.parts.remaining_attempts(UID), 39, "one attempt per scan");
}

/// IGF7: a camera whose stream started more than the settle ago is evaluated at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_igf_scan_of_a_long_running_stream_has_no_settle() {
    test_clock!(OFFSET, clock);
    let mut fx = eligible(clock).await;
    pass_grace(&mut fx, &OFFSET).await;
    let observer = PadObserver::new();
    stamp_stream_on_scan(&fx, &observer, clock, 5000);

    let tick = fx.worker.tick().await;
    let first_pad_ms = observer.first_seen_ms();
    assert_eq!(tick, unlocked());
    assert!(
        first_pad_ms < 500,
        "a stream older than the settle must not be waited for (first PAD call \
         {first_pad_ms} ms after the scan start)"
    );
    assert_eq!(fx.logind.unlock_calls(), 1);
}
