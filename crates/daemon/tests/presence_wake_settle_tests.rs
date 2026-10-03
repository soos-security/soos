//! Contract tests of GitHub #329 for the presence-only wake settle (spec
//! `AI/architect_spec_install_presence_warmup.md` §5, matrix IWP8–IWP10).
//!
//! When a presence scan wakes the camera from auto-standby, captures taken while the sensor's
//! auto-exposure converges are never evaluated (no PAD, no inference) during the first
//! `PRESENCE_WAKE_SETTLE_MS` (1000 ms, pinned by
//! `presence_wake_settle_consensus_tests::test_iwp_settle_constant_value`). The consensus rules
//! are unchanged afterwards: a spoof capture still vetoes the scan.
//!
//! A woken camera is simulated with the existing mock API: the mock camera is starved (not
//! ready, no frame) before the scan, and the spy camera's `on_wake` hook, which runs inside
//! `notify_activity`, un-starves it. The spy camera restamps every capture with the test clock,
//! so capture stamps and the worker clock share one domain.

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
use soos_daemon::DaemonError;
use soos_inference_ort::{AttackType, MockPadDetector, PadResult};

const UID: u32 = 1000;

/// Mirror of `soos_daemon::presence::PRESENCE_WAKE_SETTLE_MS` (pinned by the API test file).
const EXPECTED_SETTLE_MS: u64 = 1000;

/// Upper bound of how long the PAD observer waits for a first evaluation.
const OBSERVE_LIMIT: Duration = Duration::from_secs(6);

/// Value of the observer slot while no PAD evaluation has been seen.
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

/// First tick inside the grace (no camera access), then the clock jumps past the grace.
async fn pass_grace(fx: &mut Fixture, offset: &AtomicU64) {
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::InGrace)
    );
    advance(offset, 1000);
}

/// Puts the camera in auto-standby (not ready, no frame) until the next `notify_activity`.
async fn put_camera_in_standby(fx: &Fixture) {
    let inner = Arc::clone(&fx.parts.camera.inner);
    inner.set_starved(true);
    // Let the mock capture thread observe the flag, then clear any frame it raced in.
    tokio::time::sleep(Duration::from_millis(100)).await;
    inner.set_starved(true);
    assert!(
        !fx.parts.camera.is_ready(),
        "the simulated standby camera must not be ready before the scan"
    );
}

/// Observes PAD evaluations from the camera wake: records, in ms since the wake, the first
/// instant at which `MockPadDetector::call_count()` is seen above its value at the wake (observation can only
/// be late, never early, so a recorded value below the settle proves an early evaluation).
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

    /// Starts observing now (called from the `on_wake` hook, i.e. inside `notify_activity`).
    fn start(&self, fx_pad: Arc<MockPadDetector>) {
        let mut slot = self.handle.lock().unwrap();
        if slot.is_some() {
            return;
        }
        let first_seen = Arc::clone(&self.first_seen_ms);
        let woke_at = Instant::now();
        // `build_pipeline` evaluates the mock frame once to derive the enrolled embedding.
        let baseline = fx_pad.call_count();
        *slot = Some(std::thread::spawn(move || {
            while woke_at.elapsed() < OBSERVE_LIMIT {
                if fx_pad.call_count() > baseline {
                    let ms = u64::try_from(woke_at.elapsed().as_millis()).unwrap_or(u64::MAX);
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

/// Installs the wake hook: un-starve the camera (it resumes), start the PAD observer and run
/// `extra` once.
fn on_wake(fx: &Fixture, observer: &PadObserver, extra: impl Fn() + Send + Sync + 'static) {
    let inner = Arc::clone(&fx.parts.camera.inner);
    let pad = Arc::clone(&fx.parts.pad);
    let observer = observer.clone();
    *fx.parts.camera.on_wake.lock().unwrap() = Some(Box::new(move || {
        inner.set_starved(false);
        observer.start(Arc::clone(&pad));
        extra();
    }));
}

// ---------------------------------------------------------------------------------------
// IWP8 — no evaluation before the settle after a wake
// ---------------------------------------------------------------------------------------

/// IWP8: a scan that wakes the camera evaluates no capture during the settle, then unlocks
/// with the unchanged `k = 3` consensus.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_iwp_woken_scan_skips_captures_before_the_settle() {
    test_clock!(OFFSET, clock);
    let mut fx = eligible(clock).await;
    pass_grace(&mut fx, &OFFSET).await;
    put_camera_in_standby(&fx).await;
    let observer = PadObserver::new();
    on_wake(&fx, &observer, || {});

    let tick = fx.worker.tick().await;
    let first_pad_ms = observer.first_seen_ms();
    assert_eq!(tick, unlocked(), "the woken scan still unlocks the owner");
    assert!(
        first_pad_ms != NOT_SEEN && first_pad_ms >= EXPECTED_SETTLE_MS,
        "no PAD evaluation may happen before the {EXPECTED_SETTLE_MS} ms settle after a wake \
         (first PAD call seen {first_pad_ms} ms after the wake)"
    );
    assert_eq!(
        fx.parts.inferences(),
        3,
        "k = 3 passing captures after the settle"
    );
    assert_eq!(fx.logind.unlock_calls(), 1);
    assert_eq!(fx.parts.remaining_attempts(UID), 39, "one attempt per scan");
}

/// IWP8: the 2026-10-03 trace — wake-time captures look like a spoof (auto-exposure), settled
/// captures are live: the first scan after the wake unlocks instead of being vetoed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_iwp_woken_scan_unlocks_after_spoof_looking_wake_frames() {
    test_clock!(OFFSET, clock);
    let mut fx = eligible(clock).await;
    pass_grace(&mut fx, &OFFSET).await;
    put_camera_in_standby(&fx).await;
    fx.parts
        .pad
        .set_result(PadResult::spoof(0.99, AttackType::PrintPhoto));
    let observer = PadObserver::new();
    let pad = Arc::clone(&fx.parts.pad);
    let flipped = Arc::new(Mutex::new(false));
    on_wake(&fx, &observer, move || {
        let mut once = flipped.lock().unwrap();
        if *once {
            return;
        }
        *once = true;
        let pad = Arc::clone(&pad);
        // Auto-exposure converges 300 ms after the wake, well inside the settle.
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            pad.set_result(PadResult::live(0.98));
        });
    });

    let tick = fx.worker.tick().await;
    let _ = observer.first_seen_ms();
    assert_eq!(
        tick,
        unlocked(),
        "captures taken while the exposure converges must not veto the first scan after a wake"
    );
    assert_eq!(fx.logind.unlock_calls(), 1);
}

// ---------------------------------------------------------------------------------------
// IWP9 — a spoof after the settle still vetoes
// ---------------------------------------------------------------------------------------

/// IWP9: the settle never turns a veto into an allow: a spoof among the settled captures
/// vetoes the woken scan (attempt spent, nothing unlocked), and no capture was evaluated
/// before the settle.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_iwp_spoof_after_the_settle_still_vetoes() {
    test_clock!(OFFSET, clock);
    let mut fx = eligible(clock).await;
    pass_grace(&mut fx, &OFFSET).await;
    put_camera_in_standby(&fx).await;
    fx.parts.pad.set_result_sequence(vec![
        PadResult::spoof(0.99, AttackType::ScreenReplay),
        PadResult::live(0.98),
        PadResult::live(0.98),
        PadResult::live(0.98),
    ]);
    let observer = PadObserver::new();
    on_wake(&fx, &observer, || {});

    let tick = fx.worker.tick().await;
    let first_pad_ms = observer.first_seen_ms();
    assert_eq!(
        tick,
        PresenceTick::Scanned(ScanOutcome::SpoofVetoed),
        "a spoof capture after the settle vetoes the scan"
    );
    assert_eq!(fx.logind.unlock_calls(), 0, "a vetoed scan never unlocks");
    assert_eq!(fx.parts.remaining_attempts(UID), 39, "the attempt is spent");
    assert!(
        first_pad_ms != NOT_SEEN && first_pad_ms >= EXPECTED_SETTLE_MS,
        "the vetoing capture is a settled one (first PAD call {first_pad_ms} ms after the wake)"
    );
}

/// IWP9: a presentation attack held in front of the camera for the whole scan never unlocks a
/// woken scan, nor the next scan of the streaming camera.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_iwp_constant_spoof_never_unlocks_a_woken_scan() {
    test_clock!(OFFSET, clock);
    let mut fx = eligible(clock).await;
    pass_grace(&mut fx, &OFFSET).await;
    put_camera_in_standby(&fx).await;
    fx.parts
        .pad
        .set_result(PadResult::spoof(0.97, AttackType::PrintPhoto));
    let observer = PadObserver::new();
    on_wake(&fx, &observer, || {});

    let first = fx.worker.tick().await;
    let first_pad_ms = observer.first_seen_ms();
    assert_eq!(first, PresenceTick::Scanned(ScanOutcome::SpoofVetoed));
    assert!(
        first_pad_ms != NOT_SEEN && first_pad_ms >= EXPECTED_SETTLE_MS,
        "the woken scan evaluates only settled captures (first PAD call {first_pad_ms} ms)"
    );
    advance(&OFFSET, 1000);
    let second = fx.worker.tick().await;
    assert_eq!(second, PresenceTick::Scanned(ScanOutcome::SpoofVetoed));
    assert_eq!(
        fx.logind.unlock_calls(),
        0,
        "a constant spoof never unlocks"
    );
    assert_eq!(fx.parts.remaining_attempts(UID), 38, "one attempt per scan");
}

// ---------------------------------------------------------------------------------------
// IWP10 — no settle for a streaming camera
// ---------------------------------------------------------------------------------------

/// IWP10: a scan that finds the camera already streaming (no wake) applies no settle: its
/// first capture is evaluated well before `PRESENCE_WAKE_SETTLE_MS`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_iwp_scan_of_a_streaming_camera_has_no_settle() {
    test_clock!(OFFSET, clock);
    let mut fx = eligible(clock).await;
    pass_grace(&mut fx, &OFFSET).await;
    assert!(
        fx.parts.camera.is_ready(),
        "the camera is streaming before the scan"
    );
    let observer = PadObserver::new();
    let pad = Arc::clone(&fx.parts.pad);
    let obs = observer.clone();
    *fx.parts.camera.on_wake.lock().unwrap() = Some(Box::new(move || {
        obs.start(Arc::clone(&pad));
    }));

    let tick = fx.worker.tick().await;
    let first_pad_ms = observer.first_seen_ms();
    assert_eq!(tick, unlocked());
    assert!(
        first_pad_ms < EXPECTED_SETTLE_MS,
        "a scan of a streaming camera must not wait for a settle (first PAD call {first_pad_ms} ms)"
    );
}
