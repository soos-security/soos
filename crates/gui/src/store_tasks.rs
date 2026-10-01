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
    in_flight: Option<JoinHandle<()>>,
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
        let notify = Arc::clone(&self.notify);
        let tx = self.tx.clone();
        let handle = thread::Builder::new()
            .name("soos-gui-store".to_string())
            .spawn(move || {
                let outcome = run_task(&store, task);
                let _ = tx.send(outcome);
                notify();
            })?;
        self.in_flight = Some(handle);
        Ok(())
    }

    /// Returns whether a store task is still running.
    pub fn is_busy(&self) -> bool {
        self.in_flight.is_some()
    }

    /// Drains delivered outcomes without blocking. Call once per frame from the UI thread.
    pub fn poll(&mut self) -> Vec<StoreTaskOutcome> {
        let finished = self.in_flight.as_ref().is_some_and(JoinHandle::is_finished);
        let outcomes: Vec<StoreTaskOutcome> = self.rx.try_iter().collect();
        if !outcomes.is_empty() || finished {
            if finished && outcomes.is_empty() {
                tracing::error!("template store worker terminated without an outcome");
            }
            self.in_flight = None;
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
