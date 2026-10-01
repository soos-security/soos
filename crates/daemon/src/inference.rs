//! Bounded CPU inference execution and per-request decision deadlines (GitHub #158, #159).
//!
//! - [`InferenceGate`]: CPU-heavy vision inference (SCRFD + PAD + SFace) never runs on a
//!   Tokio worker thread. Each inference is executed with `tokio::task::spawn_blocking` while
//!   holding one of [`MAX_CONCURRENT_INFERENCES`] semaphore permits, so the accept loop, Status
//!   requests and timers stay responsive and at most that many blocking jobs ever exist.
//! - [`RequestDeadline`]: one deadline computed once per request, bounded by the client deadline
//!   and by the outer `connection_timeout` measured from the start of request processing, each
//!   minus [`RESPONSE_WRITE_MARGIN_MS`] reserved for writing the response and the PAM read.
//! - [`InferenceEstimator`]: bounded moving average of measured inference latency; an inference
//!   is only started when the estimate fits in the remaining budget.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::pipeline::DECISION_BUDGET_MS;

/// Maximum number of vision inferences running at the same time (blocking pool slots).
///
/// One slot, as specified in `AI/ARCHITECTURE.md` (Async Boundaries): the ONNX Runtime sessions
/// are shared behind a mutex, so a second concurrent inference would only block another thread
/// on that mutex while adding latency to both requests.
pub const MAX_CONCURRENT_INFERENCES: usize = 1;

/// Time reserved at the end of every request budget for writing the response and for the PAM
/// module to read it before its own deadline (50ms).
pub const RESPONSE_WRITE_MARGIN_MS: u64 = 50;

/// Initial per-inference latency estimate used before any inference was measured (80ms).
///
/// SCRFD-500M + MiniFASNetV2 + SFace on a desktop CPU; replaced by the measured moving
/// average after the first inference.
pub const DEFAULT_INFERENCE_ESTIMATE_MS: u64 = 80;

/// Upper bound of the latency estimate (1000ms), so one pathological measurement (host
/// suspend, CPU starvation) cannot disable biometric authentication indefinitely.
pub const MAX_INFERENCE_ESTIMATE_MS: u64 = 1000;

/// Number of warm-up passes run by [`InferenceGate::warm_up`] at daemon start (GitHub #276).
///
/// The first pass pays the one-time ONNX Runtime allocations and is discarded; the estimate
/// is seeded with the last pass, which reflects steady-state latency.
pub const WARMUP_PASSES: usize = 2;

/// Lower bound of the latency estimate (1ms): an estimate of zero would let an inference start
/// with no remaining time at all.
const MIN_INFERENCE_ESTIMATE_MS: u64 = 1;

/// Converts a duration to whole microseconds, saturating at `u64::MAX`.
fn duration_to_micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

/// Clamps a microsecond estimate to `[MIN_INFERENCE_ESTIMATE_MS, MAX_INFERENCE_ESTIMATE_MS]`.
fn clamp_estimate_micros(micros: u64) -> u64 {
    micros.clamp(
        MIN_INFERENCE_ESTIMATE_MS.saturating_mul(1000),
        MAX_INFERENCE_ESTIMATE_MS.saturating_mul(1000),
    )
}

/// Single decision deadline of one authentication request (GitHub #159).
///
/// Computed once, when the request is decoded, and threaded through the camera wake wait and
/// the multi-frame consensus loop. It combines two bounds:
/// - the client deadline (`Request::deadline_monotonic_ns`, monotonic clock domain) minus
///   [`RESPONSE_WRITE_MARGIN_MS`], or [`DECISION_BUDGET_MS`] when the client sent none;
/// - the outer `connection_timeout`, measured from the start of request processing (`Instant`
///   domain, the clock used by the Tokio timeout), minus [`RESPONSE_WRITE_MARGIN_MS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestDeadline {
    deadline_ns: u64,
    outer_deadline: Instant,
}

impl RequestDeadline {
    /// Computes the request deadline.
    ///
    /// `client_deadline_ns` of `0` or `u64::MAX` means "no client deadline". A client deadline
    /// already within the write margin yields a deadline with no remaining budget.
    #[must_use]
    pub fn compute(
        now_ns: u64,
        client_deadline_ns: u64,
        request_started: Instant,
        connection_timeout: Duration,
    ) -> Self {
        let margin = Duration::from_millis(RESPONSE_WRITE_MARGIN_MS);
        let client_budget = if client_deadline_ns == 0 || client_deadline_ns == u64::MAX {
            Duration::from_millis(DECISION_BUDGET_MS)
        } else {
            Duration::from_nanos(client_deadline_ns.saturating_sub(now_ns)).saturating_sub(margin)
        };
        let deadline_ns =
            now_ns.saturating_add(u64::try_from(client_budget.as_nanos()).unwrap_or(u64::MAX));
        // Fail closed on `Instant` overflow: no outer budget at all.
        let outer_deadline = request_started
            .checked_add(connection_timeout.saturating_sub(margin))
            .unwrap_or(request_started);
        Self {
            deadline_ns,
            outer_deadline,
        }
    }

    /// Absolute deadline in the monotonic nanosecond clock domain (client bound only).
    #[must_use]
    pub const fn deadline_ns(&self) -> u64 {
        self.deadline_ns
    }

    /// Remaining budget at the given instants (the smaller of both bounds).
    #[must_use]
    pub fn remaining_at(&self, now_ns: u64, now: Instant) -> Duration {
        let client = Duration::from_nanos(self.deadline_ns.saturating_sub(now_ns));
        let outer = self.outer_deadline.saturating_duration_since(now);
        client.min(outer)
    }

    /// Remaining budget at `now_ns` and the current `Instant`.
    #[must_use]
    pub fn remaining(&self, now_ns: u64) -> Duration {
        self.remaining_at(now_ns, Instant::now())
    }

    /// Whether no budget remains at the given instants.
    #[must_use]
    pub fn is_expired_at(&self, now_ns: u64, now: Instant) -> bool {
        self.remaining_at(now_ns, now).is_zero()
    }

    /// Whether an inference of duration `estimate` started at the given instants would finish
    /// within the budget. Never true once the budget is exhausted.
    #[must_use]
    pub fn can_start_at(&self, now_ns: u64, now: Instant, estimate: Duration) -> bool {
        let remaining = self.remaining_at(now_ns, now);
        !remaining.is_zero() && remaining >= estimate
    }

    /// [`Self::can_start_at`] evaluated at `now_ns` and the current `Instant`.
    #[must_use]
    pub fn can_start(&self, now_ns: u64, estimate: Duration) -> bool {
        self.can_start_at(now_ns, Instant::now(), estimate)
    }
}

/// Bounded exponential moving average of measured inference latency (weight 1/4 per sample).
#[derive(Debug)]
pub struct InferenceEstimator {
    estimate_micros: AtomicU64,
}

impl InferenceEstimator {
    /// Creates an estimator seeded with `initial` (clamped to the estimate bounds).
    #[must_use]
    pub fn new(initial: Duration) -> Self {
        Self {
            estimate_micros: AtomicU64::new(clamp_estimate_micros(duration_to_micros(initial))),
        }
    }

    /// Current latency estimate.
    #[must_use]
    pub fn estimate(&self) -> Duration {
        Duration::from_micros(self.estimate_micros.load(Ordering::Relaxed))
    }

    /// Replaces the estimate with `measured` (clamped to the estimate bounds).
    ///
    /// Used once by the start-up warm-up (GitHub #276), so the first request is admitted
    /// against a measured latency rather than [`DEFAULT_INFERENCE_ESTIMATE_MS`].
    pub fn seed(&self, measured: Duration) {
        self.estimate_micros.store(
            clamp_estimate_micros(duration_to_micros(measured)),
            Ordering::Relaxed,
        );
    }

    /// Folds one measured inference duration into the estimate.
    pub fn record(&self, measured: Duration) {
        let sample = clamp_estimate_micros(duration_to_micros(measured));
        let _ =
            self.estimate_micros
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                    // Rounded up so the average converges to a constant sample, including the
                    // upper clamp, instead of stalling one microsecond below it.
                    let blended = current
                        .saturating_mul(3)
                        .saturating_add(sample)
                        .saturating_add(3)
                        / 4;
                    Some(clamp_estimate_micros(blended))
                });
    }
}

impl Default for InferenceEstimator {
    fn default() -> Self {
        Self::new(Duration::from_millis(DEFAULT_INFERENCE_ESTIMATE_MS))
    }
}

/// Failure of an inference job executed on the blocking pool; the request must fail closed.
///
/// Carries no panic payload, so nothing produced inside the vision stage is ever logged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InferenceJobError {
    /// The job panicked.
    #[error("inference job panicked")]
    Panicked,
    /// The job was cancelled by the runtime (shutdown).
    #[error("inference job was cancelled")]
    Cancelled,
}

/// Semaphore-guarded gateway to the Tokio blocking pool for vision inference (GitHub #158).
#[derive(Debug)]
pub struct InferenceGate {
    permits: Arc<Semaphore>,
    max_concurrent: usize,
    estimator: Arc<InferenceEstimator>,
}

impl InferenceGate {
    /// Creates a gate with `max_concurrent` slots (at least one) and an initial estimate.
    #[must_use]
    pub fn new(max_concurrent: usize, initial_estimate: Duration) -> Self {
        let max_concurrent = max_concurrent.max(1);
        Self {
            permits: Arc::new(Semaphore::new(max_concurrent)),
            max_concurrent,
            estimator: Arc::new(InferenceEstimator::new(initial_estimate)),
        }
    }

    /// Number of inference slots.
    #[must_use]
    pub const fn max_concurrent(&self) -> usize {
        self.max_concurrent
    }

    /// Number of currently free inference slots.
    #[must_use]
    pub fn available_permits(&self) -> usize {
        self.permits.available_permits()
    }

    /// Current per-inference latency estimate.
    #[must_use]
    pub fn estimate(&self) -> Duration {
        self.estimator.estimate()
    }

    /// Waits at most `max_wait` for a free inference slot.
    ///
    /// Returns `None` when no slot frees up in time: the caller must not queue the inference.
    pub async fn acquire_within(&self, max_wait: Duration) -> Option<OwnedSemaphorePermit> {
        if let Ok(permit) = self.permits.clone().try_acquire_owned() {
            return Some(permit);
        }
        if max_wait.is_zero() {
            return None;
        }
        match tokio::time::timeout(max_wait, self.permits.clone().acquire_owned()).await {
            Ok(Ok(permit)) => Some(permit),
            Ok(Err(_)) | Err(_) => None,
        }
    }

    /// Runs `job` on the blocking pool while holding `permit`.
    ///
    /// The permit moves into the blocking job, so the slot is released only when the job
    /// actually finishes, even if the awaiting request future is cancelled: a timed-out request
    /// leaves at most the single job it had already started, never a queue. The measured job
    /// duration updates the latency estimate.
    ///
    /// # Errors
    ///
    /// Returns [`InferenceJobError`] when the job panicked or was cancelled.
    pub async fn run<F, T>(
        &self,
        permit: OwnedSemaphorePermit,
        job: F,
    ) -> Result<T, InferenceJobError>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let estimator = self.estimator.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let started = Instant::now();
            let output = job();
            estimator.record(started.elapsed());
            output
        })
        .await
        .map_err(|err| {
            if err.is_panic() {
                InferenceJobError::Panicked
            } else {
                InferenceJobError::Cancelled
            }
        })
    }
}

impl InferenceGate {
    /// Runs `job` [`WARMUP_PASSES`] times on the blocking pool while holding an inference
    /// slot, then seeds the latency estimate with the last pass (GitHub #276).
    ///
    /// Called at daemon start, before the socket is bound, so no request competes for the
    /// slot. Returns the measured duration of the last pass. Job outputs are dropped unseen.
    ///
    /// # Errors
    ///
    /// Returns [`InferenceJobError`] when a pass panicked or was cancelled; the estimate is
    /// then left unchanged (the conservative default) and the slot is released.
    pub async fn warm_up<F, T>(&self, job: F) -> Result<Duration, InferenceJobError>
    where
        F: Fn() -> T + Send + 'static,
        T: Send + 'static,
    {
        let permit = self
            .permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| InferenceJobError::Cancelled)?;
        let measured = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut last = Duration::ZERO;
            for _ in 0..WARMUP_PASSES {
                let started = Instant::now();
                drop(job());
                last = started.elapsed();
            }
            last
        })
        .await
        .map_err(|err| {
            if err.is_panic() {
                InferenceJobError::Panicked
            } else {
                InferenceJobError::Cancelled
            }
        })?;
        self.estimator.seed(measured);
        Ok(measured)
    }
}

impl Default for InferenceGate {
    fn default() -> Self {
        Self::new(
            MAX_CONCURRENT_INFERENCES,
            Duration::from_millis(DEFAULT_INFERENCE_ESTIMATE_MS),
        )
    }
}
