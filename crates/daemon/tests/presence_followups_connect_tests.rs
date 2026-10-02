//! Contract tests of GitHub #325 item 2 (matrix PFU2): the logind connection is opened by
//! `PresenceLogind::connect()`, awaited by the worker before every snapshot under
//! `DBUS_CONNECT_TIMEOUT_MS` and outside the `DBUS_CALL_TIMEOUT_MS` bound, so the connect
//! bound is reachable; every connect failure is `LogindUnavailable` with backoff, before any
//! snapshot call. Also PFU6: no connection attempt while the store holds no template.

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

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{
    all_logind_errors, build_pipeline, fast_presence_config, locked_session, Enrollment,
    MockPresenceLogind, PipelineOptions, PipelineParts, ScriptedAccountGuard, TestDisplay, MS_NS,
};
use soos_daemon::inference::InferenceGate;
use soos_daemon::pipeline::EMBEDDING_MODEL_ID;
use soos_daemon::presence::account::AccountState;
use soos_daemon::presence::display::DisplayState;
use soos_daemon::presence::logind::{
    LogindSessionState, PresenceLogind, PresenceLogindError, SessionId,
};
use soos_daemon::presence::switch::PresenceSwitch;
use soos_daemon::presence::worker::{PresenceTick, PresenceWorker, SkipReason};
use soos_daemon::presence::{DBUS_CALL_TIMEOUT_MS, DBUS_CONNECT_TIMEOUT_MS, PRESENCE_DISABLE_FLAG};
use soos_daemon::DaemonError;

const UID: u32 = 1000;

// The connect bound is the larger one (otherwise PFU2 could not be observed).
const _: () = assert!(DBUS_CONNECT_TIMEOUT_MS > DBUS_CALL_TIMEOUT_MS);

fn advance(offset: &AtomicU64, ms: u64) {
    offset.fetch_add(ms * MS_NS, Ordering::SeqCst);
}

/// How `connect()` behaves.
#[derive(Clone)]
enum ConnectBehaviour {
    /// Succeeds after this delay.
    After(Duration),
    /// Never resolves.
    Hang,
    /// Fails with this error at once.
    Fail(PresenceLogindError),
}

struct ConnectScript {
    behaviour: Mutex<ConnectBehaviour>,
    calls: AtomicUsize,
    events: Mutex<Vec<&'static str>>,
}

/// `MockPresenceLogind` plus a scriptable `connect()` and an ordered event log.
#[derive(Clone)]
struct ConnectingLogind {
    inner: MockPresenceLogind,
    script: Arc<ConnectScript>,
}

impl ConnectingLogind {
    fn new(behaviour: ConnectBehaviour) -> Self {
        Self {
            inner: MockPresenceLogind::default(),
            script: Arc::new(ConnectScript {
                behaviour: Mutex::new(behaviour),
                calls: AtomicUsize::new(0),
                events: Mutex::new(Vec::new()),
            }),
        }
    }
    fn set(&self, behaviour: ConnectBehaviour) {
        *self.script.behaviour.lock().unwrap() = behaviour;
    }
    fn connect_calls(&self) -> usize {
        self.script.calls.load(Ordering::SeqCst)
    }
    fn events(&self) -> Vec<&'static str> {
        self.script.events.lock().unwrap().clone()
    }
    fn event(&self, name: &'static str) {
        self.script.events.lock().unwrap().push(name);
    }
}

impl PresenceLogind for ConnectingLogind {
    async fn connect(&self) -> Result<(), PresenceLogindError> {
        self.script.calls.fetch_add(1, Ordering::SeqCst);
        self.event("connect");
        let behaviour = self.script.behaviour.lock().unwrap().clone();
        match behaviour {
            ConnectBehaviour::After(delay) => {
                tokio::time::sleep(delay).await;
                Ok(())
            }
            ConnectBehaviour::Hang => std::future::pending().await,
            ConnectBehaviour::Fail(err) => Err(err),
        }
    }

    async fn seat_sessions(&self) -> Result<Vec<LogindSessionState>, PresenceLogindError> {
        self.event("seat_sessions");
        self.inner.seat_sessions().await
    }

    async fn session_state(
        &self,
        id: &SessionId,
    ) -> Result<Option<LogindSessionState>, PresenceLogindError> {
        self.event("session_state");
        self.inner.session_state(id).await
    }

    async fn lid_closed(&self) -> Result<bool, PresenceLogindError> {
        self.event("lid_closed");
        self.inner.lid_closed().await
    }

    async fn unlock_session(&self, id: &SessionId) -> Result<(), PresenceLogindError> {
        self.event("unlock_session");
        self.inner.unlock_session(id).await
    }
}

struct Fixture {
    worker: PresenceWorker<ConnectingLogind, TestDisplay, ScriptedAccountGuard>,
    logind: ConnectingLogind,
    switch_dir: std::path::PathBuf,
    parts: PipelineParts,
}

async fn fixture(
    clock: fn() -> Result<u64, DaemonError>,
    enrolled: Vec<(u32, Enrollment)>,
    behaviour: ConnectBehaviour,
) -> Fixture {
    let mut options = PipelineOptions::new(clock);
    options.enrolled = enrolled;
    let parts = build_pipeline(options).await;
    let switch_dir = parts.temp.path().join("etc-soos");
    std::fs::create_dir_all(&switch_dir).unwrap();
    let logind = ConnectingLogind::new(behaviour);
    logind
        .inner
        .set_sessions(vec![locked_session("2", UID, "alice")]);
    let worker = PresenceWorker::new(
        fast_presence_config(),
        logind.clone(),
        TestDisplay::new(DisplayState::On),
        PresenceSwitch::new(switch_dir.clone()),
        ScriptedAccountGuard::always(AccountState::Usable),
        parts.components.clone(),
        InferenceGate::new(1, Duration::from_millis(80)),
    )
    .with_expected_embedding_model(EMBEDDING_MODEL_ID)
    .with_clock_fn(clock);
    Fixture {
        worker,
        logind,
        switch_dir,
        parts,
    }
}

fn enrolled() -> Vec<(u32, Enrollment)> {
    vec![(UID, Enrollment::LiveIdentity)]
}

/// PFU2: a connect slower than `DBUS_CALL_TIMEOUT_MS` but within `DBUS_CONNECT_TIMEOUT_MS`
/// is awaited (it is not under the call bound), then the snapshot runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pfu_connect_slower_than_the_call_bound_is_still_awaited() {
    let delay = Duration::from_millis((DBUS_CALL_TIMEOUT_MS + DBUS_CONNECT_TIMEOUT_MS) / 2);
    test_clock!(OFFSET, clock);
    let mut fx = fixture(clock, enrolled(), ConnectBehaviour::After(delay)).await;
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::InGrace),
        "a {delay:?} connect is within the connect bound: the tick proceeds to the snapshot"
    );
    assert_eq!(fx.logind.connect_calls(), 1);
    assert_eq!(fx.logind.inner.seat_calls(), 1);
    assert_eq!(fx.logind.events(), vec!["connect", "seat_sessions"]);
}

/// PFU2: a hung connect ends the tick with `LogindUnavailable` after the connect bound (not
/// the call bound, not never), before any snapshot call, and starts the backoff.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pfu_hung_connect_is_bounded_by_the_connect_timeout() {
    test_clock!(OFFSET, clock);
    let mut fx = fixture(clock, enrolled(), ConnectBehaviour::Hang).await;
    let started = Instant::now();
    let tick = tokio::time::timeout(
        Duration::from_millis(DBUS_CONNECT_TIMEOUT_MS + 5000),
        fx.worker.tick(),
    )
    .await
    .expect("a hung connect must be bounded");
    let elapsed = started.elapsed();
    assert_eq!(tick, PresenceTick::Skipped(SkipReason::LogindUnavailable));
    assert!(
        elapsed >= Duration::from_millis(DBUS_CONNECT_TIMEOUT_MS - 5),
        "the connect bound ({DBUS_CONNECT_TIMEOUT_MS} ms) applies, not the call bound \
         ({DBUS_CALL_TIMEOUT_MS} ms): {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_millis(DBUS_CONNECT_TIMEOUT_MS + 2000),
        "the tick ends near the connect bound: {elapsed:?}"
    );
    assert_eq!(
        fx.logind.inner.seat_calls(),
        0,
        "no snapshot without a connection"
    );
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::Backoff),
        "a connect failure starts the reconnect backoff"
    );
    assert_eq!(
        fx.logind.connect_calls(),
        1,
        "no connect during the backoff"
    );
    assert_eq!(fx.logind.inner.unlock_calls(), 0);
    assert_eq!(fx.parts.camera.wakes(), 0);
}

/// PFU2: every connect error is `LogindUnavailable` (never `TooManySessions`), before any
/// snapshot call, with backoff, nothing spent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pfu_connect_failure_skips_before_the_snapshot() {
    for err in all_logind_errors() {
        test_clock!(OFFSET, clock);
        let mut fx = fixture(clock, enrolled(), ConnectBehaviour::Fail(err.clone())).await;
        assert_eq!(
            fx.worker.tick().await,
            PresenceTick::Skipped(SkipReason::LogindUnavailable),
            "{err:?}"
        );
        assert_eq!(fx.logind.inner.seat_calls(), 0, "{err:?}: no snapshot");
        assert_eq!(
            fx.worker.tick().await,
            PresenceTick::Skipped(SkipReason::Backoff),
            "{err:?}: backoff"
        );
        assert_eq!(fx.logind.inner.unlock_calls(), 0, "{err:?}");
        assert_eq!(fx.parts.camera.wakes(), 0, "{err:?}");
        assert_eq!(fx.parts.tracked_uids(), 0, "{err:?}: no attempt");
    }
}

/// PFU2: a connect failure clears the lock tracker like a snapshot failure (the grace
/// restarts once logind is reachable again).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pfu_connect_failure_restarts_the_grace() {
    test_clock!(OFFSET, clock);
    let mut fx = fixture(clock, enrolled(), ConnectBehaviour::After(Duration::ZERO)).await;
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::InGrace)
    );
    fx.logind
        .set(ConnectBehaviour::Fail(PresenceLogindError::BusUnavailable));
    advance(&OFFSET, 1000);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::LogindUnavailable)
    );
    fx.logind.set(ConnectBehaviour::After(Duration::ZERO));
    // Wait out the first reconnect backoff (1 s of real time).
    tokio::time::sleep(Duration::from_millis(1100)).await;
    advance(&OFFSET, 2000);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::InGrace),
        "the lock is observed again with a fresh grace"
    );
    assert_eq!(fx.logind.inner.unlock_calls(), 0);
}

/// PFU2: `connect()` precedes the snapshot on every tick; PFU6: neither runs while the kill
/// switch is engaged or the store holds no template.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pfu_connect_precedes_every_snapshot_and_is_skipped_without_templates() {
    test_clock!(OFFSET, clock);
    let mut fx = fixture(clock, enrolled(), ConnectBehaviour::After(Duration::ZERO)).await;
    for _ in 0..3 {
        let _ = fx.worker.tick().await;
        advance(&OFFSET, 1000);
    }
    let events = fx.logind.events();
    let snapshots: Vec<usize> = events
        .iter()
        .enumerate()
        .filter(|(_, e)| **e == "seat_sessions")
        .map(|(i, _)| i)
        .collect();
    assert!(!snapshots.is_empty());
    for i in snapshots {
        assert!(
            i > 0 && events[i - 1] == "connect",
            "every snapshot directly follows a connect: {events:?}"
        );
    }

    std::fs::write(fx.switch_dir.join(PRESENCE_DISABLE_FLAG), "").unwrap();
    let before = fx.logind.connect_calls();
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::KillSwitch)
    );
    assert_eq!(
        fx.logind.connect_calls(),
        before,
        "no connect under the kill switch"
    );
    std::fs::remove_file(fx.switch_dir.join(PRESENCE_DISABLE_FLAG)).unwrap();

    test_clock!(EMPTY_OFFSET, empty_clock);
    let mut empty = fixture(empty_clock, vec![], ConnectBehaviour::After(Duration::ZERO)).await;
    for _ in 0..5 {
        assert_eq!(
            empty.worker.tick().await,
            PresenceTick::Skipped(SkipReason::NotEnrolled)
        );
        advance(&EMPTY_OFFSET, 1000);
    }
    assert_eq!(
        empty.logind.connect_calls(),
        0,
        "no bus connection while the store holds no template"
    );
    assert!(empty.logind.events().is_empty(), "no D-Bus traffic at all");
}
