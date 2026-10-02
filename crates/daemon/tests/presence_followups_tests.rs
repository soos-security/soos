//! Contract tests of GitHub #325 (review follow-ups of the presence auto-unlock, #324) for
//! the worker tick and the PAM line scan: matrix PFU1 (fresh attempt stamp), PFU4 (documented
//! over-detection of the PAM line scan), PFU5 (at most one account check in flight) and PFU6
//! (no logind traffic while the store holds no template).
//!
//! Power rule (as in `presence_worker_tests`): "no attempt" is asserted through the shared
//! rate limiter, "no camera" through the `notify_activity` counter, "no D-Bus" through the
//! mock logind call counters, and `unlock_calls == 0` after every failure path.

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

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use common::{
    build_worker, fast_presence_config, locked_session, sid, Enrollment, PipelineOptions,
    WorkerFixture, MS_NS, SECOND_NS,
};
use soos_daemon::pipeline::current_monotonic_nanos;
use soos_daemon::presence::account::{
    scan_pam_faillock_options, AccountGuard, AccountState, UserName,
};
use soos_daemon::presence::worker::{PresenceTick, ScanOutcome, SkipReason};
use soos_daemon::presence::ACCOUNT_CHECK_TIMEOUT_MS;
use soos_daemon::DaemonError;

const UID: u32 = 1000;

fn advance(offset: &AtomicU64, ms: u64) {
    offset.fetch_add(ms * MS_NS, Ordering::SeqCst);
}

fn unlocked(id: &str, uid: u32) -> PresenceTick {
    PresenceTick::Scanned(ScanOutcome::Unlocked {
        session: sid(id),
        uid,
    })
}

/// No unlock, no camera wake, no inference, no attempt.
fn assert_nothing_spent<A: AccountGuard>(fx: &WorkerFixture<A>, label: &str) {
    assert_eq!(fx.logind.unlock_calls(), 0, "{label}: no unlock call");
    assert_eq!(fx.parts.camera.wakes(), 0, "{label}: no camera wake");
    assert_eq!(fx.parts.inferences(), 0, "{label}: no inference");
    assert_eq!(fx.parts.tracked_uids(), 0, "{label}: no attempt recorded");
}

// ---------------------------------------------------------------------------------------
// Account guard doubles
// ---------------------------------------------------------------------------------------

type Hook = Box<dyn Fn() + Send + Sync>;

/// Usable guard that runs `hook` on its first call only (step 7 of the first candidate tick,
/// i.e. between the step-3 clock read and the step-10 attempt).
struct FirstCallHookGuard {
    fired: AtomicBool,
    hook: Hook,
}

impl FirstCallHookGuard {
    fn new(hook: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            fired: AtomicBool::new(false),
            hook: Box::new(hook),
        }
    }
}

impl AccountGuard for FirstCallHookGuard {
    fn check(&self, _user: &UserName, _uid: u32) -> AccountState {
        if !self.fired.swap(true, Ordering::SeqCst) {
            (self.hook)();
        }
        AccountState::Usable
    }
}

/// A guard that blocks every call until the gate opens and records concurrent entries.
#[derive(Default)]
struct GateState {
    open: Mutex<bool>,
    opened: Condvar,
    entered: AtomicUsize,
    active: AtomicUsize,
    max_active: AtomicUsize,
    exited: AtomicUsize,
}

#[derive(Clone, Default)]
struct GatedGuard(Arc<GateState>);

impl GatedGuard {
    fn release(&self) {
        *self.0.open.lock().unwrap() = true;
        self.0.opened.notify_all();
    }
    fn entered(&self) -> usize {
        self.0.entered.load(Ordering::SeqCst)
    }
    fn exited(&self) -> usize {
        self.0.exited.load(Ordering::SeqCst)
    }
    fn max_active(&self) -> usize {
        self.0.max_active.load(Ordering::SeqCst)
    }
}

impl AccountGuard for GatedGuard {
    fn check(&self, _user: &UserName, _uid: u32) -> AccountState {
        self.0.entered.fetch_add(1, Ordering::SeqCst);
        let active = self.0.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.0.max_active.fetch_max(active, Ordering::SeqCst);
        let mut open = self.0.open.lock().unwrap();
        while !*open {
            open = self.0.opened.wait(open).unwrap();
        }
        drop(open);
        self.0.active.fetch_sub(1, Ordering::SeqCst);
        self.0.exited.fetch_add(1, Ordering::SeqCst);
        AccountState::Usable
    }
}

/// Opens the gate when dropped, so a failing assertion never leaves a blocking-pool thread
/// parked (the runtime would wait for it on drop).
struct ReleaseOnDrop(GatedGuard);

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

// ---------------------------------------------------------------------------------------
// PFU1 — the attempt is stamped with a fresh clock read
// ---------------------------------------------------------------------------------------

static STAMP_OFFSET: AtomicU64 = AtomicU64::new(0);

fn stamp_clock() -> Result<u64, DaemonError> {
    current_monotonic_nanos().map(|now| now + STAMP_OFFSET.load(Ordering::SeqCst))
}

/// PFU1: the attempt recorded for a scan carries a clock read taken after step 7, not the
/// step-3 timestamp. The guard shifts the test clock by 1 s at step 7; probing the limiter
/// 60.5 s after the pre-tick reading still counts the attempt only if it was stamped with the
/// fresh (shifted) value.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pfu_attempt_is_stamped_with_a_fresh_clock_read() {
    let guard = FirstCallHookGuard::new(|| {
        STAMP_OFFSET.fetch_add(SECOND_NS, Ordering::SeqCst);
    });
    let mut fx = build_worker(
        PipelineOptions::new(stamp_clock),
        fast_presence_config(),
        guard,
    )
    .await;
    fx.logind
        .set_sessions(vec![locked_session("2", UID, "alice")]);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::InGrace)
    );
    advance(&STAMP_OFFSET, 1000);
    let before = stamp_clock().unwrap();
    assert_eq!(fx.worker.tick().await, unlocked("2", UID));

    let engine = fx.parts.policy.try_read().expect("policy lock free");
    let limiter = engine.rate_limiter().expect("rate limiter configured");
    let probe = before + 60 * SECOND_NS + 500 * MS_NS;
    assert_eq!(
        limiter.remaining_attempts(UID, probe),
        39,
        "the attempt must be stamped at or after the step-7 clock shift (fresh read right \
         before record_attempt_with_reserve), not with the step-3 timestamp"
    );
}

static WINDOW_OFFSET: AtomicU64 = AtomicU64::new(0);

fn window_clock() -> Result<u64, DaemonError> {
    current_monotonic_nanos().map(|now| now + WINDOW_OFFSET.load(Ordering::SeqCst))
}

/// PFU1: the reserve is evaluated at the fresh timestamp. With `max_attempts = 6`, an earlier
/// attempt inside the window at step 3 but outside it at step 10 must no longer count.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pfu_reserve_is_evaluated_at_the_fresh_timestamp() {
    let guard = FirstCallHookGuard::new(|| {
        WINDOW_OFFSET.fetch_add(SECOND_NS, Ordering::SeqCst);
    });
    let mut options = PipelineOptions::new(window_clock);
    options.max_attempts = 6;
    let mut fx = build_worker(options, fast_presence_config(), guard).await;
    fx.logind
        .set_sessions(vec![locked_session("2", UID, "alice")]);
    let earlier = window_clock().unwrap();
    fx.parts
        .policy
        .write()
        .await
        .record_attempt_with_reserve(UID, earlier, 0)
        .expect("one earlier attempt");
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::InGrace)
    );
    // Step 3 of the next tick sits 59.5 s after the earlier attempt (inside the 60 s window,
    // reserve reached); the step-7 shift moves the fresh read past the window.
    advance(&WINDOW_OFFSET, 59_500);
    assert_eq!(
        fx.worker.tick().await,
        unlocked("2", UID),
        "the earlier attempt left the window at the fresh timestamp: presence may scan"
    );
}

static FAIL_OFFSET: AtomicU64 = AtomicU64::new(0);
static FAIL_NOW: AtomicBool = AtomicBool::new(false);

fn failing_after_step_7_clock() -> Result<u64, DaemonError> {
    if FAIL_NOW.load(Ordering::SeqCst) {
        return Err(DaemonError::Clock("injected clock failure".into()));
    }
    current_monotonic_nanos().map(|now| now + FAIL_OFFSET.load(Ordering::SeqCst))
}

/// PFU1: a clock failure at the fresh read skips the tick with `ClockUnavailable`: no attempt,
/// no camera wake, no inference, no unlock.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pfu_clock_failure_before_the_attempt_skips_without_attempt_or_scan() {
    let guard = FirstCallHookGuard::new(|| FAIL_NOW.store(true, Ordering::SeqCst));
    let mut fx = build_worker(
        PipelineOptions::new(failing_after_step_7_clock),
        fast_presence_config(),
        guard,
    )
    .await;
    fx.logind
        .set_sessions(vec![locked_session("2", UID, "alice")]);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::InGrace)
    );
    advance(&FAIL_OFFSET, 1000);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::ClockUnavailable)
    );
    assert_nothing_spent(&fx, "clock failure before the attempt");
}

static BACK_OFFSET: AtomicU64 = AtomicU64::new(10 * SECOND_NS);

fn regressing_clock() -> Result<u64, DaemonError> {
    current_monotonic_nanos().map(|now| now + BACK_OFFSET.load(Ordering::SeqCst))
}

/// PFU1: a fresh reading older than the step-3 reading (injected clock regression) is a clock
/// failure: `ClockUnavailable`, nothing spent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pfu_clock_regression_before_the_attempt_skips_the_tick() {
    let guard = FirstCallHookGuard::new(|| {
        BACK_OFFSET.fetch_sub(5 * SECOND_NS, Ordering::SeqCst);
    });
    let mut fx = build_worker(
        PipelineOptions::new(regressing_clock),
        fast_presence_config(),
        guard,
    )
    .await;
    fx.logind
        .set_sessions(vec![locked_session("2", UID, "alice")]);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::InGrace)
    );
    advance(&BACK_OFFSET, 1000);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::ClockUnavailable)
    );
    assert_nothing_spent(&fx, "clock regression before the attempt");
}

static LOCK_OFFSET: AtomicU64 = AtomicU64::new(0);

fn lock_clock() -> Result<u64, DaemonError> {
    current_monotonic_nanos().map(|now| now + LOCK_OFFSET.load(Ordering::SeqCst))
}

/// PFU1 (round-1 evaluation F5): the fresh read happens after the policy write lock is
/// acquired. Another task holds the lock across step 10 and shifts the clock by 1 s just before
/// releasing it; the recorded attempt must carry the shifted value.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pfu_attempt_is_stamped_after_the_policy_lock_is_acquired() {
    let reached_step_7 = Arc::new(tokio::sync::Notify::new());
    let signal = Arc::clone(&reached_step_7);
    let guard = FirstCallHookGuard::new(move || signal.notify_one());
    let mut fx = build_worker(
        PipelineOptions::new(lock_clock),
        fast_presence_config(),
        guard,
    )
    .await;
    fx.logind
        .set_sessions(vec![locked_session("2", UID, "alice")]);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::InGrace)
    );
    advance(&LOCK_OFFSET, 1000);
    let before = lock_clock().unwrap();

    let policy = Arc::clone(&fx.parts.policy);
    let (held_tx, held_rx) = tokio::sync::oneshot::channel();
    let holder = tokio::spawn(async move {
        let engine = policy.write().await;
        held_tx.send(()).unwrap();
        reached_step_7.notified().await;
        // Steps 8-9 are mock calls: the tick is parked on the lock well within this delay.
        tokio::time::sleep(Duration::from_millis(300)).await;
        LOCK_OFFSET.fetch_add(SECOND_NS, Ordering::SeqCst);
        drop(engine);
    });
    held_rx.await.unwrap();
    assert_eq!(fx.worker.tick().await, unlocked("2", UID));
    holder.await.unwrap();

    let engine = fx.parts.policy.try_read().expect("policy lock free");
    let limiter = engine.rate_limiter().expect("rate limiter configured");
    let probe = before + 60 * SECOND_NS + 500 * MS_NS;
    assert_eq!(
        limiter.remaining_attempts(UID, probe),
        39,
        "the stamp must be read once the policy lock is held (after the 1 s shift made by \
         the lock holder), not before waiting for the lock"
    );
}

// ---------------------------------------------------------------------------------------
// PFU5 — at most one account check in flight
// ---------------------------------------------------------------------------------------

/// PFU5 (auditor constraint 9 / T11 of #323): while one account check blocks past
/// `ACCOUNT_CHECK_TIMEOUT_MS`, the next tick refuses the candidate at once without entering
/// the guard again (never two concurrent checks); once the first check has ended, the guard
/// is consulted again and the session can be unlocked.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pfu_second_account_check_never_runs_while_one_is_in_flight() {
    test_clock!(OFFSET, clock);
    let guard = GatedGuard::default();
    let _release = ReleaseOnDrop(guard.clone());
    let mut fx = build_worker(
        PipelineOptions::new(clock),
        fast_presence_config(),
        guard.clone(),
    )
    .await;
    fx.logind
        .set_sessions(vec![locked_session("2", UID, "alice")]);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::InGrace)
    );

    // Tick A: the check blocks and times out; the blocking job stays in flight.
    advance(&OFFSET, 1000);
    let tick_a = fx.worker.tick().await;
    assert!(
        tick_a == PresenceTick::Skipped(SkipReason::AccountRefused)
            || tick_a == PresenceTick::Skipped(SkipReason::NoCandidate),
        "a hung check refuses the candidate, got {tick_a:?}"
    );
    assert_eq!(guard.entered(), 1, "tick A entered the guard once");

    // Tick B while the first check is still running: refused at once, no second entry.
    advance(&OFFSET, 2000);
    let started = Instant::now();
    let tick_b = fx.worker.tick().await;
    let elapsed = started.elapsed();
    assert!(
        tick_b == PresenceTick::Skipped(SkipReason::AccountRefused)
            || tick_b == PresenceTick::Skipped(SkipReason::NoCandidate),
        "a check in flight makes the next one undeterminable, got {tick_b:?}"
    );
    assert_eq!(
        guard.entered(),
        1,
        "no second account check may start while one is in flight"
    );
    assert_eq!(guard.max_active(), 1, "never two concurrent account checks");
    assert!(
        elapsed < Duration::from_millis(ACCOUNT_CHECK_TIMEOUT_MS),
        "the in-flight refusal is immediate, not another {ACCOUNT_CHECK_TIMEOUT_MS} ms wait \
         ({elapsed:?})"
    );
    assert_nothing_spent(&fx, "account check in flight");

    // The first check ends: the guard is consulted again (the flag is released when the
    // blocking job finishes, which may lag the guard's return by a few instructions).
    guard.release();
    let deadline = Instant::now() + Duration::from_secs(10);
    while guard.exited() < 1 {
        assert!(Instant::now() < deadline, "the released check must end");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let mut tick_c = PresenceTick::Skipped(SkipReason::AccountRefused);
    for _ in 0..200 {
        advance(&OFFSET, 2000);
        tick_c = fx.worker.tick().await;
        if tick_c != PresenceTick::Skipped(SkipReason::AccountRefused) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(tick_c, unlocked("2", UID));
    assert!(
        guard.entered() >= 2,
        "the guard is entered again once the first check has ended"
    );
    assert_eq!(guard.max_active(), 1, "never two concurrent account checks");
}

// ---------------------------------------------------------------------------------------
// PFU6 — no logind traffic while the store holds no template
// ---------------------------------------------------------------------------------------

/// PFU6: with an empty store, 10 ticks over a locked session make no logind call at all
/// (no snapshot, no lid read, no re-check, no unlock) and skip with `NotEnrolled`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pfu_no_logind_traffic_while_nobody_is_enrolled() {
    test_clock!(OFFSET, clock);
    let mut options = PipelineOptions::new(clock);
    options.enrolled = vec![];
    let mut fx = build_worker(
        options,
        fast_presence_config(),
        common::ScriptedAccountGuard::always(AccountState::Usable),
    )
    .await;
    fx.logind
        .set_sessions(vec![locked_session("2", UID, "alice")]);
    for _ in 0..10 {
        assert_eq!(
            fx.worker.tick().await,
            PresenceTick::Skipped(SkipReason::NotEnrolled)
        );
        advance(&OFFSET, 1000);
    }
    assert_eq!(
        fx.logind.seat_calls(),
        0,
        "no snapshot without any template"
    );
    assert_eq!(fx.logind.lid_calls(), 0);
    assert_eq!(fx.logind.state_calls(), 0);
    assert_nothing_spent(&fx, "nobody enrolled");
}

/// PFU6: the probe runs every tick: a template enrolled while the daemon runs is picked up on
/// the next tick (logind polled again, the lock observed with a fresh grace), no restart.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pfu_new_enrollment_is_picked_up_on_the_next_tick() {
    test_clock!(OFFSET, clock);
    let mut options = PipelineOptions::new(clock);
    options.enrolled = vec![];
    let mut fx = build_worker(
        options,
        fast_presence_config(),
        common::ScriptedAccountGuard::always(AccountState::Usable),
    )
    .await;
    fx.logind
        .set_sessions(vec![locked_session("2", UID, "alice")]);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::NotEnrolled)
    );
    assert_eq!(fx.logind.seat_calls(), 0);
    fx.parts.enroll(UID, Enrollment::LiveIdentity);
    advance(&OFFSET, 1000);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::InGrace),
        "the next tick polls logind and starts the grace"
    );
    assert_eq!(fx.logind.seat_calls(), 1);
    advance(&OFFSET, 1000);
    assert_eq!(fx.worker.tick().await, unlocked("2", UID));
}

/// PFU6: removing the last template stops the polling and clears the lock tracker, so after a
/// re-enrollment the lock is observed with a fresh grace (never scanned on the old one).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pfu_removed_enrollment_stops_polling_and_restarts_the_grace() {
    test_clock!(OFFSET, clock);
    let mut fx = build_worker(
        PipelineOptions::new(clock),
        fast_presence_config(),
        common::ScriptedAccountGuard::always(AccountState::Usable),
    )
    .await;
    fx.logind
        .set_sessions(vec![locked_session("2", UID, "alice")]);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::InGrace)
    );
    assert_eq!(fx.logind.seat_calls(), 1);
    assert!(fx.parts.store.delete(UID).unwrap(), "template removed");
    advance(&OFFSET, 1000);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::NotEnrolled)
    );
    assert_eq!(
        fx.logind.seat_calls(),
        1,
        "no snapshot once the store is empty"
    );
    fx.parts.enroll(UID, Enrollment::LiveIdentity);
    advance(&OFFSET, 1000);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::InGrace),
        "the tracker was cleared: the grace restarts instead of scanning on the old lock"
    );
    assert_nothing_spent(&fx, "grace restarted after re-enrollment");
    advance(&OFFSET, 1000);
    assert_eq!(fx.worker.tick().await, unlocked("2", UID));
}

/// PFU6: a store that cannot be listed skips with `TemplateStoreError` before any logind call.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pfu_store_listing_error_skips_before_logind() {
    test_clock!(OFFSET, clock);
    let mut fx = build_worker(
        PipelineOptions::new(clock),
        fast_presence_config(),
        common::ScriptedAccountGuard::always(AccountState::Usable),
    )
    .await;
    fx.logind
        .set_sessions(vec![locked_session("2", UID, "alice")]);
    std::fs::remove_dir_all(fx.parts.temp.path().join("biometrics")).unwrap();
    for _ in 0..3 {
        assert_eq!(
            fx.worker.tick().await,
            PresenceTick::Skipped(SkipReason::TemplateStoreError)
        );
        advance(&OFFSET, 1000);
    }
    assert_eq!(fx.logind.seat_calls(), 0, "no snapshot on a store error");
    assert_nothing_spent(&fx, "store listing error");
}

// ---------------------------------------------------------------------------------------
// PFU4 — the PAM line scan deliberately over-detects (characterisation)
// ---------------------------------------------------------------------------------------

/// PFU4: an option glued to the module by Unicode whitespace (no-break, em, ideographic
/// space; libpam splits only at ASCII space, tab and newline) is still detected.
#[test]
fn test_pfu_pam_scan_detects_options_across_unicode_whitespace() {
    for separator in ['\u{00A0}', '\u{2003}', '\u{3000}'] {
        let line = format!("auth required pam_faillock.so{separator}deny=5\n");
        assert!(
            scan_pam_faillock_options(&line),
            "U+{:04X}: an option after the module name is detected (deliberate over-detection)",
            u32::from(separator)
        );
    }
}

/// PFU4: the `#` comment is dropped before the continuation check, so a `\` right before a
/// `#` joins the next physical line (libpam ends a physical line that holds a `#`).
#[test]
fn test_pfu_pam_scan_backslash_before_comment_continues_the_line() {
    let stack = "auth required pam_faillock.so preauth \\# trailing note\n    deny=4\n";
    assert!(
        scan_pam_faillock_options(stack),
        "a backslash before a comment continues the line (deliberate over-detection)"
    );
}

/// PFU4 (round-1 evaluation F1): the scan is a superset of libpam. libpam splits only at ASCII
/// blanks, so it passes `deny=1` here although a Unicode split opens an unterminated `[` token.
#[test]
fn test_pfu_pam_scan_unterminated_bracket_after_unicode_space_is_detected() {
    for line in [
        "auth required pam_faillock.so x\u{a0}[a deny=1\n",
        "auth required pam_faillock.so [x deny=1\n",
        "auth required pam_faillock.so preauth [unterminated \\\n    even_deny_root\n",
    ] {
        assert!(
            scan_pam_faillock_options(line),
            "{line:?}: an option after the module name is detected whatever the brackets"
        );
    }
}

/// PFU4 (round-1 evaluation F1): libpam ends a physical line that holds a `#`, so the
/// second line is its own PAM line with `deny=1`, even when the guard joins it to a first line
/// whose comment hid a trailing `\` and an unterminated bracket.
#[test]
fn test_pfu_pam_scan_line_after_a_commented_continuation_is_detected() {
    for stack in [
        "auth [default=1 \\ # note\nauth required pam_faillock.so deny=1\n",
        "auth [default=1 \\# note\nauth required pam_faillock.so unlock_time=5\n",
        "auth required pam_unix.so [x \\ # note\nauth required pam_faillock.so dir=/x\n",
    ] {
        assert!(
            scan_pam_faillock_options(stack),
            "{stack:?}: libpam applies the option of the second line"
        );
    }
}

/// PFU4 (round-2 evaluation R2-F1): libpam follows an `include` / `substack` target that is
/// absolute, nested or relative; the guard does not scan such files, so the line is refused.
#[test]
fn test_pfu_pam_scan_include_of_a_path_is_detected() {
    for line in [
        "auth include /etc/security/site-auth\n",
        "auth substack sub/file\n",
        "auth include ../x\n",
        "-account INCLUDE /opt/pam/account\n",
        "session Substack ./local\n",
        "@include /etc/pam.d/sub/common-auth\n",
        "@INCLUDE /etc/site\n",
        "@Include ../common\n",
        "login auth include /etc/site\n",
        "auth\tinclude\u{a0}/etc/site\n",
    ] {
        assert!(
            scan_pam_faillock_options(line),
            "{line:?}: libpam reads a stack file the guard never scans"
        );
    }
}

/// PFU4 (auditor A14): the path target need not be the token right after the directive.
#[test]
fn test_pfu_pam_scan_include_path_after_other_tokens_is_detected() {
    for line in ["auth include x\u{a0}/y\n", "auth substack a\u{2003}../b\n"] {
        assert!(
            scan_pam_faillock_options(line),
            "{line:?}: a later token holding `/` follows the directive"
        );
    }
}

/// PFU4 (auditor A13): options are searched after the FIRST `pam_faillock.so`, so a later
/// occurrence of the name never hides an option placed before it.
#[test]
fn test_pfu_pam_scan_searches_after_the_first_module_occurrence() {
    let line = "auth required pam_faillock.so deny=1 note=pam_faillock.so\n";
    assert!(
        scan_pam_faillock_options(line),
        "{line:?}: `deny=1` follows the first pam_faillock.so"
    );
}

/// PFU4: plain-name includes resolve inside the scanned PAM directories; paths elsewhere on
/// a line are not includes.
#[test]
fn test_pfu_pam_scan_plain_name_include_stays_usable() {
    for content in [
        "auth include system-auth\n",
        "auth substack password-auth\n",
        "@include common-auth\n",
        "-session include postlogin\n",
        "auth required pam_env.so envfile=/etc/environment\n",
        "auth required /usr/lib/security/pam_unix.so\n",
        "# auth include /etc/site\n",
    ] {
        assert!(!scan_pam_faillock_options(content), "{content:?}");
    }
}

/// PFU4: the superset rule keeps the documented negatives undetected.
#[test]
fn test_pfu_pam_scan_superset_keeps_negatives_undetected() {
    for content in [
        "auth [deny=1] pam_faillock.so preauth\n",
        "auth [success=1 default=bad] pam_faillock.so authfail\n",
        "auth required pam_unix.so deny=1\n",
        "auth required pam_faillock.so preauth # deny=1\n",
        "# auth required pam_faillock.so deny=1\n",
        "auth required pam_faillock.so preauth silent audit no_log_info local_users_only nodelay\n",
    ] {
        assert!(!scan_pam_faillock_options(content), "{content:?}");
    }
}

/// PFU4: the module token is matched by suffix, so a path or a prefixed name is detected.
#[test]
fn test_pfu_pam_scan_matches_the_module_by_suffix() {
    for line in [
        "auth required /usr/lib/security/pam_faillock.so deny=3\n",
        "auth required x-pam_faillock.so unlock_time=1\n",
    ] {
        assert!(scan_pam_faillock_options(line), "{line:?}");
    }
    assert!(
        !scan_pam_faillock_options("auth required pam_faillock.so preauth silent\n"),
        "no policy option, nothing detected"
    );
}
