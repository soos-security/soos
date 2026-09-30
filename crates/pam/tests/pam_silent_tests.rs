//! Contract tests for `PAM_SILENT` and the daemon-absent conversation (GitHub #221,
//! review PAM-08), driven through the REAL entry points (`pam_sm_authenticate` C ABI and
//! `PamHooks::sm_authenticate`) with a REAL Linux-PAM handle (GitHub #227, review PAM-15).
//!
//! - With `PAM_SILENT` in `flags`, the module never calls the conversation function
//!   (Linux-PAM module writer's guide), whatever the outcome.
//! - When the daemon socket cannot be connected (not installed, stopped), the module never
//!   announces a face lookup it cannot perform.
//!
//! Every pathway asserts the `PAM_IGNORE` fallback (never `PAM_SUCCESS` on failure).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

mod common;

use std::ffi::{CStr, CString};
use std::os::unix::net::UnixListener;
use std::path::Path;

use common::{spawn_daemon, with_pam_handle, LOOKING_FOR_FACE};
use pam_bindings::constants::{PAM_DISALLOW_NULL_AUTHTOK, PAM_SILENT};
use pam_soos::{pam_sm_authenticate, PamHooks, PamResultCode, SoosPam, PAM_IGNORE, PAM_SUCCESS};
use soos_protocol::types::{ReasonClass, Verdict};
use tempfile::tempdir;

/// Unique service name so no host flag file (`/etc/soos/<service>.disable`) applies.
const SERVICE: &str = "soos-silent-contract";

fn module_args(socket: &Path) -> Vec<CString> {
    vec![
        CString::new(format!("socket={}", socket.display())).unwrap(),
        CString::new("timeout_ms=1000").unwrap(),
        CString::new("uid=1000").unwrap(),
    ]
}

/// Calls the C ABI entry point with a real handle, `flags` and the module arguments.
fn call_c_entry(flags: i32, socket: &Path) -> (i32, Vec<String>) {
    let args = module_args(socket);
    let argv: Vec<*const u8> = args.iter().map(|a| a.as_ptr().cast::<u8>()).collect();
    let argc = i32::try_from(argv.len()).unwrap();
    with_pam_handle(SERVICE, |h| {
        pam_sm_authenticate(std::ptr::from_mut(h), flags, argc, argv.as_ptr())
    })
}

/// Runs the C entry point against a one-shot daemon answering `verdict`.
fn c_entry_with_daemon(flags: i32, verdict: Verdict) -> (i32, Vec<String>) {
    let tmp = tempdir().unwrap();
    let sock = tmp.path().join("daemon.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let daemon = spawn_daemon(listener, verdict, ReasonClass::FaceMatch);
    let out = call_c_entry(flags, &sock);
    let seen = daemon.join().unwrap();
    assert!(seen.is_some(), "the daemon must have been contacted");
    out
}

// ---------------------------------------------------------------------------
// PAM_SILENT through the C ABI entry point
// ---------------------------------------------------------------------------

/// `PAM_SILENT` + daemon `Allow`: authenticated, and not a single conversation call.
#[test]
fn test_c_entry_pam_silent_allow_emits_no_conversation() {
    let (code, messages) = c_entry_with_daemon(PAM_SILENT as i32, Verdict::Allow);
    assert_eq!(code, PAM_SUCCESS);
    assert!(
        messages.is_empty(),
        "PAM_SILENT must suppress every PAM_TEXT_INFO, got {messages:?}"
    );
}

/// `PAM_SILENT` + daemon `Deny`: password fallback, no conversation.
#[test]
fn test_c_entry_pam_silent_deny_emits_no_conversation() {
    let (code, messages) = c_entry_with_daemon(PAM_SILENT as i32, Verdict::Deny);
    assert_eq!(code, PAM_IGNORE);
    assert!(messages.is_empty(), "got {messages:?}");
}

/// `PAM_SILENT` + daemon absent: password fallback, no conversation.
#[test]
fn test_c_entry_pam_silent_daemon_absent_emits_no_conversation() {
    let tmp = tempdir().unwrap();
    let (code, messages) = call_c_entry(PAM_SILENT as i32, &tmp.path().join("absent.sock"));
    assert_eq!(code, PAM_IGNORE);
    assert!(messages.is_empty(), "got {messages:?}");
}

/// `PAM_SILENT` combined with other flag bits is still silent (bit test, not equality).
#[test]
fn test_c_entry_pam_silent_combined_with_other_flags_is_silent() {
    let flags = (PAM_SILENT | PAM_DISALLOW_NULL_AUTHTOK) as i32;
    let (code, messages) = c_entry_with_daemon(flags, Verdict::Allow);
    assert_eq!(code, PAM_SUCCESS);
    assert!(messages.is_empty(), "got {messages:?}");
}

/// Without `PAM_SILENT` the connected flow keeps its two informational messages.
#[test]
fn test_c_entry_without_silent_keeps_lookup_and_verdict_messages() {
    let (code, messages) = c_entry_with_daemon(PAM_DISALLOW_NULL_AUTHTOK as i32, Verdict::Allow);
    assert_eq!(code, PAM_SUCCESS);
    assert_eq!(
        messages,
        vec![
            LOOKING_FOR_FACE.to_owned(),
            "[soos] Face recognized. Unlocking...".to_owned()
        ]
    );
}

// ---------------------------------------------------------------------------
// PAM_SILENT through PamHooks::sm_authenticate
// ---------------------------------------------------------------------------

/// The `PamHooks` entry point honors `PAM_SILENT` as well.
#[test]
fn test_pamhooks_sm_authenticate_pam_silent_emits_no_conversation() {
    let tmp = tempdir().unwrap();
    let sock = tmp.path().join("daemon.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let daemon = spawn_daemon(listener, Verdict::Deny, ReasonClass::NoFace);
    let args = module_args(&sock);
    let (code, messages) = with_pam_handle(SERVICE, |h| {
        let refs: Vec<&CStr> = args.iter().map(CString::as_c_str).collect();
        SoosPam::sm_authenticate(h, refs, PAM_SILENT)
    });
    assert!(daemon.join().unwrap().is_some());
    assert_eq!(code, PamResultCode::PAM_IGNORE);
    assert!(messages.is_empty(), "got {messages:?}");
}

// ---------------------------------------------------------------------------
// Daemon not installed / not running
// ---------------------------------------------------------------------------

/// Socket path absent (`ENOENT`): no "Looking for face..." announcement and at most one
/// line of output, still `PAM_IGNORE`.
#[test]
fn test_c_entry_daemon_absent_never_announces_face_lookup() {
    let tmp = tempdir().unwrap();
    let (code, messages) = call_c_entry(0, &tmp.path().join("absent.sock"));
    assert_eq!(code, PAM_IGNORE);
    assert!(
        !messages.iter().any(|m| m == LOOKING_FOR_FACE),
        "an unreachable daemon must not announce a face lookup, got {messages:?}"
    );
    assert!(messages.len() <= 1, "got {messages:?}");
}

/// Stale socket file with no listener (`ECONNREFUSED`, daemon stopped): same contract.
#[test]
fn test_c_entry_daemon_stopped_never_announces_face_lookup() {
    let tmp = tempdir().unwrap();
    let sock = tmp.path().join("stale.sock");
    drop(UnixListener::bind(&sock).unwrap());
    assert!(sock.exists(), "the stale socket file must remain");
    let (code, messages) = call_c_entry(0, &sock);
    assert_eq!(code, PAM_IGNORE);
    assert!(
        !messages.iter().any(|m| m == LOOKING_FOR_FACE),
        "a stopped daemon must not announce a face lookup, got {messages:?}"
    );
    assert!(messages.len() <= 1, "got {messages:?}");
}
