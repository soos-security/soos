//! Contract tests of GitHub #323 for presence logging (matrix PAU16, PAU19 "no system bus",
//! F6 gate transitions).
//!
//! - an unlock logs exactly one `info` line carrying the session ID and the UID;
//! - no frame, embedding, template or score field is logged above `debug`;
//! - an unchanged skip reason is logged once (transition-only, no 1 Hz spam);
//! - logind unavailability is warned once per transition (no system bus ⇒ one warn and
//!   backoff);
//! - lid/screen gate transitions are logged at `info` (value-free), once per transition.
//!
//! The subscriber is thread-local, so these tests run on a `current_thread` runtime.

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

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use common::{
    build_worker, capture_logs, fast_presence_config, locked_session, sid, LogBuffer,
    PipelineOptions, ScriptedAccountGuard, MS_NS,
};
use soos_daemon::presence::account::AccountState;
use soos_daemon::presence::logind::PresenceLogindError;
use soos_daemon::presence::worker::{PresenceTick, ScanOutcome, SkipReason};
use soos_inference_ort::{AttackType, PadResult};

fn advance(offset: &AtomicU64, ms: u64) {
    offset.fetch_add(ms * MS_NS, Ordering::SeqCst);
}

fn lines_at_or_above_info(logs: &LogBuffer) -> Vec<String> {
    logs.contents()
        .lines()
        .filter(|l| l.contains(" INFO ") || l.contains(" WARN ") || l.contains(" ERROR "))
        .map(str::to_string)
        .collect()
}

fn count(logs: &LogBuffer, level: &str, needle: &str) -> usize {
    logs.contents()
        .lines()
        .filter(|l| l.contains(&format!(" {level} ")) && l.contains(needle))
        .count()
}

/// Field names that would carry biometric values or raw captures.
const FORBIDDEN_FIELDS: [&str; 7] = [
    "score=",
    "similarity=",
    "sim=",
    "embedding=",
    "template=",
    "frame=",
    "pad_score=",
];

/// Stable, value-free skip-reason codes (snake_case of the variant).
#[test]
fn test_pau_skip_reason_codes_are_stable_snake_case() {
    let cases = [
        (SkipReason::KillSwitch, "kill_switch"),
        (SkipReason::LogindUnavailable, "logind_unavailable"),
        (SkipReason::TooManySessions, "too_many_sessions"),
        (SkipReason::TrackerOverflow, "tracker_overflow"),
        (SkipReason::NoLockedSession, "no_locked_session"),
        (SkipReason::InGrace, "in_grace"),
        (SkipReason::NotDue, "not_due"),
        (SkipReason::NotEnrolled, "not_enrolled"),
        (SkipReason::ForeignTemplate, "foreign_template"),
        (SkipReason::TemplateStoreError, "template_store_error"),
        (SkipReason::NoCandidate, "no_candidate"),
        (SkipReason::AmbiguousCandidates, "ambiguous_candidates"),
        (SkipReason::LidClosed, "lid_closed"),
        (SkipReason::DisplayOff, "display_off"),
        (SkipReason::InteractiveDemand, "interactive_demand"),
        (SkipReason::RateLimited, "rate_limited"),
        (SkipReason::ClockUnavailable, "clock_unavailable"),
        (SkipReason::Backoff, "backoff"),
        (SkipReason::AccountRefused, "account_refused"),
    ];
    for (reason, code) in cases {
        assert_eq!(reason.as_str(), code);
    }
}

/// PAU16: a presence unlock logs exactly one `info` line with the session ID and the UID,
/// and nothing above `debug` carries a biometric value.
#[tokio::test]
async fn test_pau_unlock_logs_one_info_line_without_biometric_values() {
    test_clock!(OFFSET, clock);
    let mut fx = build_worker(
        PipelineOptions::new(clock),
        fast_presence_config(),
        ScriptedAccountGuard::always(AccountState::Usable),
    )
    .await;
    let (logs, _guard) = capture_logs();
    fx.logind
        .set_sessions(vec![locked_session("2", 1000, "alice")]);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::InGrace)
    );
    advance(&OFFSET, 1000);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Scanned(ScanOutcome::Unlocked {
            session: sid("2"),
            uid: 1000
        })
    );
    let unlock_lines: Vec<String> = lines_at_or_above_info(&logs)
        .into_iter()
        .filter(|l| l.contains(" INFO ") && l.contains("session_id"))
        .collect();
    assert_eq!(
        unlock_lines.len(),
        1,
        "exactly one info line for the unlock: {unlock_lines:?}"
    );
    assert!(
        unlock_lines[0].contains("uid=1000") || unlock_lines[0].contains("uid = 1000"),
        "the unlock line names the UID: {}",
        unlock_lines[0]
    );
    for line in lines_at_or_above_info(&logs) {
        for field in FORBIDDEN_FIELDS {
            assert!(!line.contains(field), "no `{field}` above debug: {line}");
        }
        assert!(
            !line.contains("alice"),
            "users are identified by uid above debug, never by name: {line}"
        );
    }
}

/// PAU16 / D7: a spoof veto is warned (value-free) and no biometric field appears above
/// `debug`.
#[tokio::test]
async fn test_pau_spoof_veto_is_warned_without_biometric_values() {
    test_clock!(OFFSET, clock);
    let mut fx = build_worker(
        PipelineOptions::new(clock),
        fast_presence_config(),
        ScriptedAccountGuard::always(AccountState::Usable),
    )
    .await;
    let (logs, _guard) = capture_logs();
    fx.parts
        .pad
        .set_result(PadResult::spoof(0.99, AttackType::ScreenReplay));
    fx.logind
        .set_sessions(vec![locked_session("2", 1000, "alice")]);
    let _ = fx.worker.tick().await;
    advance(&OFFSET, 1000);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Scanned(ScanOutcome::SpoofVetoed)
    );
    assert!(
        logs.contents().lines().any(|l| l.contains(" WARN ")),
        "a presence spoof veto is logged at warn"
    );
    assert_eq!(fx.logind.unlock_calls(), 0);
    for line in lines_at_or_above_info(&logs) {
        for field in FORBIDDEN_FIELDS {
            assert!(!line.contains(field), "no `{field}` above debug: {line}");
        }
        assert!(
            !line.contains("alice"),
            "users are identified by uid above debug, never by name: {line}"
        );
    }
}

/// PAU16: an unchanged skip reason is logged once, a changed one again.
#[tokio::test]
async fn test_pau_repeated_skip_reason_is_logged_once() {
    test_clock!(OFFSET, clock);
    let mut fx = build_worker(
        PipelineOptions::new(clock),
        fast_presence_config(),
        ScriptedAccountGuard::always(AccountState::Usable),
    )
    .await;
    let (logs, _guard) = capture_logs();
    let code = SkipReason::NoLockedSession.as_str();
    for _ in 0..5 {
        assert_eq!(
            fx.worker.tick().await,
            PresenceTick::Skipped(SkipReason::NoLockedSession)
        );
        advance(&OFFSET, 1000);
    }
    let occurrences = logs.contents().matches(code).count();
    assert_eq!(occurrences, 1, "five identical skips must log once");

    fx.logind
        .set_sessions(vec![locked_session("2", 1000, "alice")]);
    fx.logind.set_lid(Ok(true));
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::InGrace)
    );
    fx.logind.set_sessions(vec![]);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::NoLockedSession)
    );
    assert_eq!(
        logs.contents().matches(code).count(),
        2,
        "a reason logged again after a change"
    );
}

/// PAU19: without a system bus the worker warns once and backs off; a later recovery and
/// a new outage warn again (transition-only).
#[tokio::test]
async fn test_pau_missing_system_bus_warns_once_per_transition() {
    test_clock!(OFFSET, clock);
    let mut fx = build_worker(
        PipelineOptions::new(clock),
        fast_presence_config(),
        ScriptedAccountGuard::always(AccountState::Usable),
    )
    .await;
    let (logs, _guard) = capture_logs();
    fx.logind
        .set_sessions_error(PresenceLogindError::BusUnavailable);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::LogindUnavailable)
    );
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::Backoff)
    );
    advance(&OFFSET, 1100);
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::LogindUnavailable)
    );
    let warns = logs
        .contents()
        .lines()
        .filter(|l| l.contains(" WARN "))
        .count();
    assert_eq!(
        warns,
        1,
        "an ongoing outage is warned once:\n{}",
        logs.contents()
    );

    advance(&OFFSET, 2100);
    tokio::time::sleep(Duration::from_millis(2100)).await;
    fx.logind.set_sessions(vec![]);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::NoLockedSession)
    );
    fx.logind
        .set_sessions_error(PresenceLogindError::BusUnavailable);
    assert_eq!(
        fx.worker.tick().await,
        PresenceTick::Skipped(SkipReason::LogindUnavailable)
    );
    let warns = logs
        .contents()
        .lines()
        .filter(|l| l.contains(" WARN "))
        .count();
    assert_eq!(warns, 2, "a new outage after a recovery is warned again");
    assert_eq!(fx.logind.unlock_calls(), 0);
}

/// F6: lid/screen gate transitions are logged at `info`, value-free, once per transition.
#[tokio::test]
async fn test_pau_gate_transitions_are_logged_at_info_once() {
    test_clock!(OFFSET, clock);
    let mut fx = build_worker(
        PipelineOptions::new(clock),
        fast_presence_config(),
        ScriptedAccountGuard::always(AccountState::Usable),
    )
    .await;
    let (logs, _guard) = capture_logs();
    fx.logind
        .set_sessions(vec![locked_session("2", 1000, "alice")]);
    fx.logind.set_lid(Ok(true));
    let _ = fx.worker.tick().await;
    for _ in 0..3 {
        advance(&OFFSET, 1000);
        assert_eq!(
            fx.worker.tick().await,
            PresenceTick::Skipped(SkipReason::LidClosed)
        );
    }
    assert_eq!(
        count(&logs, "INFO", "lid_closed"),
        1,
        "one info line when the gate closes:\n{}",
        logs.contents()
    );
    fx.logind.set_lid(Ok(false));
    advance(&OFFSET, 1000);
    let _ = fx.worker.tick().await;
    assert_eq!(
        count(&logs, "INFO", "lid_closed"),
        2,
        "one more info line when the gate opens again"
    );
    assert!(
        count(&logs, "INFO", "display_state") >= 1,
        "the gate line names the display state"
    );
}

// ---------------------------------------------------------------------------------------
// Round 2 (auditor T6): account sources never reach the logs
// ---------------------------------------------------------------------------------------

const HASH_MARKER: &str = "HASHMARKERq7";
const SOURCE_MARKER: &str = "SRCMARKERz9";
const CONF_MARKER: &str = "confmarkerkey";
const USER: &str = "logmarkuser";
const NOW_S: u64 = 20_000 * 86_400;

fn tally(records: &[(u16, u64)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (status, time) in records {
        let mut r = [0u8; 64];
        r[..SOURCE_MARKER.len()].copy_from_slice(SOURCE_MARKER.as_bytes());
        r[54..56].copy_from_slice(&status.to_ne_bytes());
        r[56..64].copy_from_slice(&time.to_ne_bytes());
        out.extend_from_slice(&r);
    }
    out
}

fn fixed_now() -> Result<u64, soos_daemon::DaemonError> {
    Ok(NOW_S)
}

/// One account-guard scenario: `(label, faillock.conf, shadow, tally bytes, PAM stack)`.
type GuardLogCase = (&'static str, String, String, Vec<u8>, Option<&'static str>);

/// T6: across usable, faillocked, expired, malformed-shadow, unknown-conf-key and PAM
/// option paths, no shadow hash, tally source bytes or file content is logged at any
/// level, and the user name never appears above debug.
#[test]
fn test_pau_account_guard_never_logs_its_sources() {
    use soos_daemon::presence::account::{AccountGuard, SystemAccountGuard, UserName};
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let tally_dir = root.join("faillock");
    std::fs::create_dir_all(&tally_dir).unwrap();
    std::fs::set_permissions(&tally_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    let pam = root.join("pam.d");
    std::fs::create_dir_all(&pam).unwrap();
    let conf = root.join("faillock.conf");
    let shadow = root.join("shadow");
    let good_conf = format!("dir = {}\n", tally_dir.display());
    let good_shadow = format!("{USER}:$6${HASH_MARKER}:19900:0:99999:7:::\n");
    let guard = SystemAccountGuard::new()
        .with_faillock_conf(conf.clone())
        .with_vendor_faillock_conf(root.join("vendor.conf"))
        .with_default_faillock_dir(tally_dir.clone())
        .with_pam_dirs(vec![pam.clone()])
        .with_shadow(shadow.clone())
        .with_realtime_fn(fixed_now);
    let user = UserName::parse(USER).unwrap();
    let (logs, _guard) = capture_logs();
    let scenarios: Vec<GuardLogCase> = vec![
        (
            "usable",
            good_conf.clone(),
            good_shadow.clone(),
            Vec::new(),
            None,
        ),
        (
            "faillocked",
            good_conf.clone(),
            good_shadow.clone(),
            tally(&[(1, NOW_S - 1), (1, NOW_S - 2), (1, NOW_S - 3)]),
            None,
        ),
        (
            "expired",
            good_conf.clone(),
            format!("{USER}:$6${HASH_MARKER}:19900:0:99999:7::1:\n"),
            Vec::new(),
            None,
        ),
        (
            "malformed shadow",
            good_conf.clone(),
            format!("{USER}:$6${HASH_MARKER}:x:0:99999:7:::\n"),
            Vec::new(),
            None,
        ),
        (
            "unknown conf key",
            format!("{good_conf}{CONF_MARKER} = 1\n"),
            good_shadow.clone(),
            Vec::new(),
            None,
        ),
        (
            "pam option",
            good_conf.clone(),
            good_shadow.clone(),
            Vec::new(),
            Some("auth required pam_faillock.so deny=1 confmarkerkey\n"),
        ),
    ];
    for (label, conf_text, shadow_text, tally_bytes, pam_text) in scenarios {
        std::fs::write(&conf, conf_text).unwrap();
        std::fs::write(&shadow, shadow_text).unwrap();
        std::fs::write(tally_dir.join(USER), tally_bytes).unwrap();
        let stack = pam.join("system-auth");
        match pam_text {
            Some(text) => std::fs::write(&stack, text).unwrap(),
            None => {
                let _ = std::fs::remove_file(&stack);
            }
        }
        let _ = guard.check(&user, 1000);
        let contents = logs.contents();
        assert!(
            !contents.contains(HASH_MARKER),
            "{label}: shadow hash logged"
        );
        assert!(
            !contents.contains(SOURCE_MARKER),
            "{label}: tally source logged"
        );
        assert!(
            !contents.contains(CONF_MARKER),
            "{label}: file content logged"
        );
        for line in lines_at_or_above_info(&logs) {
            assert!(
                !line.contains(USER),
                "{label}: user name above debug: {line}"
            );
        }
    }
}
