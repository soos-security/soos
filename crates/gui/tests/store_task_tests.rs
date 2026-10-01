//! GitHub #291 (SGU1, SGU2): in direct (non-Polkit) mode the GUI never mutates the template
//! store on the UI thread.
//!
//! Contract:
//! - `StoreTaskRunner::submit` returns immediately even while another process (`soos-enroll`)
//!   holds the store lock; the `enroll` / `delete` runs on a background thread and its outcome
//!   comes back through `poll`.
//! - A lock timeout is reported as `StoreTaskError::Busy`, whose message is the user-facing
//!   `STORE_BUSY_MESSAGE`; nothing is written or removed.
//! - One store task at a time; the `Debug` output of a task never carries embedding values.
//! - `app.rs` calls neither `store.enroll` nor `store.delete` directly.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions"
)]

use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use nix::fcntl::{Flock, FlockArg};
use soos_biometric_store::{BiometricStore, BiometricStoreError, BiometricTemplate, MasterKey};
use soos_gui::store_tasks::{
    StoreTask, StoreTaskError, StoreTaskOutcome, StoreTaskRunner, StoreTaskSubmitError,
    STORE_BUSY_MESSAGE,
};
use tempfile::TempDir;
use zeroize::Zeroizing;

const UID: u32 = 1000;
/// Lock wait of the store under test (production keeps `STORE_LOCK_TIMEOUT`).
const LOCK_WAIT: Duration = Duration::from_millis(600);

fn template(value: f32) -> BiometricTemplate {
    BiometricTemplate::new(
        UID,
        "arcface_w600k_mbf".to_string(),
        "2.0.0".to_string(),
        7,
        Zeroizing::new(vec![value; 512]),
    )
    .unwrap()
}

fn setup() -> (
    TempDir,
    Arc<BiometricStore>,
    StoreTaskRunner,
    Arc<AtomicUsize>,
) {
    let temp = TempDir::new().unwrap();
    let store = Arc::new(
        BiometricStore::new(temp.path().join("bio"), MasterKey::generate().unwrap())
            .unwrap()
            .with_lock_timeout(LOCK_WAIT),
    );
    let notified = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&notified);
    let runner = StoreTaskRunner::new(
        Arc::clone(&store),
        Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }),
    );
    (temp, store, runner, notified)
}

fn hold_lock(dir: &Path) -> Flock<File> {
    Flock::lock(File::open(dir).unwrap(), FlockArg::LockExclusiveNonblock)
        .map_err(|(_, e)| e)
        .unwrap()
}

/// Polls the runner like the UI does once per frame, until one outcome arrives.
fn wait_outcome(runner: &mut StoreTaskRunner) -> StoreTaskOutcome {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let mut outcomes = runner.poll();
        if let Some(outcome) = outcomes.pop() {
            assert!(outcomes.is_empty(), "one outcome per task");
            return outcome;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("no store task outcome within 10 s");
}

#[test]
fn test_sgu_enroll_submit_never_blocks_on_a_held_store_lock() {
    let (temp, store, mut runner, notified) = setup();
    let lock = hold_lock(&temp.path().join("bio"));

    let started = Instant::now();
    runner.submit(StoreTask::Enroll(template(0.5))).unwrap();
    let submit_took = started.elapsed();
    assert!(
        submit_took < LOCK_WAIT / 2,
        "submit must return without waiting for the store lock, took {submit_took:?}"
    );
    assert!(runner.is_busy());

    match wait_outcome(&mut runner) {
        StoreTaskOutcome::Enrolled { uid, result } => {
            assert_eq!(uid, UID);
            let err = result.unwrap_err();
            assert_eq!(err, StoreTaskError::Busy);
            assert_eq!(err.to_string(), STORE_BUSY_MESSAGE);
        }
        other => panic!("expected Enrolled, got {other:?}"),
    }
    assert!(!store.exists(UID).unwrap(), "nothing is written");
    assert!(!runner.is_busy());
    assert!(notified.load(Ordering::SeqCst) >= 1, "the UI is woken up");

    drop(lock);
    runner.submit(StoreTask::Enroll(template(0.5))).unwrap();
    match wait_outcome(&mut runner) {
        StoreTaskOutcome::Enrolled { uid, result } => {
            assert_eq!(uid, UID);
            assert_eq!(result, Ok(()));
        }
        other => panic!("expected Enrolled, got {other:?}"),
    }
    assert!(store.exists(UID).unwrap());
}

#[test]
fn test_sgu_delete_submit_never_blocks_and_keeps_the_template_when_busy() {
    let (temp, store, mut runner, _notified) = setup();
    store.enroll(&template(0.5)).unwrap();
    let lock = hold_lock(&temp.path().join("bio"));

    let started = Instant::now();
    runner.submit(StoreTask::Delete { uid: UID }).unwrap();
    assert!(started.elapsed() < LOCK_WAIT / 2);

    match wait_outcome(&mut runner) {
        StoreTaskOutcome::Deleted { uid, result } => {
            assert_eq!(uid, UID);
            assert_eq!(result, Err(StoreTaskError::Busy));
        }
        other => panic!("expected Deleted, got {other:?}"),
    }
    assert!(store.exists(UID).unwrap(), "the template is kept");

    drop(lock);
    runner.submit(StoreTask::Delete { uid: UID }).unwrap();
    match wait_outcome(&mut runner) {
        StoreTaskOutcome::Deleted { uid, result } => {
            assert_eq!(uid, UID);
            assert_eq!(result, Ok(true));
        }
        other => panic!("expected Deleted, got {other:?}"),
    }
    assert!(!store.exists(UID).unwrap());
}

#[test]
fn test_sgu_only_one_store_task_runs_at_a_time() {
    let (temp, _store, mut runner, _notified) = setup();
    let lock = hold_lock(&temp.path().join("bio"));
    runner.submit(StoreTask::Delete { uid: UID }).unwrap();

    let second = runner.submit(StoreTask::Enroll(template(0.5)));
    assert!(
        matches!(second, Err(StoreTaskSubmitError::Busy)),
        "{second:?}"
    );
    drop(lock);
    let _ = wait_outcome(&mut runner);
    runner.submit(StoreTask::Delete { uid: UID }).unwrap();
    let _ = wait_outcome(&mut runner);
}

#[test]
fn test_sgu_store_task_errors_map_lock_timeout_to_the_busy_message() {
    let busy = StoreTaskError::from(&BiometricStoreError::LockTimeout("held".to_string()));
    assert_eq!(busy, StoreTaskError::Busy);
    assert!(STORE_BUSY_MESSAGE.contains("another soos operation is using the template store"));
    assert!(STORE_BUSY_MESSAGE.contains("try again"));

    let other = StoreTaskError::from(&BiometricStoreError::InvalidPath("bad".to_string()));
    match other {
        StoreTaskError::Failed(msg) => assert!(msg.contains("bad"), "{msg}"),
        StoreTaskError::Busy => panic!("only a lock timeout is reported as busy"),
    }
}

#[test]
fn test_sgu_store_task_debug_never_prints_embedding_values() {
    let shown = format!("{:?}", StoreTask::Enroll(template(0.123_456)));
    assert!(!shown.contains("0.123"), "{shown}");
    assert!(shown.contains("1000"), "{shown}");
}

#[test]
fn test_sgu_app_never_mutates_the_store_on_the_ui_thread() {
    let app = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/app.rs")).unwrap();
    for forbidden in ["store.enroll(", "store.delete(", ".enroll(&template)"] {
        assert!(
            !app.contains(forbidden),
            "app.rs (UI thread) must not call `{forbidden}`; submit a StoreTask instead"
        );
    }
    assert!(app.contains("StoreTask::Enroll"));
    assert!(app.contains("StoreTask::Delete"));
}
