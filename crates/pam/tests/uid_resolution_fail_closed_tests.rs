//! Fail-closed username -> UID resolution and the `uid=` / PAM_USER match (GitHub #300,
//! #302; owner decisions 2026-10-02; matrix rows PUR1-PUR8).
//!
//! - #300: a PAM user that cannot be resolved (missing, unknown, non-UTF-8, not a C string,
//!   any `getpwnam_r` failure) never falls back to the caller's real UID. In a setuid-root
//!   host (`su B` from A, `polkit-agent-helper-1 B`) that fallback sent A's UID for
//!   PAM_USER = B, so A's face unlocked B. The module now returns `PAM_IGNORE` without
//!   contacting the daemon, and sends no `event=password-failed` event.
//! - #302: `uid=` is honoured only when PAM_USER resolves to the same UID; otherwise
//!   `PAM_IGNORE` without contacting the daemon.
//! - The `getuid()` stand-in survives only on the null-handle (`Detached`) path, which
//!   libpam never uses.
//!
//! Every pathway asserts the `PAM_IGNORE` fallback (never `PAM_SUCCESS` on failure).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

mod common;

use std::ffi::{CStr, CString};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixListener;
use std::path::Path;

use common::{
    assert_never_contacted, silent_listener, spawn_daemon, with_pam_handle_as, CONTRACT_USER,
};
use pam_soos::config::{parse_cstrs, PamConfig, PamEvent};
use pam_soos::{pam_sm_authenticate, PamResultCode, SoosPam, PAM_IGNORE, PAM_SUCCESS};
use soos_protocol::types::{ReasonClass, Verdict};
use tempfile::tempdir;

/// Unique service name so no host flag file (`/etc/soos/<service>.disable`) applies.
const SERVICE: &str = "soos-uid-resolution-contract";

/// A user name no test host defines.
const UNKNOWN_USER: &CStr = c"nonexistent_soos_user_pur";

/// Real UID of this test process.
fn process_uid() -> u32 {
    std::fs::metadata("/proc/self").unwrap().uid()
}

fn module_args(socket: &Path, extra: &[&str]) -> Vec<CString> {
    let mut args = vec![
        CString::new(format!("socket={}", socket.display())).unwrap(),
        CString::new("timeout_ms=1000").unwrap(),
    ];
    args.extend(extra.iter().map(|a| CString::new(*a).unwrap()));
    args
}

/// Calls the exported C entry point with a REAL `pam_start` handle for `user` and asserts
/// that the module never connected to the daemon socket.
fn c_entry_unreached(user: &CStr, extra: &[&str]) -> (i32, Vec<String>) {
    let tmp = tempdir().unwrap();
    let sock = tmp.path().join("daemon.sock");
    let listener = silent_listener(&sock);
    let args = module_args(&sock, extra);
    let argv: Vec<*const u8> = args.iter().map(|a| a.as_ptr().cast::<u8>()).collect();
    let argc = i32::try_from(argv.len()).unwrap();
    let out = with_pam_handle_as(SERVICE, user, |h| {
        pam_sm_authenticate(std::ptr::from_mut(h), 0, argc, argv.as_ptr())
    });
    assert_never_contacted(&listener, "real handle");
    out
}

/// PUR1 (#300, reviewer PoC): a real handle whose PAM user does not exist returns
/// `PAM_IGNORE` and never contacts the daemon (previously `uid_hint = getuid()` reached
/// an `Allow` daemon and returned `PAM_SUCCESS`).
#[test]
fn test_pur_real_handle_nonexistent_user_returns_ignore_without_daemon_contact() {
    let (code, messages) = c_entry_unreached(UNKNOWN_USER, &[]);
    assert_eq!(code, PAM_IGNORE);
    assert!(messages.is_empty(), "got {messages:?}");
}

/// PUR1 (#300): same through `SoosPam::authenticate_with_config(Some(handle))`, the
/// `PamHooks` path; also with `event=password-failed` (no event sent).
#[test]
fn test_pur_real_handle_nonexistent_user_rust_entry_and_event_path() {
    for event in [None, Some(PamEvent::PasswordFailed)] {
        let tmp = tempdir().unwrap();
        let sock = tmp.path().join("daemon.sock");
        let listener = silent_listener(&sock);
        let config = PamConfig {
            timeout_ms: 1000,
            socket_path: sock.clone(),
            flag_dir: tmp.path().to_path_buf(),
            event,
            ..Default::default()
        };
        let (code, messages) = with_pam_handle_as(SERVICE, UNKNOWN_USER, |h| {
            SoosPam::authenticate_with_config(Some(h), &config)
        });
        assert_eq!(code, PamResultCode::PAM_IGNORE, "{event:?}");
        assert!(messages.is_empty(), "{event:?}: got {messages:?}");
        assert_never_contacted(&listener, "nonexistent user");
    }
}

/// PUR2 (#300): a non-UTF-8 PAM user name is unresolvable: `PAM_IGNORE`, no daemon
/// contact.
#[test]
fn test_pur_real_handle_non_utf8_user_returns_ignore_without_daemon_contact() {
    let (code, _) = c_entry_unreached(c"r\xffoot", &[]);
    assert_eq!(code, PAM_IGNORE);
}

/// PUR3 (#300): no password-failed event is sent for an unresolvable PAM user.
#[test]
fn test_pur_real_handle_nonexistent_user_sends_no_password_failed_event() {
    let (code, _) = c_entry_unreached(UNKNOWN_USER, &["event=password-failed"]);
    assert_eq!(code, PAM_IGNORE);
}

/// PUR4 (#302, issue test): PAM user `root` with `uid=1000` and an `Allow` daemon returns
/// `PAM_IGNORE` and sends no request; same on the event path.
#[test]
fn test_pur_real_handle_uid_argument_mismatching_pam_user_returns_ignore() {
    let (code, messages) = c_entry_unreached(CONTRACT_USER, &["uid=1000"]);
    assert_eq!(code, PAM_IGNORE);
    assert!(messages.is_empty(), "got {messages:?}");
    let (code, _) = c_entry_unreached(CONTRACT_USER, &["uid=1000", "event=password-failed"]);
    assert_eq!(code, PAM_IGNORE);
}

/// PUR4 (#302): a nonexistent PAM user with any `uid=` is still refused (the argument is
/// never a substitute for an unresolvable user).
#[test]
fn test_pur_real_handle_uid_argument_does_not_rescue_unknown_user() {
    let uid_arg = format!("uid={}", process_uid());
    let (code, _) = c_entry_unreached(UNKNOWN_USER, &[uid_arg.as_str()]);
    assert_eq!(code, PAM_IGNORE);
}

/// PUR5 (#302, control): PAM user `root` with `uid=0` is honoured: the request reaches an
/// `Allow` daemon with `uid_hint = 0` and authenticates.
#[test]
fn test_pur_real_handle_uid_argument_matching_pam_user_authenticates() {
    let tmp = tempdir().unwrap();
    let sock = tmp.path().join("daemon.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let daemon = spawn_daemon(listener, Verdict::Allow, ReasonClass::FaceMatch);
    let args = module_args(&sock, &["uid=0"]);
    let refs: Vec<&CStr> = args.iter().map(CString::as_c_str).collect();
    let config = parse_cstrs(refs);
    let (code, _) = with_pam_handle_as(SERVICE, CONTRACT_USER, |h| {
        SoosPam::authenticate_with_config(Some(h), &config)
    });
    assert_eq!(code, PamResultCode::PAM_SUCCESS);
    assert_eq!(daemon.join().unwrap().unwrap().uid_hint, 0);
}

/// PUR6 (#300): an injected resolver reporting failure (`None`) returns `PAM_IGNORE` and
/// never contacts the daemon, on both the authentication and the event path.
#[test]
fn test_pur_resolver_failure_returns_ignore_without_daemon_contact() {
    for event in [None, Some(PamEvent::PasswordFailed)] {
        let tmp = tempdir().unwrap();
        let sock = tmp.path().join("daemon.sock");
        let listener = silent_listener(&sock);
        let config = PamConfig {
            timeout_ms: 1000,
            socket_path: sock.clone(),
            flag_dir: tmp.path().to_path_buf(),
            event,
            ..Default::default()
        };
        let code = SoosPam::authenticate_with_uid_resolver(None, &config, |_| None);
        assert_eq!(code, PamResultCode::PAM_IGNORE, "{event:?}");
        assert_never_contacted(&listener, "resolver failure");
    }
}

/// Runs the null-handle C entry point (detached path) with `uid=<uid>` against an
/// `Allow` daemon reached through a non-blocking listener.
fn detached_c_entry(uid: u32, expect_contact: bool) -> i32 {
    let tmp = tempdir().unwrap();
    let sock = tmp.path().join("daemon.sock");
    let uid_arg = format!("uid={uid}");
    let args = module_args(&sock, &[uid_arg.as_str()]);
    let argv: Vec<*const u8> = args.iter().map(|a| a.as_ptr().cast::<u8>()).collect();
    let argc = i32::try_from(argv.len()).unwrap();
    if expect_contact {
        let listener = UnixListener::bind(&sock).unwrap();
        let daemon = spawn_daemon(listener, Verdict::Allow, ReasonClass::FaceMatch);
        let code = pam_sm_authenticate(std::ptr::null_mut(), 0, argc, argv.as_ptr());
        assert_eq!(daemon.join().unwrap().unwrap().uid_hint, uid);
        code
    } else {
        let listener = silent_listener(&sock);
        let code = pam_sm_authenticate(std::ptr::null_mut(), 0, argc, argv.as_ptr());
        assert_never_contacted(&listener, "detached uid mismatch");
        code
    }
}

/// PUR7: the null-handle (`Detached`) path keeps the process UID as its PAM-user stand-in
/// (libpam never passes a null handle): `uid=` equal to it authenticates, any other
/// `uid=` falls back without contacting the daemon.
#[test]
fn test_pur_detached_path_matches_uid_argument_against_process_uid() {
    let me = process_uid();
    assert_eq!(detached_c_entry(me, true), PAM_SUCCESS);
    assert_eq!(detached_c_entry(me.wrapping_add(1), false), PAM_IGNORE);
}
