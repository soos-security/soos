//! Contract tests for the [`pam_soos::PamFeedback`] seam (GitHub #220, review PAM-07;
//! GitHub #227, review PAM-15): the authentication flow reaches the PAM conversation,
//! the user lookup and the `PAM_SERVICE` item ONLY through this trait, so a recorder
//! can assert message selection, `PAM_SILENT` and the user lookup without any libpam
//! handle (real or fake).
//!
//! Every pathway asserts the `PAM_IGNORE` fallback (never `PAM_SUCCESS` on failure).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

mod common;

use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixListener;
use std::path::Path;

use common::{spawn_daemon, SeenRequest, LOOKING_FOR_FACE};
use pam_bindings::constants::{PAM_DISALLOW_NULL_AUTHTOK, PAM_SILENT};
use pam_soos::config::{PamConfig, PamEvent};
use pam_soos::{PamFeedback, PamFlag, PamResultCode, SoosPam};
use soos_protocol::types::{ReasonClass, Verdict};
use tempfile::tempdir;

/// Test double recording every call the module makes on its PAM environment.
#[derive(Default)]
struct Recorder {
    infos: Vec<String>,
    user: Option<String>,
    service: Option<Vec<u8>>,
    user_calls: usize,
    service_calls: usize,
}

impl PamFeedback for Recorder {
    fn info(&mut self, msg: &str) {
        self.infos.push(msg.to_owned());
    }

    fn user(&mut self) -> Option<String> {
        self.user_calls += 1;
        self.user.clone()
    }

    fn service(&mut self) -> Option<Vec<u8>> {
        self.service_calls += 1;
        self.service.clone()
    }
}

fn config(socket: &Path, flag_dir: &Path, uid: Option<u32>) -> PamConfig {
    PamConfig {
        timeout_ms: 1000,
        socket_path: socket.to_path_buf(),
        uid,
        flag_dir: flag_dir.to_path_buf(),
        ..Default::default()
    }
}

/// Runs one authentication through `recorder` against a daemon answering `verdict`.
fn run(
    recorder: &mut Recorder,
    uid: Option<u32>,
    verdict: Verdict,
    flags: PamFlag,
) -> (PamResultCode, Option<SeenRequest>) {
    let tmp = tempdir().unwrap();
    let sock = tmp.path().join("daemon.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let daemon = spawn_daemon(listener, verdict, ReasonClass::FaceMatch);
    let cfg = config(&sock, tmp.path(), uid);
    let code = SoosPam::authenticate_with_feedback(recorder, &cfg, flags);
    (code, daemon.join().unwrap())
}

// ---------------------------------------------------------------------------
// Message selection
// ---------------------------------------------------------------------------

#[test]
fn test_feedback_allow_emits_lookup_then_recognized() {
    let mut rec = Recorder::default();
    let (code, seen) = run(&mut rec, Some(1000), Verdict::Allow, 0);
    assert_eq!(code, PamResultCode::PAM_SUCCESS);
    assert!(seen.is_some());
    assert_eq!(
        rec.infos,
        vec![LOOKING_FOR_FACE, "[soos] Face recognized. Unlocking..."]
    );
}

#[test]
fn test_feedback_deny_emits_lookup_then_not_recognized() {
    let mut rec = Recorder::default();
    let (code, _) = run(&mut rec, Some(1000), Verdict::Deny, 0);
    assert_eq!(code, PamResultCode::PAM_IGNORE);
    assert_eq!(
        rec.infos,
        vec![LOOKING_FOR_FACE, "[soos] Face not recognized."]
    );
}

#[test]
fn test_feedback_unavailable_and_protocol_error_emit_generic_text() {
    for verdict in [Verdict::Unavailable, Verdict::ProtocolError] {
        let mut rec = Recorder::default();
        let (code, _) = run(&mut rec, Some(1000), verdict, 0);
        assert_eq!(code, PamResultCode::PAM_IGNORE, "{verdict:?}");
        assert_eq!(
            rec.infos,
            vec![LOOKING_FOR_FACE, "[soos] Face verification unavailable."],
            "{verdict:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// PAM_SILENT
// ---------------------------------------------------------------------------

#[test]
fn test_feedback_pam_silent_suppresses_every_message_for_every_verdict() {
    for verdict in [
        Verdict::Allow,
        Verdict::Deny,
        Verdict::Unavailable,
        Verdict::ProtocolError,
    ] {
        for flags in [PAM_SILENT, PAM_SILENT | PAM_DISALLOW_NULL_AUTHTOK] {
            let mut rec = Recorder::default();
            let (code, seen) = run(&mut rec, Some(1000), verdict, flags);
            assert!(seen.is_some(), "PAM_SILENT must not skip the daemon");
            let expected = if verdict == Verdict::Allow {
                PamResultCode::PAM_SUCCESS
            } else {
                PamResultCode::PAM_IGNORE
            };
            assert_eq!(code, expected, "{verdict:?} flags={flags:#x}");
            assert!(
                rec.infos.is_empty(),
                "{verdict:?} flags={flags:#x} emitted {:?}",
                rec.infos
            );
        }
    }
}

#[test]
fn test_feedback_pam_silent_daemon_absent_emits_nothing() {
    let tmp = tempdir().unwrap();
    let cfg = config(&tmp.path().join("absent.sock"), tmp.path(), Some(1000));
    let mut rec = Recorder::default();
    let code = SoosPam::authenticate_with_feedback(&mut rec, &cfg, PAM_SILENT);
    assert_eq!(code, PamResultCode::PAM_IGNORE);
    assert!(rec.infos.is_empty(), "got {:?}", rec.infos);
}

// ---------------------------------------------------------------------------
// Daemon not installed / not running
// ---------------------------------------------------------------------------

#[test]
fn test_feedback_daemon_absent_never_announces_face_lookup() {
    let tmp = tempdir().unwrap();
    let cfg = config(&tmp.path().join("absent.sock"), tmp.path(), Some(1000));
    let mut rec = Recorder::default();
    let code = SoosPam::authenticate_with_feedback(&mut rec, &cfg, 0);
    assert_eq!(code, PamResultCode::PAM_IGNORE);
    assert!(
        !rec.infos.iter().any(|m| m == LOOKING_FOR_FACE),
        "got {:?}",
        rec.infos
    );
    assert!(rec.infos.len() <= 1, "got {:?}", rec.infos);
}

#[test]
fn test_feedback_connection_refused_never_announces_face_lookup() {
    let tmp = tempdir().unwrap();
    let sock = tmp.path().join("stale.sock");
    drop(UnixListener::bind(&sock).unwrap());
    let cfg = config(&sock, tmp.path(), Some(1000));
    let mut rec = Recorder::default();
    let code = SoosPam::authenticate_with_feedback(&mut rec, &cfg, 0);
    assert_eq!(code, PamResultCode::PAM_IGNORE);
    assert!(
        !rec.infos.iter().any(|m| m == LOOKING_FOR_FACE),
        "got {:?}",
        rec.infos
    );
    assert!(rec.infos.len() <= 1, "got {:?}", rec.infos);
}

// ---------------------------------------------------------------------------
// User lookup (previously only reachable with a real libpam handle)
// ---------------------------------------------------------------------------

#[test]
fn test_feedback_user_lookup_resolves_uid_when_no_uid_argument() {
    let mut rec = Recorder {
        user: Some("root".to_owned()),
        ..Default::default()
    };
    let (code, seen) = run(&mut rec, None, Verdict::Allow, 0);
    assert_eq!(code, PamResultCode::PAM_SUCCESS);
    assert_eq!(rec.user_calls, 1);
    assert_eq!(seen.unwrap().uid_hint, 0, "root must resolve to uid 0");
}

#[test]
fn test_feedback_user_lookup_skipped_when_uid_argument_present() {
    let mut rec = Recorder {
        user: Some("root".to_owned()),
        ..Default::default()
    };
    let (_, seen) = run(&mut rec, Some(4242), Verdict::Deny, 0);
    assert_eq!(rec.user_calls, 0, "uid= must take precedence over PAM_USER");
    assert_eq!(seen.unwrap().uid_hint, 4242);
}

#[test]
fn test_feedback_unknown_user_falls_back_to_process_uid() {
    let process_uid = std::fs::metadata("/proc/self").unwrap().uid();
    for user in [None, Some("soos-no-such-user-contract".to_owned())] {
        let mut rec = Recorder {
            user,
            ..Default::default()
        };
        let (code, seen) = run(&mut rec, None, Verdict::Deny, 0);
        assert_eq!(code, PamResultCode::PAM_IGNORE);
        assert_eq!(rec.user_calls, 1);
        assert_eq!(seen.unwrap().uid_hint, process_uid);
    }
}

// ---------------------------------------------------------------------------
// PAM_SERVICE item and disable flags through the seam
// ---------------------------------------------------------------------------

#[test]
fn test_feedback_service_item_forwarded_and_gdm_disable_honored() {
    let mut rec = Recorder {
        service: Some(b"gdm-password".to_vec()),
        ..Default::default()
    };
    let (_, seen) = run(&mut rec, Some(1000), Verdict::Deny, 0);
    assert_eq!(seen.unwrap().service, "gdm-password");

    let tmp = tempdir().unwrap();
    let sock = tmp.path().join("daemon.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    listener.set_nonblocking(true).unwrap();
    std::fs::write(tmp.path().join("gdm.disable"), "").unwrap();
    let mut rec = Recorder {
        service: Some(b"gdm-password".to_vec()),
        user: Some("root".to_owned()),
        ..Default::default()
    };
    let code = SoosPam::authenticate_with_feedback(&mut rec, &config(&sock, tmp.path(), None), 0);
    assert_eq!(code, PamResultCode::PAM_IGNORE);
    assert!(listener.accept().is_err(), "disabled: no daemon contact");
    assert!(
        rec.infos.is_empty(),
        "disabled: silent, got {:?}",
        rec.infos
    );
    assert_eq!(rec.user_calls, 0, "disabled: no user lookup");
}

#[test]
fn test_feedback_password_failed_event_is_silent() {
    let tmp = tempdir().unwrap();
    let mut cfg = config(&tmp.path().join("absent.sock"), tmp.path(), Some(1000));
    cfg.event = Some(PamEvent::PasswordFailed);
    let mut rec = Recorder::default();
    let code = SoosPam::authenticate_with_feedback(&mut rec, &cfg, 0);
    assert_eq!(code, PamResultCode::PAM_IGNORE);
    assert!(rec.infos.is_empty(), "got {:?}", rec.infos);
}
