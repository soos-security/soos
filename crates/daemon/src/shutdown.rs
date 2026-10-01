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
//! that records the source location only. [`install_panic_hook_with`] selects the message
//! policy: debug builds may log the (bounded) panic message, release builds never do
//! (GitHub #287, owner decision 2026-10-01).
//!
//! GitHub #287 follow-ups:
//! - a failed `accept()` (`EMFILE`, `ENFILE`, `ENOBUFS`, `ENOMEM`, ...) is retried after a
//!   bounded exponential [`AcceptBackoff`] instead of a hot loop; the shutdown signal
//!   interrupts the backoff sleep;
//! - blocking side-effect jobs (spoof evidence writes) run in [`BlockingTasks`] and are
//!   drained at shutdown within what is left of the drain budget;
//! - GitHub #289: the runtime is shut down with [`shutdown_runtime`] (`Runtime::shutdown_timeout`)
//!   with what is still left of that budget, so an abandoned blocking job no longer delays exit.

use std::future::Future;
use std::io;
use std::panic::Location;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
use tokio::net::{UnixListener, UnixStream};
use tokio::runtime::Runtime;
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
/// the socket and calls [`ConnectionTasks::drain`]. A failed `accept()` is retried after
/// the default [`AcceptBackoff`] (GitHub #287).
pub async fn accept_until_shutdown<S>(
    listener: &UnixListener,
    dispatcher: Arc<ConnectionDispatcher>,
    tasks: &mut ConnectionTasks,
    shutdown: S,
) where
    S: Future,
{
    let accept = move || async move { listener.accept().await.map(|(stream, _addr)| stream) };
    accept_with_backoff(
        accept,
        dispatcher,
        tasks,
        AcceptBackoff::default(),
        shutdown,
    )
    .await;
}

/// First delay after a failed `accept()` (GitHub #287).
pub const ACCEPT_BACKOFF_INITIAL: Duration = Duration::from_millis(5);

/// Upper bound of the `accept()` backoff delay (GitHub #287).
pub const ACCEPT_BACKOFF_MAX: Duration = Duration::from_secs(1);

/// Smallest delay ever applied, so a zero configuration cannot become a busy loop.
const ACCEPT_BACKOFF_FLOOR: Duration = Duration::from_millis(1);

/// Bounded exponential backoff applied after a failed `accept()` (GitHub #287).
///
/// Resource exhaustion errors (`EMFILE`, `ENFILE`, `ENOBUFS`, `ENOMEM`) make `accept()`
/// fail immediately and repeatedly; retrying at once would spin a core and flood the logs.
/// The delay starts at `initial`, doubles after each consecutive error, is capped at `max`,
/// and is reset by a successful accept. It never stops the accept loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcceptBackoff {
    initial: Duration,
    max: Duration,
    next: Duration,
}

impl AcceptBackoff {
    /// Creates a backoff; `initial` is raised to 1 ms, and `max` to `initial` if lower.
    #[must_use]
    pub fn new(initial: Duration, max: Duration) -> Self {
        let initial = initial.max(ACCEPT_BACKOFF_FLOOR);
        let max = max.max(initial);
        Self {
            initial,
            max,
            next: initial,
        }
    }

    /// Records a failed accept and returns the delay to wait before the next attempt.
    pub fn on_error(&mut self) -> Duration {
        let delay = self.next;
        self.next = self.next.saturating_mul(2).min(self.max);
        delay
    }

    /// Records a successful accept: the next error starts again from `initial`.
    pub fn on_success(&mut self) {
        self.next = self.initial;
    }
}

impl Default for AcceptBackoff {
    fn default() -> Self {
        Self::new(ACCEPT_BACKOFF_INITIAL, ACCEPT_BACKOFF_MAX)
    }
}

/// Accept loop behind [`accept_until_shutdown`], with an injectable `accept` source.
///
/// After a failed `accept`, the loop waits for the delay returned by
/// [`AcceptBackoff::on_error`] before the next attempt; `shutdown` interrupts that wait,
/// and finished handlers keep being reaped during it. The loop only returns when
/// `shutdown` resolves.
pub async fn accept_with_backoff<A, F, S>(
    mut accept: A,
    dispatcher: Arc<ConnectionDispatcher>,
    tasks: &mut ConnectionTasks,
    mut backoff: AcceptBackoff,
    shutdown: S,
) where
    A: FnMut() -> F,
    F: Future<Output = io::Result<UnixStream>>,
    S: Future,
{
    tokio::pin!(shutdown);
    let mut pause: Option<Duration> = None;
    loop {
        if let Some(delay) = pause.take() {
            let sleep = tokio::time::sleep(delay);
            tokio::pin!(sleep);
            loop {
                tokio::select! {
                    biased;
                    _ = &mut shutdown => return,
                    Some(result) = tasks.set.join_next(), if !tasks.set.is_empty() => {
                        let _ = classify(result);
                    }
                    () = &mut sleep => break,
                }
            }
        }
        tokio::select! {
            biased;
            _ = &mut shutdown => return,
            Some(result) = tasks.set.join_next(), if !tasks.set.is_empty() => {
                let _ = classify(result);
            }
            accept_result = accept() => {
                match accept_result {
                    Ok(stream) => {
                        backoff.on_success();
                        let disp = Arc::clone(&dispatcher);
                        tasks.spawn(async move {
                            if let Err(err) = disp.handle_connection(stream).await {
                                warn!(error = %err, "Connection handler finished with error");
                            }
                        });
                    }
                    Err(err) => {
                        let delay = backoff.on_error();
                        error!(
                            error = %err,
                            retry_in_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
                            "Accept failed; backing off before the next attempt"
                        );
                        pause = Some(delay);
                    }
                }
            }
        }
    }
}

/// Classifies a joined blocking job; a panic is reported without its payload.
fn classify_blocking(result: Result<(), JoinError>) -> TaskOutcome {
    match result {
        Ok(()) => TaskOutcome::Completed,
        Err(err) if err.is_panic() => {
            error!("Blocking background job panicked; panic message withheld from logs");
            TaskOutcome::Panicked
        }
        Err(_) => TaskOutcome::Aborted,
    }
}

/// Tracked set of blocking side-effect jobs, such as spoof evidence writes (GitHub #287).
///
/// A job runs on the Tokio blocking pool like a detached `spawn_blocking`, but stays
/// tracked so that shutdown can wait for it. Finished jobs are reaped on every spawn, so
/// the set is bounded by the number of jobs actually running. A blocking job cannot be
/// cancelled: [`BlockingTasks::drain`] waits at most its budget and reports the jobs still
/// running at the deadline as `aborted` (abandoned: no longer awaited, and not awaited by
/// [`shutdown_runtime`] either once its budget is spent, GitHub #289).
#[derive(Default)]
pub struct BlockingTasks {
    set: Mutex<JoinSet<()>>,
}

impl BlockingTasks {
    /// Creates an empty job set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, JoinSet<()>> {
        // The guarded set holds no invariant that a panicking holder could break.
        self.set.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs `job` on the blocking pool and tracks it. Must be called inside a Tokio runtime.
    pub fn spawn_blocking<F>(&self, job: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let mut set = self.lock();
        while let Some(result) = set.try_join_next() {
            let _ = classify_blocking(result);
        }
        set.spawn_blocking(job);
    }

    /// Number of tracked jobs not yet reaped.
    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.lock().len()
    }

    /// Waits at most `budget` for every tracked job.
    ///
    /// Jobs spawned after the call starts are tracked in a fresh set and not awaited.
    pub async fn drain(&self, budget: Duration) -> DrainReport {
        let mut set = std::mem::take(&mut *self.lock());
        let mut report = DrainReport::default();
        if set.is_empty() {
            return report;
        }
        info!(
            in_flight = set.len(),
            budget_ms = u64::try_from(budget.as_millis()).unwrap_or(u64::MAX),
            "Waiting for in-flight background writes before shutdown"
        );
        let drained = tokio::time::timeout(budget, async {
            while let Some(result) = set.join_next().await {
                report.record(classify_blocking(result));
            }
        })
        .await;
        if drained.is_err() {
            report.aborted = report.aborted.saturating_add(set.len());
            warn!(
                remaining = set.len(),
                "Background write drain budget expired; abandoning the remaining writes"
            );
            // A running blocking job cannot be cancelled: detach it instead.
            set.detach_all();
        }
        info!(
            completed = report.completed,
            panicked = report.panicked,
            abandoned = report.aborted,
            "Background write drain finished"
        );
        report
    }
}

/// What is left at `now` of a shutdown `budget` that started at `started` (GitHub #289).
///
/// Saturates at zero once the budget is spent and never exceeds `budget` (a `now` earlier
/// than `started` counts as no time elapsed).
#[must_use]
pub fn remaining_budget(started: Instant, budget: Duration, now: Instant) -> Duration {
    budget.saturating_sub(now.saturating_duration_since(started))
}

/// Shuts the Tokio runtime down, waiting at most `remaining` for its blocking jobs
/// (GitHub #289).
///
/// Dropping a runtime waits for every `spawn_blocking` job without bound, so a blocking
/// write abandoned by [`BlockingTasks::drain`] (or an inference left running by an aborted
/// handler) would still delay process exit. [`Runtime::shutdown_timeout`] cancels the async
/// tasks and leaves the blocking jobs still running after `remaining` behind: they are not
/// awaited and end with the process. This is what bounds exit by the drain budget.
pub fn shutdown_runtime(runtime: Runtime, remaining: Duration) {
    info!(
        budget_ms = u64::try_from(remaining.as_millis()).unwrap_or(u64::MAX),
        "Shutting down the runtime; blocking jobs still running after the budget are not awaited"
    );
    runtime.shutdown_timeout(remaining);
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

/// Upper bound, in characters, of a panic message logged by a debug build (GitHub #287).
pub const MAX_DEBUG_PANIC_MESSAGE_CHARS: usize = 512;

/// What the process panic hook logs besides the location (GitHub #287).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanicMessagePolicy {
    /// Location and thread only; the panic message is withheld (release builds, GitHub #259).
    Withhold,
    /// Location, thread and the panic message truncated to
    /// [`MAX_DEBUG_PANIC_MESSAGE_CHARS`] characters (debug builds only).
    LogMessage,
}

impl PanicMessagePolicy {
    /// Policy for a build with or without `debug_assertions`.
    #[must_use]
    pub const fn for_debug_assertions(debug_assertions: bool) -> Self {
        if debug_assertions {
            Self::LogMessage
        } else {
            Self::Withhold
        }
    }

    /// Policy of the current build: always [`PanicMessagePolicy::Withhold`] in release.
    #[must_use]
    pub const fn for_build() -> Self {
        Self::for_debug_assertions(cfg!(debug_assertions))
    }
}

/// Returns the panic message truncated to [`MAX_DEBUG_PANIC_MESSAGE_CHARS`] characters.
fn bounded_panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    let message = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("<non-string panic payload>");
    message
        .chars()
        .take(MAX_DEBUG_PANIC_MESSAGE_CHARS)
        .collect()
}

/// Debug-build panic report: location, thread and the bounded panic message.
fn log_panic_with_message(location: Option<&Location<'_>>, message: &str) {
    let thread = std::thread::current();
    let thread_name = thread.name().unwrap_or("unnamed");
    match location {
        Some(loc) => error!(
            file = loc.file(),
            line = loc.line(),
            thread = thread_name,
            panic_message = message,
            "soos-daemon panicked (debug build: panic message included)"
        ),
        None => error!(
            thread = thread_name,
            panic_message = message,
            "soos-daemon panicked (debug build: panic message included)"
        ),
    }
}

/// Replaces the default panic hook with a `tracing` report that never logs the payload.
///
/// This is the release behaviour of GitHub #259, unchanged.
pub fn install_panic_hook() {
    install_panic_hook_with(PanicMessagePolicy::Withhold);
}

/// Replaces the default panic hook with a `tracing` report following `policy`.
pub fn install_panic_hook_with(policy: PanicMessagePolicy) {
    match policy {
        PanicMessagePolicy::Withhold => {
            std::panic::set_hook(Box::new(|info| log_panic(info.location())));
        }
        PanicMessagePolicy::LogMessage => {
            std::panic::set_hook(Box::new(|info| {
                let message = bounded_panic_message(info.payload());
                log_panic_with_message(info.location(), &message);
            }));
        }
    }
}
