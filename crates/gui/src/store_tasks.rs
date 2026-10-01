//! Template store mutations executed off the UI thread (GitHub #291).
//!
//! In direct mode (system or developer store, no Polkit) the GUI used to call
//! `BiometricStore::enroll` / `BiometricStore::delete` from a click handler. Both take the
//! exclusive store lock and wait up to `STORE_LOCK_TIMEOUT` (5 s) while `soos-enroll` holds
//! it, freezing the whole window. They now run on a background thread through
//! [`StoreTaskRunner`], the in-process counterpart of [`crate::privileged::TaskRunner`]: the UI
//! submits a [`StoreTask`], the outcome comes back over a channel drained by
//! [`StoreTaskRunner::poll`] each frame, and at most one store task runs at a time. A lock
//! timeout is reported as [`StoreTaskError::Busy`] with the user-facing
//! [`STORE_BUSY_MESSAGE`].

#![forbid(unsafe_code)]

use std::fmt;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use soos_biometric_store::{BiometricStore, BiometricStoreError, BiometricTemplate};

use crate::privileged::Notify;

/// User-facing message for a store that stayed locked by another soos process (an
/// `enroll`, `import`, `delete` or `migrate` of `soos-enroll`) for the whole lock timeout.
pub const STORE_BUSY_MESSAGE: &str =
    "another soos operation is using the template store; try again";

/// A store mutation requested by the UI.
pub enum StoreTask {
    /// `BiometricStore::enroll` of the template (its embedding is zeroized on drop).
    Enroll(BiometricTemplate),
    /// `BiometricStore::delete` of the template of `uid`.
    Delete {
        /// Target user ID.
        uid: u32,
    },
}

impl fmt::Debug for StoreTask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Enroll(template) => f
                .debug_struct("Enroll")
                .field("uid", &template.uid)
                .field("embedding_dim", &template.embedding.len())
                .finish_non_exhaustive(),
            Self::Delete { uid } => f.debug_struct("Delete").field("uid", uid).finish(),
        }
    }
}

/// Why a store task failed (messages carry paths and UIDs only, never embedding values).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreTaskError {
    /// The store lock stayed held by another operation for the whole lock timeout; nothing
    /// was changed.
    #[error("{STORE_BUSY_MESSAGE}")]
    Busy,
    /// Any other store error.
    #[error("{0}")]
    Failed(String),
}

impl From<&BiometricStoreError> for StoreTaskError {
    fn from(err: &BiometricStoreError) -> Self {
        match err {
            BiometricStoreError::LockTimeout(_) => Self::Busy,
            other => Self::Failed(other.to_string()),
        }
    }
}

/// Result of a [`StoreTask`], delivered back to the UI thread.
#[derive(Debug)]
pub enum StoreTaskOutcome {
    /// Result of [`StoreTask::Enroll`].
    Enrolled {
        /// Target user ID.
        uid: u32,
        /// Enrollment result.
        result: Result<(), StoreTaskError>,
    },
    /// Result of [`StoreTask::Delete`] (`Ok(false)`: no template existed).
    Deleted {
        /// Target user ID.
        uid: u32,
        /// Deletion result.
        result: Result<bool, StoreTaskError>,
    },
}

/// Error returned by [`StoreTaskRunner::submit`].
#[derive(Debug, thiserror::Error)]
pub enum StoreTaskSubmitError {
    /// Another store task is still running.
    #[error("another template store operation is already in progress")]
    Busy,
    /// The worker thread could not be spawned.
    #[error("failed to start the template store worker: {0}")]
    Spawn(#[from] std::io::Error),
}

/// Runs one store mutation at a time on a background thread.
pub struct StoreTaskRunner {
    store: Arc<BiometricStore>,
    notify: Notify,
    tx: Sender<StoreTaskOutcome>,
    rx: Receiver<StoreTaskOutcome>,
    in_flight: Option<(JoinHandle<()>, PendingTask)>,
}

/// Kind and target of the task in flight, kept so that a worker that panics or ends without
/// an outcome still reports a failure for the right operation and UID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingTask {
    uid: u32,
    is_enroll: bool,
}

impl PendingTask {
    const fn enroll(uid: u32) -> Self {
        Self {
            uid,
            is_enroll: true,
        }
    }

    const fn delete(uid: u32) -> Self {
        Self {
            uid,
            is_enroll: false,
        }
    }

    fn of(task: &StoreTask) -> Self {
        match task {
            StoreTask::Enroll(template) => Self::enroll(template.uid),
            StoreTask::Delete { uid } => Self::delete(*uid),
        }
    }

    /// The failed outcome of this task, with `reason` as its message.
    fn failed(self, reason: &str) -> StoreTaskOutcome {
        let error = StoreTaskError::Failed(reason.to_string());
        if self.is_enroll {
            StoreTaskOutcome::Enrolled {
                uid: self.uid,
                result: Err(error),
            }
        } else {
            StoreTaskOutcome::Deleted {
                uid: self.uid,
                result: Err(error),
            }
        }
    }
}

/// Message of the outcome delivered when the store mutation panicked.
const WORKER_PANIC_MESSAGE: &str = "the template store operation failed unexpectedly";

/// Message of the outcome synthesized when the worker ended without delivering one.
const WORKER_LOST_MESSAGE: &str = "template store worker terminated without an outcome";

/// Spawns the worker thread running `job`; a panic in `job` is delivered as a failed outcome
/// of `pending` instead of leaving the UI waiting.
fn spawn_worker(
    pending: PendingTask,
    tx: Sender<StoreTaskOutcome>,
    notify: Notify,
    job: impl FnOnce() -> StoreTaskOutcome + Send + 'static,
) -> std::io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("soos-gui-store".to_string())
        .spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job))
                .unwrap_or_else(|_| {
                    tracing::error!("template store worker panicked");
                    pending.failed(WORKER_PANIC_MESSAGE)
                });
            let _ = tx.send(outcome);
            notify();
        })
}

impl StoreTaskRunner {
    /// Creates a runner for `store`, calling `notify` whenever an outcome is delivered.
    pub fn new(store: Arc<BiometricStore>, notify: Notify) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            store,
            notify,
            tx,
            rx,
            in_flight: None,
        }
    }

    /// Starts `task` on a worker thread and returns immediately (it never waits for the
    /// store lock).
    ///
    /// # Errors
    ///
    /// [`StoreTaskSubmitError::Busy`] while another store task runs;
    /// [`StoreTaskSubmitError::Spawn`] if the worker thread cannot be created.
    pub fn submit(&mut self, task: StoreTask) -> Result<(), StoreTaskSubmitError> {
        if self.in_flight.is_some() {
            return Err(StoreTaskSubmitError::Busy);
        }
        let store = Arc::clone(&self.store);
        let pending = PendingTask::of(&task);
        let handle = spawn_worker(
            pending,
            self.tx.clone(),
            Arc::clone(&self.notify),
            move || run_task(&store, task),
        )?;
        self.in_flight = Some((handle, pending));
        Ok(())
    }

    /// Returns whether a store task is still running.
    pub fn is_busy(&self) -> bool {
        self.in_flight.is_some()
    }

    /// Drains delivered outcomes without blocking. Call once per frame from the UI thread.
    pub fn poll(&mut self) -> Vec<StoreTaskOutcome> {
        let finished = self
            .in_flight
            .as_ref()
            .is_some_and(|(handle, _)| handle.is_finished());
        let mut outcomes: Vec<StoreTaskOutcome> = self.rx.try_iter().collect();
        if !outcomes.is_empty() || finished {
            if let Some((_, pending)) = self.in_flight.take() {
                if outcomes.is_empty() {
                    tracing::error!("{WORKER_LOST_MESSAGE}");
                    outcomes.push(pending.failed(WORKER_LOST_MESSAGE));
                }
            }
        }
        outcomes
    }
}

/// Executes one task against the store (on the worker thread).
fn run_task(store: &BiometricStore, task: StoreTask) -> StoreTaskOutcome {
    match task {
        StoreTask::Enroll(template) => StoreTaskOutcome::Enrolled {
            uid: template.uid,
            result: store
                .enroll(&template)
                .map_err(|e| StoreTaskError::from(&e)),
        },
        StoreTask::Delete { uid } => StoreTaskOutcome::Deleted {
            uid,
            result: store.delete(uid).map_err(|e| StoreTaskError::from(&e)),
        },
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    reason = "Unit tests of the worker failure path use direct assertions"
)]
mod worker_failure_tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait_outcome(rx: &Receiver<StoreTaskOutcome>) -> StoreTaskOutcome {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(outcome) = rx.try_recv() {
                return outcome;
            }
            assert!(Instant::now() < deadline, "the worker delivered no outcome");
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// A panic inside the store mutation is delivered as a failed outcome of the submitted
    /// kind and UID, so the UI leaves its "Saving the template..." state.
    #[test]
    fn test_sgu_worker_panic_yields_failed_enroll_outcome() {
        let (tx, rx) = mpsc::channel();
        let notify: Notify = Arc::new(|| {});
        let handle = spawn_worker(PendingTask::enroll(1234), tx, notify, || {
            panic!("simulated store panic")
        })
        .unwrap();
        handle.join().unwrap();
        match wait_outcome(&rx) {
            StoreTaskOutcome::Enrolled { uid, result } => {
                assert_eq!(uid, 1234);
                assert!(matches!(result, Err(StoreTaskError::Failed(_))));
            }
            other => panic!("unexpected outcome {other:?}"),
        }
    }

    #[test]
    fn test_sgu_worker_panic_yields_failed_delete_outcome() {
        let (tx, rx) = mpsc::channel();
        let notify: Notify = Arc::new(|| {});
        let handle = spawn_worker(PendingTask::delete(42), tx, notify, || {
            panic!("simulated store panic")
        })
        .unwrap();
        handle.join().unwrap();
        match wait_outcome(&rx) {
            StoreTaskOutcome::Deleted { uid, result } => {
                assert_eq!(uid, 42);
                assert!(matches!(result, Err(StoreTaskError::Failed(_))));
            }
            other => panic!("unexpected outcome {other:?}"),
        }
    }

    /// Runner over a temporary store whose in-flight worker is a thread that drops its
    /// outcome sender without sending anything (a worker lost without an outcome).
    fn runner_with_lost_worker(pending: PendingTask) -> (tempfile::TempDir, StoreTaskRunner) {
        let temp = tempfile::TempDir::new().unwrap();
        let store = Arc::new(
            BiometricStore::new(
                temp.path().join("bio"),
                soos_biometric_store::MasterKey::generate().unwrap(),
            )
            .unwrap(),
        );
        let notify: Notify = Arc::new(|| {});
        let mut runner = StoreTaskRunner::new(store, notify);
        let tx = runner.tx.clone();
        let handle = thread::Builder::new()
            .name("soos-gui-store".to_string())
            .spawn(move || drop(tx))
            .unwrap();
        runner.in_flight = Some((handle, pending));
        (temp, runner)
    }

    /// Polls like the UI does once per frame until `poll` returns something.
    fn poll_until_outcome(runner: &mut StoreTaskRunner) -> Vec<StoreTaskOutcome> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let outcomes = runner.poll();
            if !outcomes.is_empty() {
                return outcomes;
            }
            assert!(
                Instant::now() < deadline,
                "poll never reported the lost worker"
            );
            assert!(runner.is_busy(), "the runner stays busy until it reports");
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// GitHub #293 (FGP3): `poll` itself turns a finished worker that delivered nothing into
    /// exactly one failed outcome of the submitted kind and UID, and frees the runner.
    #[test]
    fn test_fgp_poll_reports_a_lost_delete_worker_as_one_failure() {
        let (_temp, mut runner) = runner_with_lost_worker(PendingTask::delete(42));
        assert!(runner.is_busy());
        let mut outcomes = poll_until_outcome(&mut runner);
        assert_eq!(outcomes.len(), 1, "exactly one outcome: {outcomes:?}");
        match outcomes.pop().unwrap() {
            StoreTaskOutcome::Deleted { uid, result } => {
                assert_eq!(uid, 42);
                assert_eq!(
                    result,
                    Err(StoreTaskError::Failed(WORKER_LOST_MESSAGE.to_string()))
                );
            }
            other => panic!("unexpected outcome {other:?}"),
        }
        assert!(!runner.is_busy(), "the runner is free after the report");
        assert!(runner.poll().is_empty(), "the failure is reported once");
    }

    #[test]
    fn test_fgp_poll_reports_a_lost_enroll_worker_as_one_failure() {
        let (_temp, mut runner) = runner_with_lost_worker(PendingTask::enroll(1234));
        let mut outcomes = poll_until_outcome(&mut runner);
        assert_eq!(outcomes.len(), 1, "exactly one outcome: {outcomes:?}");
        match outcomes.pop().unwrap() {
            StoreTaskOutcome::Enrolled { uid, result } => {
                assert_eq!(uid, 1234);
                assert_eq!(
                    result,
                    Err(StoreTaskError::Failed(WORKER_LOST_MESSAGE.to_string()))
                );
            }
            other => panic!("unexpected outcome {other:?}"),
        }
        assert!(!runner.is_busy(), "the runner is free after the report");
        assert!(runner.poll().is_empty(), "the failure is reported once");
    }

    /// A worker that ends without sending anything still yields a failed outcome from `poll`.
    #[test]
    fn test_sgu_missing_outcome_is_synthesized_as_failure() {
        let pending = PendingTask::enroll(7);
        match pending.failed("template store worker terminated without an outcome") {
            StoreTaskOutcome::Enrolled { uid, result } => {
                assert_eq!(uid, 7);
                assert!(matches!(result, Err(StoreTaskError::Failed(_))));
            }
            other => panic!("unexpected outcome {other:?}"),
        }
    }
}
