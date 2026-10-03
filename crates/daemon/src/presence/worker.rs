//! Presence worker: one tick = one decision (GitHub #323, spec §2.6 tick algorithm).
//!
//! Every step that fails ends the tick without an unlock. `UnlockSession` is reached only
//! from a consensus `Allow` followed by a fresh logind re-check of the candidate session, a
//! fresh account check and a kill-switch re-check, within `MAX_ALLOW_TO_UNLOCK_MS`.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::watch;
use tracing::{debug, info, warn};

use super::account::{AccountGuard, AccountRefusal, AccountState, UserName};
use super::config::PresenceConfig;
use super::display::{DisplayProbe, DisplayState};
use super::logind::{LogindSessionState, PresenceLogind, PresenceLogindError, SessionId};
use super::switch::PresenceSwitch;
use super::tracker::{select_candidate, LockEntry, LockTracker};
use super::{
    reconnect_backoff, ACCOUNT_CHECK_TIMEOUT_MS, DBUS_CALL_TIMEOUT_MS, DBUS_CONNECT_TIMEOUT_MS,
    LOCK_POLL_INTERVAL_MS, MAX_ALLOW_TO_UNLOCK_MS, MAX_PRESENCE_SEAT_SESSIONS,
    PRESENCE_RESERVED_ATTEMPTS, PRESENCE_WAKE_SETTLE_MS,
};
use crate::consensus::{
    run_face_consensus, wake_camera, ConsensusContext, ConsensusRun, MAX_CAMERA_WAKE_WAIT_MS,
};
use crate::error::DaemonError;
use crate::inference::{
    InferenceGate, InferencePriority, RequestDeadline, RESPONSE_WRITE_MARGIN_MS,
};
use crate::pipeline::{
    classify_template, current_monotonic_nanos, current_monotonic_nanos_from_clock,
    PipelineComponents, TemplateModelBinding, DECISION_BUDGET_MS,
};
use soos_biometric_store::BiometricTemplate;
use soos_policy::ConsensusDecision;
use soos_protocol::types::ReasonClass;

/// Why a tick ended without a scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// `/etc/soos/disabled` or `/etc/soos/presence.disable` (or a stat error).
    KillSwitch,
    /// logind could not be queried.
    LogindUnavailable,
    /// Too many logind sessions.
    TooManySessions,
    /// Too many locked sessions to track.
    TrackerOverflow,
    /// No bound, locked session.
    NoLockedSession,
    /// The lock grace has not elapsed.
    InGrace,
    /// The last scan is more recent than the scan interval (or the session is waiting for
    /// an unlock confirmation, or its locker ignored an unlock).
    NotDue,
    /// The session owner has no template, or the store holds none at all and logind is
    /// not polled (GitHub #325).
    NotEnrolled,
    /// The template belongs to another embedding model.
    ForeignTemplate,
    /// The template could not be read.
    TemplateStoreError,
    /// No candidate is left.
    NoCandidate,
    /// More than one candidate (D5).
    AmbiguousCandidates,
    /// logind reports the lid closed.
    LidClosed,
    /// Every connected display is DPMS-off.
    DisplayOff,
    /// A PAM request holds the inference priority.
    InteractiveDemand,
    /// The shared rate limit keeps its reserve for PAM.
    RateLimited,
    /// The monotonic clock failed.
    ClockUnavailable,
    /// Inside the logind reconnect backoff window.
    Backoff,
    /// The account guard refused the owner (or could not determine its state).
    AccountRefused,
    /// The daemon is stopping: no new attempt is started.
    ShuttingDown,
}

impl SkipReason {
    /// Stable, value-free code for logs (snake case of the variant).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::KillSwitch => "kill_switch",
            Self::LogindUnavailable => "logind_unavailable",
            Self::TooManySessions => "too_many_sessions",
            Self::TrackerOverflow => "tracker_overflow",
            Self::NoLockedSession => "no_locked_session",
            Self::InGrace => "in_grace",
            Self::NotDue => "not_due",
            Self::NotEnrolled => "not_enrolled",
            Self::ForeignTemplate => "foreign_template",
            Self::TemplateStoreError => "template_store_error",
            Self::NoCandidate => "no_candidate",
            Self::AmbiguousCandidates => "ambiguous_candidates",
            Self::LidClosed => "lid_closed",
            Self::DisplayOff => "display_off",
            Self::InteractiveDemand => "interactive_demand",
            Self::RateLimited => "rate_limited",
            Self::ClockUnavailable => "clock_unavailable",
            Self::Backoff => "backoff",
            Self::AccountRefused => "account_refused",
            Self::ShuttingDown => "shutting_down",
        }
    }
}

/// Result of a scan that started (an attempt was recorded).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanOutcome {
    /// `UnlockSession` succeeded.
    Unlocked {
        /// Unlocked session.
        session: SessionId,
        /// Its owner.
        uid: u32,
    },
    /// The consensus did not reach `Allow`.
    NoMatch,
    /// A capture was classified as a presentation attack.
    SpoofVetoed,
    /// A PAM request appeared during the scan.
    Preempted,
    /// The camera did not become ready.
    CameraUnavailable,
    /// The consensus aborted (clock, inference job or vision failure).
    Aborted(ReasonClass),
    /// The re-check refused (gone, unlocked, unbound, other UID or name, other ID, error).
    SessionChanged,
    /// More than `MAX_ALLOW_TO_UNLOCK_MS` passed since the `Allow` (or the clock failed).
    AllowExpired,
    /// `UnlockSession` failed or timed out.
    UnlockFailed,
    /// The fresh account check after the `Allow` refused.
    AccountRefused(AccountRefusal),
    /// The kill switch was engaged between the start of the tick and the unlock call.
    KillSwitchEngaged,
    /// The daemon started stopping during the scan; nothing is unlocked.
    ShuttingDown,
    /// logind reported the lid closed at the re-check after the `Allow`.
    LidClosed,
}

/// Result of one tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresenceTick {
    /// No scan.
    Skipped(SkipReason),
    /// A scan started.
    Scanned(ScanOutcome),
}

/// The one candidate of a tick (template held only until the consensus returns).
struct Candidate {
    id: SessionId,
    entry: LockEntry,
    user_name: UserName,
    template: BiometricTemplate,
}

/// Releases the "account check in flight" flag when the blocking check ends.
struct InFlight(Arc<AtomicBool>);

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Converts milliseconds to nanoseconds, saturating.
const fn ms_to_ns(ms: u64) -> u64 {
    ms.saturating_mul(1_000_000)
}

/// `CLOCK_BOOTTIME` in nanoseconds: unlike `CLOCK_MONOTONIC` it keeps counting during
/// system suspend, so a suspend between the `Allow` and the unlock expires the `Allow`.
fn boot_nanos() -> Result<u64, DaemonError> {
    current_monotonic_nanos_from_clock(nix::time::ClockId::CLOCK_BOOTTIME)
}

/// Runs `future` under the logind call bound.
async fn bounded<T>(
    future: impl Future<Output = Result<T, PresenceLogindError>>,
) -> Result<T, PresenceLogindError> {
    match tokio::time::timeout(Duration::from_millis(DBUS_CALL_TIMEOUT_MS), future).await {
        Ok(result) => result,
        Err(_) => Err(PresenceLogindError::Timeout),
    }
}

/// The presence auto-unlock worker.
pub struct PresenceWorker<L: PresenceLogind, D: DisplayProbe, A: AccountGuard> {
    config: PresenceConfig,
    logind: L,
    display: D,
    switch: PresenceSwitch,
    account_guard: Arc<A>,
    account_check_in_flight: Arc<AtomicBool>,
    pipeline: PipelineComponents,
    inference: InferenceGate,
    expected_model: Option<String>,
    clock_fn: fn() -> Result<u64, DaemonError>,
    boot_clock_fn: fn() -> Result<u64, DaemonError>,
    tracker: LockTracker,
    consecutive_failures: u32,
    backoff_until: Option<Instant>,
    last_skip: Option<SkipReason>,
    gate_open: bool,
    logind_down: bool,
    last_spoof_warning: Option<(SessionId, u64)>,
    shutdown: Option<watch::Receiver<bool>>,
}

impl<L: PresenceLogind, D: DisplayProbe, A: AccountGuard> std::fmt::Debug
    for PresenceWorker<L, D, A>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PresenceWorker")
            .field("config", &self.config)
            .field("tracker", &self.tracker)
            .field("consecutive_failures", &self.consecutive_failures)
            .finish_non_exhaustive()
    }
}

impl<L: PresenceLogind, D: DisplayProbe, A: AccountGuard> PresenceWorker<L, D, A> {
    /// Builds a worker; the production clock is `CLOCK_MONOTONIC`, the clock of the shared
    /// rate limiter.
    #[must_use]
    pub fn new(
        config: PresenceConfig,
        logind: L,
        display: D,
        switch: PresenceSwitch,
        account_guard: A,
        pipeline: PipelineComponents,
        inference: InferenceGate,
    ) -> Self {
        Self {
            config,
            logind,
            display,
            switch,
            account_guard: Arc::new(account_guard),
            account_check_in_flight: Arc::new(AtomicBool::new(false)),
            pipeline,
            inference,
            expected_model: None,
            clock_fn: current_monotonic_nanos,
            boot_clock_fn: boot_nanos,
            tracker: LockTracker::default(),
            consecutive_failures: 0,
            backoff_until: None,
            last_skip: None,
            gate_open: true,
            logind_down: false,
            last_spoof_warning: None,
            shutdown: None,
        }
    }

    /// Refuses templates of another embedding model (same rule as dispatcher Step 8b).
    #[must_use]
    pub fn with_expected_embedding_model(mut self, id: impl Into<String>) -> Self {
        self.expected_model = Some(id.into());
        self
    }

    /// Replaces the monotonic clock (test hook).
    #[must_use]
    pub fn with_clock_fn(mut self, clock_fn: fn() -> Result<u64, DaemonError>) -> Self {
        self.clock_fn = clock_fn;
        self
    }

    /// Replaces the boot clock (test hook; production reads `CLOCK_BOOTTIME`).
    #[must_use]
    pub fn with_boot_clock_fn(mut self, boot_clock_fn: fn() -> Result<u64, DaemonError>) -> Self {
        self.boot_clock_fn = boot_clock_fn;
        self
    }

    /// Ticks every `LOCK_POLL_INTERVAL_MS` until `shutdown` turns `true`.
    pub async fn run(mut self, mut shutdown: watch::Receiver<bool>) {
        self.shutdown = Some(shutdown.clone());
        loop {
            if *shutdown.borrow() {
                break;
            }
            let _ = self.tick().await;
            if *shutdown.borrow() {
                break;
            }
            tokio::select! {
                () = tokio::time::sleep(Duration::from_millis(LOCK_POLL_INTERVAL_MS)) => {}
                changed = shutdown.changed() => {
                    if changed.is_err() {
                        break;
                    }
                }
            }
        }
        debug!("Presence worker stopped");
    }

    /// Whether the daemon asked the worker to stop.
    fn stop_requested(&self) -> bool {
        self.shutdown.as_ref().is_some_and(|rx| *rx.borrow())
    }

    /// Ends a tick without a scan (debug log on change of reason only).
    fn skip(&mut self, reason: SkipReason) -> PresenceTick {
        if self.last_skip != Some(reason) {
            debug!(reason = reason.as_str(), "Presence tick skipped");
            self.last_skip = Some(reason);
        }
        PresenceTick::Skipped(reason)
    }

    /// Ends a tick that started a scan.
    fn scanned(&mut self, outcome: ScanOutcome) -> PresenceTick {
        self.last_skip = None;
        debug!(outcome = ?outcome, "Presence scan finished");
        PresenceTick::Scanned(outcome)
    }

    /// Extends the exponential reconnect backoff by one failure.
    fn start_backoff(&mut self) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        self.backoff_until =
            Instant::now().checked_add(reconnect_backoff(self.consecutive_failures));
    }

    /// Records one logind snapshot failure (backoff, warn on transition only).
    fn logind_failed(&mut self, err: &PresenceLogindError) {
        self.start_backoff();
        if !self.logind_down {
            warn!(
                error = %err,
                "systemd-logind unavailable; presence auto-unlock backs off (sessions stay locked)"
            );
            self.logind_down = true;
        }
    }

    /// Records a successful logind snapshot (backoff reset).
    fn logind_succeeded(&mut self) {
        self.consecutive_failures = 0;
        self.backoff_until = None;
        if self.logind_down {
            info!("systemd-logind reachable again; presence auto-unlock resumed");
            self.logind_down = false;
        }
    }

    /// Account check on the blocking pool, bounded by `ACCOUNT_CHECK_TIMEOUT_MS`, with at
    /// most one check in flight (a still-running check makes the next one undeterminable).
    async fn check_account(&self, user: &UserName, uid: u32) -> AccountState {
        let undeterminable = AccountState::Refused(AccountRefusal::Undeterminable);
        if self.account_check_in_flight.swap(true, Ordering::SeqCst) {
            return undeterminable;
        }
        let in_flight = InFlight(Arc::clone(&self.account_check_in_flight));
        let guard = Arc::clone(&self.account_guard);
        let user = user.clone();
        let job = tokio::task::spawn_blocking(move || {
            let _in_flight = in_flight;
            guard.check(&user, uid)
        });
        match tokio::time::timeout(Duration::from_millis(ACCOUNT_CHECK_TIMEOUT_MS), job).await {
            Ok(Ok(state)) => state,
            Ok(Err(_)) | Err(_) => undeterminable,
        }
    }

    /// Logs lid/screen gate transitions at info (value-free).
    fn note_gate(
        &mut self,
        open: bool,
        lid: &Result<bool, PresenceLogindError>,
        screen: DisplayState,
    ) {
        if open == self.gate_open {
            return;
        }
        self.gate_open = open;
        let lid_closed = match lid {
            Ok(true) => "true",
            Ok(false) => "false",
            Err(_) => "unknown",
        };
        if open {
            info!(
                lid_closed,
                display_state = screen.as_str(),
                "Presence scan gate open"
            );
        } else {
            info!(
                lid_closed,
                display_state = screen.as_str(),
                "Presence scan gated by the lid or the screen state"
            );
        }
    }

    /// Step 7: store, model binding and account guard for every due session, then D5.
    async fn select(
        &mut self,
        due: Vec<(SessionId, LockEntry)>,
        snapshot: &[LogindSessionState],
    ) -> Result<Candidate, SkipReason> {
        let mut last_drop = SkipReason::NoCandidate;
        let mut kept: Vec<Candidate> = Vec::new();
        for (id, entry) in due {
            let template = match self.pipeline.biometric_store.get(entry.uid) {
                Ok(Some(template)) => template,
                Ok(None) => {
                    last_drop = SkipReason::NotEnrolled;
                    continue;
                }
                Err(_) => {
                    last_drop = SkipReason::TemplateStoreError;
                    continue;
                }
            };
            if let Some(expected) = self.expected_model.as_deref() {
                let binding = classify_template(
                    expected,
                    self.pipeline.vision.embedding_dimension(),
                    &template.model_id,
                    template.embedding.len(),
                );
                if binding == TemplateModelBinding::Foreign {
                    last_drop = SkipReason::ForeignTemplate;
                    continue;
                }
            }
            let Some(user_name) = snapshot
                .iter()
                .find(|state| state.id == id)
                .and_then(|state| state.user_name.clone())
            else {
                last_drop = SkipReason::AccountRefused;
                continue;
            };
            if self.check_account(&user_name, entry.uid).await != AccountState::Usable {
                last_drop = SkipReason::AccountRefused;
                continue;
            }
            kept.push(Candidate {
                id,
                entry,
                user_name,
                template,
            });
        }
        if kept.is_empty() {
            return Err(last_drop);
        }
        let ids: Vec<(SessionId, LockEntry)> = kept
            .iter()
            .map(|candidate| (candidate.id.clone(), candidate.entry.clone()))
            .collect();
        select_candidate(ids)?;
        kept.pop().ok_or(SkipReason::NoCandidate)
    }

    /// One deterministic iteration.
    pub async fn tick(&mut self) -> PresenceTick {
        // 1. Kill switch first (before any clock, D-Bus, store or camera access).
        if self.switch.is_engaged() {
            self.tracker.clear();
            return self.skip(SkipReason::KillSwitch);
        }
        // 2. Reconnect backoff.
        if self
            .backoff_until
            .is_some_and(|until| Instant::now() < until)
        {
            return self.skip(SkipReason::Backoff);
        }
        // 3. Clock.
        let Ok(now_ns) = (self.clock_fn)() else {
            return self.skip(SkipReason::ClockUnavailable);
        };
        // 3b. Enrollment probe, before any D-Bus traffic (connection included): while the
        // store holds no template, logind is not polled. Re-evaluated every tick, so a new
        // enrollment is picked up within one tick. The tracker is cleared because sessions
        // are not observed while skipping (the grace restarts after the next enrollment).
        match self.pipeline.biometric_store.has_enrolled_template() {
            Ok(true) => {}
            Ok(false) => {
                self.tracker.clear();
                return self.skip(SkipReason::NotEnrolled);
            }
            Err(_) => {
                self.tracker.clear();
                return self.skip(SkipReason::TemplateStoreError);
            }
        }
        // 4a. Connection, under its own bound and outside the call bound.
        let connected = match tokio::time::timeout(
            Duration::from_millis(DBUS_CONNECT_TIMEOUT_MS),
            self.logind.connect(),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(PresenceLogindError::Timeout),
        };
        if let Err(err) = connected {
            self.logind_failed(&err);
            self.tracker.clear();
            return self.skip(SkipReason::LogindUnavailable);
        }
        // 4b. logind snapshot.
        let snapshot = match bounded(self.logind.seat_sessions()).await {
            Ok(snapshot) => snapshot,
            Err(err) => {
                self.logind_failed(&err);
                self.tracker.clear();
                let reason = if err == PresenceLogindError::TooManySessions {
                    SkipReason::TooManySessions
                } else {
                    SkipReason::LogindUnavailable
                };
                return self.skip(reason);
            }
        };
        self.logind_succeeded();
        if snapshot.len() > MAX_PRESENCE_SEAT_SESSIONS {
            self.tracker.clear();
            return self.skip(SkipReason::TooManySessions);
        }
        // 5. Lock periods.
        if let Err(reason) = self.tracker.observe(now_ns, &snapshot) {
            return self.skip(reason);
        }
        for id in self.tracker.expire_unconfirmed_unlocks(now_ns) {
            warn!(
                session_id = id.as_str(),
                "The screen locker ignored UnlockSession; this lock period is no longer scanned"
            );
        }
        if self.tracker.is_empty() {
            return self.skip(SkipReason::NoLockedSession);
        }
        // 6. Grace and scan interval.
        let due = self
            .tracker
            .due(now_ns, self.config.lock_grace, self.config.scan_interval);
        if due.is_empty() {
            let reason = if self.tracker.any_in_grace(now_ns, self.config.lock_grace) {
                SkipReason::InGrace
            } else {
                SkipReason::NotDue
            };
            return self.skip(reason);
        }
        // 7. Enrollment, model binding, account guard, exactly one candidate.
        let candidate = match self.select(due, &snapshot).await {
            Ok(candidate) => candidate,
            Err(reason) => return self.skip(reason),
        };
        // 8. Lid and screen gating ("when detectable").
        let lid = bounded(self.logind.lid_closed()).await;
        let screen = self.display.display_state();
        let gated = if lid == Ok(true) {
            Some(SkipReason::LidClosed)
        } else if screen == DisplayState::Off {
            Some(SkipReason::DisplayOff)
        } else {
            None
        };
        self.note_gate(gated.is_none(), &lid, screen);
        if let Some(reason) = gated {
            return self.skip(reason);
        }
        // 9. PAM priority.
        if self.inference.interactive_demand() > 0 {
            return self.skip(SkipReason::InteractiveDemand);
        }
        if self.stop_requested() {
            return self.skip(SkipReason::ShuttingDown);
        }
        // 10. One attempt in the shared limiter, keeping the PAM reserve, stamped with a
        // clock read taken under the policy write lock, right before recording.
        let policy = Arc::clone(&self.pipeline.policy);
        let mut engine = policy.write().await;
        let attempt_ns = match (self.clock_fn)() {
            Ok(ns) if ns >= now_ns => ns,
            // Clock error, or a reading older than step 3: no attempt, no scan.
            Ok(_) | Err(_) => {
                drop(engine);
                return self.skip(SkipReason::ClockUnavailable);
            }
        };
        let recorded = engine.record_attempt_with_reserve(
            candidate.entry.uid,
            attempt_ns,
            PRESENCE_RESERVED_ATTEMPTS,
        );
        drop(engine);
        if recorded.is_err() {
            return self.skip(SkipReason::RateLimited);
        }
        self.tracker.mark_scan_started(&candidate.id, attempt_ns);
        let outcome = self.scan(candidate).await;
        self.scanned(outcome)
    }

    /// Steps 11–14 of one scan.
    async fn scan(&mut self, candidate: Candidate) -> ScanOutcome {
        let Candidate {
            id,
            entry,
            user_name,
            template,
        } = candidate;
        let uid = entry.uid;

        // 11. Camera wake. Whether this scan wakes the camera is sampled before
        // `notify_activity` (GitHub #329): a woken sensor needs its auto-exposure to settle.
        let woke = !self.pipeline.camera.is_ready();
        self.pipeline.camera.notify_activity();
        if !wake_camera(
            &self.pipeline.camera,
            Duration::from_millis(MAX_CAMERA_WAKE_WAIT_MS),
        )
        .await
        {
            return ScanOutcome::CameraUnavailable;
        }

        // 12. The unchanged consensus at background priority.
        let thresholds = *self.pipeline.policy.read().await.thresholds();
        let Ok(start_ns) = (self.clock_fn)() else {
            return ScanOutcome::Aborted(ReasonClass::InternalError);
        };
        // After a wake, captures stamped before `start + PRESENCE_WAKE_SETTLE_MS` are never
        // evaluated; the decision budget starts at the end of the settle, so the settle never
        // eats it. A scan of a streaming camera keeps the plain window (bound 0).
        let settle_ms = if woke { PRESENCE_WAKE_SETTLE_MS } else { 0 };
        let not_before_ns = if woke {
            start_ns.saturating_add(settle_ms.saturating_mul(1_000_000))
        } else {
            0
        };
        if woke {
            debug!(
                settle_ms,
                "Camera woken by presence; captures before the settle are not evaluated"
            );
        }
        let deadline = RequestDeadline::compute(
            not_before_ns.max(start_ns),
            0,
            Instant::now(),
            Duration::from_millis(
                DECISION_BUDGET_MS
                    .saturating_add(RESPONSE_WRITE_MARGIN_MS)
                    .saturating_add(settle_ms),
            ),
        )
        .with_not_before(not_before_ns);
        let run = {
            let ctx = ConsensusContext {
                camera: &self.pipeline.camera,
                vision: &self.pipeline.vision,
                inference: &self.inference,
                thresholds,
                clock: self.clock_fn,
                priority: InferencePriority::Background,
                uid,
            };
            run_face_consensus(&ctx, template.embedding.as_slice(), deadline).await
        };
        // The template (zeroized on drop) is not needed past the consensus.
        drop(template);
        let frames_evaluated = match run {
            ConsensusRun::Preempted => return ScanOutcome::Preempted,
            ConsensusRun::Aborted { reason, .. } => return ScanOutcome::Aborted(reason),
            ConsensusRun::Decided {
                decision: ConsensusDecision::SpoofVetoed,
                ..
            } => {
                let key = (id.clone(), entry.epoch);
                if self.last_spoof_warning.as_ref() != Some(&key) {
                    warn!(
                        uid,
                        session_id = id.as_str(),
                        "Presence scan vetoed by presentation attack detection; the session stays locked"
                    );
                    self.last_spoof_warning = Some(key);
                }
                return ScanOutcome::SpoofVetoed;
            }
            ConsensusRun::Decided {
                decision: ConsensusDecision::Pending(_),
                ..
            } => return ScanOutcome::NoMatch,
            ConsensusRun::Decided {
                decision: ConsensusDecision::Allow,
                frames_evaluated,
                ..
            } => frames_evaluated,
        };

        // 13. Fresh, single-use re-check of the candidate after the Allow.
        let Ok(allow_ns) = (self.clock_fn)() else {
            return ScanOutcome::AllowExpired;
        };
        let Ok(allow_boot_ns) = (self.boot_clock_fn)() else {
            return ScanOutcome::AllowExpired;
        };
        let fresh = match bounded(self.logind.session_state(&id)).await {
            Ok(Some(fresh)) => fresh,
            Ok(None) | Err(_) => return ScanOutcome::SessionChanged,
        };
        if fresh.id != id {
            return ScanOutcome::SessionChanged;
        }
        if !fresh.locked {
            return ScanOutcome::SessionChanged;
        }
        if fresh.record.check_local_seat_session_of(uid).is_err() {
            return ScanOutcome::SessionChanged;
        }
        if fresh.user_name.as_ref() != Some(&user_name) {
            return ScanOutcome::SessionChanged;
        }
        if let AccountState::Refused(refusal) = self.check_account(&user_name, uid).await {
            info!(
                uid,
                refusal = refusal.as_str(),
                "Presence unlock refused by the account guard; the session stays locked"
            );
            return ScanOutcome::AccountRefused(refusal);
        }
        // The lid may have closed during the scan; only a closed lid refuses (an error or
        // a timeout does not, same rule as the gate).
        if bounded(self.logind.lid_closed()).await == Ok(true) {
            return ScanOutcome::LidClosed;
        }
        if self.switch.is_engaged() {
            self.tracker.clear();
            return ScanOutcome::KillSwitchEngaged;
        }
        if self.stop_requested() {
            return ScanOutcome::ShuttingDown;
        }
        let Ok(unlock_ns) = (self.clock_fn)() else {
            return ScanOutcome::AllowExpired;
        };
        let Ok(unlock_boot_ns) = (self.boot_clock_fn)() else {
            return ScanOutcome::AllowExpired;
        };
        // The window holds on both clocks: monotonic, and boot time (suspend included).
        let window = ms_to_ns(MAX_ALLOW_TO_UNLOCK_MS);
        for elapsed in [
            unlock_ns.checked_sub(allow_ns),
            unlock_boot_ns.checked_sub(allow_boot_ns),
        ] {
            match elapsed {
                Some(elapsed) if elapsed <= window => {}
                Some(_) | None => return ScanOutcome::AllowExpired,
            }
        }

        // 14. The single UnlockSession call (no retry).
        match bounded(self.logind.unlock_session(&id)).await {
            Ok(()) => {
                self.tracker.mark_unlock_requested(&id, unlock_ns);
                info!(
                    session_id = id.as_str(),
                    uid,
                    captures_evaluated = frames_evaluated,
                    "Presence verified the session owner; locked session unlocked through logind"
                );
                ScanOutcome::Unlocked { session: id, uid }
            }
            Err(err) => {
                warn!(
                    session_id = id.as_str(),
                    uid,
                    error = %err,
                    "UnlockSession failed; the session stays locked"
                );
                self.start_backoff();
                ScanOutcome::UnlockFailed
            }
        }
    }
}
