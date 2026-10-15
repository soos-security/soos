//! Polling cadence, error back-off and the `--watch` loop.

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use crate::format::json;
use crate::shared::{boottime_now_ms, shared_fetch, Source};
use crate::view::{LockView, Presented};
use crate::{ERROR_BACKOFF_MS, EXCHANGE_BUDGET_MS, MAX_POLL_INTERVAL_MS, MIN_POLL_INTERVAL_MS};

/// Clamps a requested poll interval into `[MIN_POLL_INTERVAL_MS, MAX_POLL_INTERVAL_MS]`.
#[must_use]
pub fn clamp_interval_ms(requested: u64) -> u64 {
    requested.clamp(MIN_POLL_INTERVAL_MS, MAX_POLL_INTERVAL_MS)
}

/// Consecutive-failure back-off (`ERROR_BACKOFF_MS`).
#[derive(Debug, Clone, Default)]
pub struct Backoff {
    failures: u32,
}

impl Backoff {
    /// Delay before the next poll: `interval` after a success (which resets the back-off), else
    /// the larger of `interval` and the back-off step of this failure count.
    pub fn next_delay(&mut self, interval: Duration, ok: bool) -> Duration {
        if ok {
            self.failures = 0;
            return interval;
        }
        self.failures = self.failures.saturating_add(1);
        let step = usize::try_from(self.failures)
            .unwrap_or(usize::MAX)
            .min(ERROR_BACKOFF_MS.len())
            .saturating_sub(1);
        let backoff = ERROR_BACKOFF_MS.get(step).copied().unwrap_or(0);
        interval.max(Duration::from_millis(backoff))
    }
}

/// Polls the user's lock state through [`crate::shared::shared_fetch`].
#[derive(Debug)]
pub struct Poller {
    socket: PathBuf,
    runtime_dir: Option<PathBuf>,
    uid: u32,
    interval: Duration,
    backoff: Backoff,
    last: LockView,
}

impl Poller {
    /// A poller of `uid` through `socket`, coordinated in `runtime_dir`; the interval is clamped.
    #[must_use]
    pub fn new(socket: PathBuf, runtime_dir: Option<PathBuf>, uid: u32, interval_ms: u64) -> Self {
        Self {
            socket,
            runtime_dir,
            uid,
            interval: Duration::from_millis(clamp_interval_ms(interval_ms)),
            backoff: Backoff::default(),
            last: LockView::UNAVAILABLE,
        }
    }

    /// The clamped poll interval.
    #[must_use]
    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// One poll: the view and the delay before the next poll. A busy coordination lock returns
    /// the previous view (unavailable before the first answer).
    pub fn poll(&mut self) -> (LockView, Duration) {
        let outcome = shared_fetch(
            self.runtime_dir.as_deref(),
            &self.socket,
            self.uid,
            Duration::from_millis(EXCHANGE_BUDGET_MS),
            &boottime_now_ms,
        );
        // Only a failed daemon exchange backs off; a cached answer, a busy lock or a missing
        // runtime directory never counts as a daemon failure.
        let ok = outcome.source != Source::Daemon || outcome.view.available();
        let delay = self.backoff.next_delay(self.interval, ok);
        // A busy coordination lock obtained no answer at all: keep the last view (no
        // Scanning -> Idle -> Scanning flicker while another client of the user is polling).
        if outcome.source != Source::LockBusy {
            self.last = outcome.view;
        }
        (self.last, delay)
    }
}

/// Whether the reader of `fd` went away (`POLLERR`, `POLLHUP` or `POLLNVAL` reported without
/// waiting). Lets `--watch` stop polling the daemon when its reader closed the pipe even while
/// the state is stable and nothing is written. A failed `poll` counts as closed (fail quiet).
#[must_use]
pub fn output_closed(fd: std::os::fd::BorrowedFd<'_>) -> bool {
    use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
    let mut fds = [PollFd::new(fd, PollFlags::empty())];
    match poll(&mut fds, PollTimeout::ZERO) {
        Ok(_) => fds
            .first()
            .and_then(|p| p.revents())
            .is_some_and(|revents| {
                revents.intersects(PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL)
            }),
        Err(_) => true,
    }
}

/// `--watch`: writes one JSON line for the first view and for every change of the presented
/// state, availability or sequence, sleeping the poller delay in between.
///
/// # Errors
///
/// Returns the first write error (for example a closed pipe); `Ok(())` once `keep_running`
/// returns `false`.
pub fn watch<W: Write>(
    poller: &mut Poller,
    out: &mut W,
    keep_running: &dyn Fn() -> bool,
) -> std::io::Result<()> {
    let mut last: Option<(Presented, bool, u64)> = None;
    while keep_running() {
        let (view, delay) = poller.poll();
        let key = (view.presented(), view.available(), view.sequence);
        if last != Some(key) {
            let mut line = json(&view);
            line.push('\n');
            out.write_all(line.as_bytes())?;
            out.flush()?;
            last = Some(key);
        }
        std::thread::sleep(delay);
    }
    Ok(())
}
