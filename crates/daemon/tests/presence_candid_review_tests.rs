//! Round 4 contract tests of GitHub #323 (candid review `CHANGES_REQUESTED`, 2026-10-02):
//!
//! 1. [MAJOR] libpam strips `[ ]` around a module argument (`pam.conf(5)`, `_pam_mkargv`):
//!    `pam_faillock.so authfail [deny=2]` and `[dir=/x y]` set faillock policy and must make
//!    the account undeterminable (PAU25), while a bracketed control field before the module
//!    token is not an argument.
//! 2. [MINOR] any shadow password field starting with `*` or `!` is locked (`*LK*`, `*NP*`).
//! 3. [MINOR] the `Allow`-to-unlock window also counts system suspend (CLOCK_BOOTTIME,
//!    injectable through `PresenceWorker::with_boot_clock_fn`), and a lid closed during the
//!    step-13 re-check refuses the unlock (`ScanOutcome::LidClosed`).
//!
//! Kept in a separate file so the pre-existing contract files keep compiling against the
//! current implementation while the new API is missing.

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

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use common::{
    build_worker, fast_presence_config, locked_session, sid, PipelineOptions, ScriptedAccountGuard,
    WorkerFixture, MS_NS,
};
use soos_daemon::pipeline::current_monotonic_nanos_from_clock;
use soos_daemon::presence::account::{
    find_shadow_entry, scan_pam_faillock_options, shadow_refusal, AccountGuard, AccountRefusal,
    AccountState, SystemAccountGuard, UserName,
};
use soos_daemon::presence::worker::{PresenceTick, ScanOutcome, SkipReason};
use soos_daemon::presence::MAX_ALLOW_TO_UNLOCK_MS;
use soos_daemon::DaemonError;

const UID: u32 = 1000;

// ---------------------------------------------------------------------------------------
// 1. Bracketed module arguments (MAJOR)
// ---------------------------------------------------------------------------------------

/// libpam strips the brackets of a bracketed argument: the option is still set.
#[test]
fn test_pau_bracketed_faillock_arguments_are_detected() {
    for line in [
        "auth [default=die] pam_faillock.so authfail [deny=2]\n",
        "auth required pam_faillock.so preauth [dir=/x y]\n",
        "auth required pam_faillock.so preauth [dir=/var/lib/faillock]\n",
        "auth required pam_faillock.so authfail [fail_interval=60]\n",
        "auth required pam_faillock.so authfail [unlock_time=10]\n",
        "auth required pam_faillock.so authfail [root_unlock_time=10]\n",
        "auth required pam_faillock.so authfail [admin_group=wheel]\n",
        "auth required pam_faillock.so authfail [conf=/etc/other conf]\n",
        "auth required pam_faillock.so authfail [even_deny_root]\n",
        "auth required pam_faillock.so preauth \\\n    [deny=1]\n",
    ] {
        assert!(
            scan_pam_faillock_options(line),
            "`{line}` sets faillock policy through a bracketed argument"
        );
    }
}

/// A bracketed control field before the module token is not a module argument.
#[test]
fn test_pau_bracketed_control_field_is_not_an_argument() {
    for line in [
        "auth [success=1 default=bad] pam_faillock.so authfail\n",
        "auth [default=die] pam_faillock.so authfail\n",
        "auth [success=1 new_authtok_reqd=done default=ignore] pam_faillock.so preauth silent\n",
        "auth [deny=1] pam_unix.so\n",
        "auth required pam_unix.so [deny=1]\n",
    ] {
        assert!(
            !scan_pam_faillock_options(line),
            "`{line}` must not be flagged"
        );
    }
}

fn guard_tree() -> (tempfile::TempDir, SystemAccountGuard, PathBuf) {
    fn now() -> Result<u64, DaemonError> {
        Ok(20_000 * 86_400)
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    let tally = root.join("faillock");
    fs::create_dir_all(&tally).unwrap();
    fs::set_permissions(&tally, fs::Permissions::from_mode(0o755)).unwrap();
    let pam = root.join("pam.d");
    fs::create_dir_all(&pam).unwrap();
    fs::write(
        root.join("faillock.conf"),
        format!("dir = {}\n", tally.display()),
    )
    .unwrap();
    fs::write(root.join("shadow"), "alice:$6$h:19900:0:99999:7:::\n").unwrap();
    let guard = SystemAccountGuard::new()
        .with_faillock_conf(root.join("faillock.conf"))
        .with_vendor_faillock_conf(root.join("vendor.conf"))
        .with_default_faillock_dir(tally)
        .with_pam_dirs(vec![pam.clone()])
        .with_shadow(root.join("shadow"))
        .with_realtime_fn(now);
    (temp, guard, pam)
}

/// Through the guard: a bracketed `deny=` / `dir=` in a PAM stack file is `Undeterminable`;
/// a bracketed control field alone keeps the account usable.
#[test]
fn test_pau_guard_detects_bracketed_faillock_arguments() {
    let alice = UserName::parse("alice").unwrap();
    for (content, expected) in [
        (
            "auth [default=die] pam_faillock.so authfail [deny=2]\n",
            AccountState::Refused(AccountRefusal::Undeterminable),
        ),
        (
            "auth required pam_faillock.so preauth [dir=/x y]\n",
            AccountState::Refused(AccountRefusal::Undeterminable),
        ),
        (
            "auth [success=1 default=bad] pam_faillock.so authfail\n",
            AccountState::Usable,
        ),
    ] {
        let (_temp, guard, pam) = guard_tree();
        assert_eq!(guard.check(&alice, UID), AccountState::Usable, "baseline");
        fs::write(pam.join("system-auth"), content).unwrap();
        assert_eq!(guard.check(&alice, UID), expected, "`{content}`");
    }
}

// ---------------------------------------------------------------------------------------
// 2. Shadow password markers (MINOR)
// ---------------------------------------------------------------------------------------

/// Any password field starting with `*` or `!` is a locked / no-login marker.
#[test]
fn test_pau_shadow_star_and_bang_markers_are_locked() {
    let alice = UserName::parse("alice").unwrap();
    for password in [
        "*LK*", "*NP*", "*", "**", "*$6$hash", "!", "!$6$hash", "!*LK*",
    ] {
        let line = format!("alice:{password}:19900:0:99999:7:::\n");
        let entry = find_shadow_entry(line.as_bytes(), &alice)
            .unwrap_or_else(|| panic!("`{password}` line must parse"));
        assert_eq!(
            shadow_refusal(&entry, 20_000 * 86_400),
            Some(AccountRefusal::PasswordLocked),
            "`{password}` is a locked marker"
        );
    }
    let (temp, guard, _pam) = guard_tree();
    fs::write(
        temp.path().join("shadow"),
        "alice:*LK*:19900:0:99999:7:::\n",
    )
    .unwrap();
    assert_eq!(
        guard.check(&alice, UID),
        AccountState::Refused(AccountRefusal::PasswordLocked)
    );
}

// ---------------------------------------------------------------------------------------
// 3. Suspend between Allow and unlock; lid closed at the re-check (MINOR)
// ---------------------------------------------------------------------------------------

fn advance(offset: &AtomicU64, ms: u64) {
    offset.fetch_add(ms * MS_NS, Ordering::SeqCst);
}

/// Replaces the worker's boot clock (CLOCK_BOOTTIME in production).
fn with_boot<A: AccountGuard>(
    fx: WorkerFixture<A>,
    boot: fn() -> Result<u64, DaemonError>,
) -> WorkerFixture<A> {
    WorkerFixture {
        worker: fx.worker.with_boot_clock_fn(boot),
        ..fx
    }
}

async fn eligible(clock: fn() -> Result<u64, DaemonError>) -> WorkerFixture<ScriptedAccountGuard> {
    let fx = build_worker(
        PipelineOptions::new(clock),
        fast_presence_config(),
        ScriptedAccountGuard::always(AccountState::Usable),
    )
    .await;
    fx.logind
        .set_sessions(vec![locked_session("2", UID, "alice")]);
    fx
}

async fn past_grace<A: AccountGuard>(
    fx: &mut WorkerFixture<A>,
    offset: &AtomicU64,
) -> PresenceTick {
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::InGrace)
    );
    advance(offset, 1000);
    fx.worker.tick().await
}

static BOOT_OFFSET: AtomicU64 = AtomicU64::new(0);

fn boot_clock() -> Result<u64, DaemonError> {
    current_monotonic_nanos_from_clock(nix::time::ClockId::CLOCK_BOOTTIME)
        .map(|now| now.saturating_add(BOOT_OFFSET.load(Ordering::SeqCst)))
}

/// A suspend between the `Allow` and the unlock (boot clock jumps, monotonic clock does
/// not) discards the `Allow`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_suspend_between_allow_and_unlock_discards_the_allow() {
    test_clock!(OFFSET, clock);
    let mut fx = with_boot(eligible(clock).await, boot_clock);
    fx.logind.on_session_state(|| {
        BOOT_OFFSET.fetch_add((MAX_ALLOW_TO_UNLOCK_MS + 5000) * MS_NS, Ordering::SeqCst);
    });
    assert_eq!(
        past_grace(&mut fx, &OFFSET).await,
        PresenceTick::Scanned(ScanOutcome::AllowExpired),
        "suspend time counts against MAX_ALLOW_TO_UNLOCK_MS"
    );
    assert_eq!(fx.logind.unlock_calls(), 0);
}

static STEADY_BOOT_OFFSET: AtomicU64 = AtomicU64::new(0);

fn steady_boot_clock() -> Result<u64, DaemonError> {
    current_monotonic_nanos_from_clock(nix::time::ClockId::CLOCK_BOOTTIME)
        .map(|now| now.saturating_add(STEADY_BOOT_OFFSET.load(Ordering::SeqCst)))
}

/// Without a suspend the injected boot clock does not prevent the unlock.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_boot_clock_without_suspend_still_unlocks() {
    test_clock!(OFFSET, clock);
    let mut fx = with_boot(eligible(clock).await, steady_boot_clock);
    assert_eq!(
        past_grace(&mut fx, &OFFSET).await,
        PresenceTick::Scanned(ScanOutcome::Unlocked {
            session: sid("2"),
            uid: UID
        })
    );
    assert_eq!(fx.logind.unlock_calls(), 1);
}

static BOOT_FAIL: AtomicBool = AtomicBool::new(false);

fn failing_after_allow_boot_clock() -> Result<u64, DaemonError> {
    if BOOT_FAIL.load(Ordering::SeqCst) {
        return Err(DaemonError::Clock("injected boot clock failure".into()));
    }
    current_monotonic_nanos_from_clock(nix::time::ClockId::CLOCK_BOOTTIME)
}

/// A boot-clock failure after the `Allow` never unlocks (`AllowExpired`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_boot_clock_failure_after_allow_never_unlocks() {
    test_clock!(OFFSET, clock);
    let mut fx = with_boot(eligible(clock).await, failing_after_allow_boot_clock);
    fx.logind
        .on_session_state(|| BOOT_FAIL.store(true, Ordering::SeqCst));
    assert_eq!(
        past_grace(&mut fx, &OFFSET).await,
        PresenceTick::Scanned(ScanOutcome::AllowExpired)
    );
    assert_eq!(fx.logind.unlock_calls(), 0);
}

/// The lid closing after the gate (during the scan, seen by the step-13 re-check) refuses the unlock (`LidClosed`); the lid is
/// read again after the `Allow`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_lid_closed_at_recheck_never_unlocks() {
    test_clock!(OFFSET, clock);
    let mut fx = eligible(clock).await;
    {
        let logind = fx.logind.clone();
        // Closed during the scan, i.e. after the gate and before the re-check.
        fx.parts
            .set_extractor_hook(move || logind.set_lid(Ok(true)));
    }
    assert_eq!(
        past_grace(&mut fx, &OFFSET).await,
        PresenceTick::Scanned(ScanOutcome::LidClosed)
    );
    assert_eq!(fx.logind.unlock_calls(), 0);
    assert!(
        fx.logind.lid_calls() >= 2,
        "the lid is read at the gate and again after the Allow"
    );
}

/// A lid read error at the re-check does not refuse (only a closed lid does, same rule as
/// the gate), so the unlock still happens.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pau_lid_error_at_recheck_does_not_refuse() {
    test_clock!(OFFSET, clock);
    let mut fx = eligible(clock).await;
    {
        let logind = fx.logind.clone();
        fx.parts.set_extractor_hook(move || {
            logind.set_lid(Err(
                soos_daemon::presence::logind::PresenceLogindError::Timeout,
            ));
        });
    }
    assert_eq!(
        past_grace(&mut fx, &OFFSET).await,
        PresenceTick::Scanned(ScanOutcome::Unlocked {
            session: sid("2"),
            uid: UID
        })
    );
    assert_eq!(fx.logind.unlock_calls(), 1);
}
