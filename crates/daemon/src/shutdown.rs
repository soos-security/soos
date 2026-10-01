//! Connection task tracking, bounded shutdown drain and panic reporting (GitHub #259).
//!
//! Every connection handler runs in a [`ConnectionTasks`] set instead of a detached
//! `tokio::spawn`:
//! - finished handlers are reaped while the accept loop runs, so the set stays bounded by
//!   the number of live connections;
//! - a handler panic is reported through `tracing` at error level, never with its panic
//!   payload (which may carry request data), and the connection closes without a verdict
//!   (PAM sees EOF and returns `PAM_IGNORE`);
//! - on SIGINT/SIGTERM the daemon stops accepting, unlinks its socket, then drains the
//!   remaining handlers for at most `connection_timeout`; stragglers are aborted.
//!
//! [`install_panic_hook`] replaces the default stderr panic hook with a `tracing` report
//! that records the source location only.

use std::future::Future;
use std::panic::Location;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UnixListener;
use tokio::task::{JoinError, JoinSet};
use tracing::{error, info, warn};

use crate::dispatcher::ConnectionDispatcher;

/// Upper bound on the wait for aborted handlers to acknowledge cancellation.
const ABORT_REAP_BUDGET: Duration = Duration::from_millis(100);

/// Outcome of [`ConnectionTasks::drain`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DrainReport {
    /// Handlers that finished on their own within the budget.
    pub completed: usize,
    /// Handlers that panicked (reported via `tracing`, payload withheld).
    pub panicked: usize,
    /// Handlers still running when the budget expired, then aborted.
    pub aborted: usize,
}

impl DrainReport {
    /// Whether every handler finished on its own without panicking.
    #[must_use]
    pub const fn is_clean(&self) -> bool {
        self.panicked == 0 && self.aborted == 0
    }

    fn record(&mut self, outcome: TaskOutcome) {
        match outcome {
            TaskOutcome::Completed => self.completed = self.completed.saturating_add(1),
            TaskOutcome::Panicked => self.panicked = self.panicked.saturating_add(1),
            TaskOutcome::Aborted => self.aborted = self.aborted.saturating_add(1),
        }
    }
}

/// Result class of one joined handler task.
enum TaskOutcome {
    Completed,
    Panicked,
    Aborted,
}

/// Classifies a joined handler and reports a panic through `tracing`.
///
/// The `JoinError` is never formatted: its `Display` includes the panic payload.
fn classify(result: Result<(), JoinError>) -> TaskOutcome {
    match result {
        Ok(()) => TaskOutcome::Completed,
        Err(err) if err.is_panic() => {
            error!(
                "Connection handler panicked; connection closed without a verdict (fail-closed)"
            );
            TaskOutcome::Panicked
        }
        Err(_) => TaskOutcome::Aborted,
    }
}

/// Tracked set of connection handler tasks.
#[derive(Default)]
pub struct ConnectionTasks {
    set: JoinSet<()>,
}

impl ConnectionTasks {
    /// Creates an empty task set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Spawns one connection handler into the set.
    pub fn spawn<F>(&mut self, handler: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.set.spawn(handler);
    }

    /// Number of handlers not yet reaped.
    #[must_use]
    pub fn len(&self) -> usize {
        self.set.len()
    }

    /// Whether no handler is tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    /// Stops tracking new work and waits at most `budget` for the tracked handlers.
    ///
    /// Handlers still running when the budget expires are aborted (their clients see
    /// EOF). The wait for aborted handlers is itself bounded; any handler that ignores
    /// cancellation past that bound is detached-aborted when the set is dropped.
    pub async fn drain(mut self, budget: Duration) -> DrainReport {
        let in_flight = self.set.len();
        info!(
            in_flight = in_flight,
            budget_ms = u64::try_from(budget.as_millis()).unwrap_or(u64::MAX),
            "Draining in-flight connections before shutdown"
        );
        let mut report = DrainReport::default();

        let drained = tokio::time::timeout(budget, async {
            while let Some(result) = self.set.join_next().await {
                report.record(classify(result));
            }
        })
        .await;

        if drained.is_err() {
            warn!(
                remaining = self.set.len(),
                "Connection drain budget expired; aborting remaining handlers (fail-closed)"
            );
            self.set.abort_all();
            let _ = tokio::time::timeout(ABORT_REAP_BUDGET, async {
                while let Some(result) = self.set.join_next().await {
                    report.record(classify(result));
                }
            })
            .await;
            // Handlers that did not acknowledge cancellation in time are counted as
            // aborted; dropping the set aborts them.
            report.aborted = report.aborted.saturating_add(self.set.len());
        }

        info!(
            completed = report.completed,
            panicked = report.panicked,
            aborted = report.aborted,
            "Connection drain finished"
        );
        report
    }
}

/// Accepts connections into `tasks` until `shutdown` resolves.
///
/// Finished handlers are reaped as they complete (panics are reported via `tracing`).
/// Returns as soon as `shutdown` resolves; the caller then drops the listener, unlinks
/// the socket and calls [`ConnectionTasks::drain`].
pub async fn accept_until_shutdown<S>(
    listener: &UnixListener,
    dispatcher: Arc<ConnectionDispatcher>,
    tasks: &mut ConnectionTasks,
    shutdown: S,
) where
    S: Future,
{
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => return,
            Some(result) = tasks.set.join_next(), if !tasks.set.is_empty() => {
                let _ = classify(result);
            }
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, _addr)) => {
                        let disp = Arc::clone(&dispatcher);
                        tasks.spawn(async move {
                            if let Err(err) = disp.handle_connection(stream).await {
                                warn!(error = %err, "Connection handler finished with error");
                            }
                        });
                    }
                    Err(err) => {
                        error!(error = %err, "Accept failed");
                    }
                }
            }
        }
    }
}

/// Reports a panic through `tracing` with its location only (GitHub #259).
fn log_panic(location: Option<&Location<'_>>) {
    let thread = std::thread::current();
    let thread_name = thread.name().unwrap_or("unnamed");
    match location {
        Some(loc) => error!(
            file = loc.file(),
            line = loc.line(),
            thread = thread_name,
            "soos-daemon panicked; panic message withheld from logs"
        ),
        None => error!(
            thread = thread_name,
            "soos-daemon panicked; panic message withheld from logs"
        ),
    }
}

/// Replaces the default panic hook with a `tracing` report that never logs the payload.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| log_panic(info.location())));
}
