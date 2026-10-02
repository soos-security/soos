//! Contractual integration tests for pam-bindings 0.3.0 PamHooks migration and syslog logging.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Test harness assertions"
)]

mod common;

use pam_bindings::constants::PamResultCode;
use pam_bindings::module::{PamHandle, PamHooks};
use pam_soos::config::{parse_cstrs, PamEvent};
use pam_soos::syslog::{format_panic_message, log_panic};
use pam_soos::SoosPam;
use std::ffi::{CStr, CString};
use std::path::PathBuf;

/// PA11: Verify SoosPam implements PamHooks and defaults to PAM_IGNORE for unhandled hooks.
#[test]
fn test_pam_hooks_unhandled_hooks_return_ignore() {
    // Unhandled hooks must systematically return PAM_IGNORE
    let args: Vec<&CStr> = Vec::new();
    let dummy_ptr = 0x1000 as *mut PamHandle;
    // SAFETY: PamHandle is an opaque zero-sized type used only as a reference in trait dispatch.
    let pamh = unsafe { &mut *dummy_ptr };

    assert_eq!(
        SoosPam::acct_mgmt(pamh, args.clone(), 0),
        PamResultCode::PAM_IGNORE
    );
    assert_eq!(
        SoosPam::sm_setcred(pamh, args.clone(), 0),
        PamResultCode::PAM_IGNORE
    );
    assert_eq!(
        SoosPam::sm_chauthtok(pamh, args.clone(), 0),
        PamResultCode::PAM_IGNORE
    );
    assert_eq!(
        SoosPam::sm_open_session(pamh, args.clone(), 0),
        PamResultCode::PAM_IGNORE
    );
    assert_eq!(
        SoosPam::sm_close_session(pamh, args, 0),
        PamResultCode::PAM_IGNORE
    );
}

/// PA11 / PA1: sm_authenticate returns PAM_IGNORE when daemon socket is unreachable.
#[test]
fn test_pam_hooks_authenticate_offline_daemon_returns_ignore() {
    // User-approved 2026-09-30 (GitHub #220): a real `pam_start` handle replaces the
    // dummy 0x1000 pointer, so production code no longer needs an address heuristic.
    let sock_arg = CString::new("socket=/tmp/nonexistent_daemon_sock_for_test.sock").unwrap();
    let timeout_arg = CString::new("timeout_ms=50").unwrap();
    let uid_arg = CString::new("uid=0").unwrap();
    let args = vec![
        sock_arg.as_c_str(),
        timeout_arg.as_c_str(),
        uid_arg.as_c_str(),
    ];

    let (res, _) = common::with_pam_handle("soos-contract", |pamh| {
        SoosPam::sm_authenticate(pamh, args, 0)
    });
    assert_eq!(res, PamResultCode::PAM_IGNORE);
}

/// PA11 / PA1: sm_authenticate with event=password-failed returns PAM_IGNORE.
#[test]
fn test_pam_hooks_authenticate_password_failed_event_returns_ignore() {
    // User-approved 2026-09-30 (GitHub #220): a real `pam_start` handle replaces the
    // dummy 0x1000 pointer, so production code no longer needs an address heuristic.
    let event_arg = CString::new("event=password-failed").unwrap();
    let timeout_arg = CString::new("timeout_ms=20").unwrap();
    let uid_arg = CString::new("uid=0").unwrap();
    let sock_arg = CString::new("socket=/tmp/nonexistent_daemon_sock_for_test.sock").unwrap();
    let args = vec![
        event_arg.as_c_str(),
        timeout_arg.as_c_str(),
        uid_arg.as_c_str(),
        sock_arg.as_c_str(),
    ];

    let (res, _) = common::with_pam_handle("soos-contract", |pamh| {
        SoosPam::sm_authenticate(pamh, args, 0)
    });
    assert_eq!(res, PamResultCode::PAM_IGNORE);
}

/// PA12: Verify syslog panic message formatting and bounds.
#[test]
fn test_syslog_panic_message_formatting() {
    let msg = format_panic_message("simulated failure", Some("crates/pam/src/lib.rs:42:1"));
    assert_eq!(
        msg,
        "soos-pam: authentication panic caught at crates/pam/src/lib.rs:42:1: simulated failure"
    );

    let msg_no_loc = format_panic_message("payload without location", None);
    assert_eq!(
        msg_no_loc,
        "soos-pam: authentication panic caught: payload without location"
    );
}

/// PA12: Verify syslog panic formatting sanitizes internal nul bytes and never contains sensitive data.
#[test]
fn test_syslog_panic_message_sanitization() {
    let dirty = "malicious\0newline\ninjection";
    let formatted = format_panic_message(dirty, Some("src/test.rs\0:1:1"));
    // Nul bytes must be replaced with safe character (e.g. space or '?')
    assert!(!formatted.contains('\0'));
    assert!(formatted.contains("malicious"));
    assert!(formatted.contains("injection"));
}

/// PA12: Verify log_panic handles panics safely without itself panicking.
#[test]
fn test_syslog_log_panic_execution() {
    // Calling log_panic should execute safely (libc::syslog) without panicking
    log_panic(
        "test panic message for syslog audit",
        Some("crates/pam/tests/pam_bindings_tests.rs:10:5"),
    );
}

/// Test parse_cstrs safely extracts PAM module options from &CStr.
#[test]
fn test_parse_cstrs_options() {
    let arg1 = CString::new("timeout_ms=120").unwrap();
    let arg2 = CString::new("event=password-failed").unwrap();
    let arg3 = CString::new("socket=/tmp/custom.sock").unwrap();
    let arg4 = CString::new("uid=1001").unwrap();
    let arg5 = CString::new("service=myservice").unwrap();

    let args = vec![
        arg1.as_c_str(),
        arg2.as_c_str(),
        arg3.as_c_str(),
        arg4.as_c_str(),
        arg5.as_c_str(),
    ];

    let config = parse_cstrs(args);
    assert_eq!(config.timeout_ms, 120);
    assert_eq!(config.event, Some(PamEvent::PasswordFailed));
    assert_eq!(config.socket_path, PathBuf::from("/tmp/custom.sock"));
    assert_eq!(config.uid, Some(1001));
    assert_eq!(config.service, "myservice");
}

/// Test parse_cstrs with unknown and empty arguments preserves safe defaults.
#[test]
fn test_parse_cstrs_unknown_and_defaults() {
    let arg1 = CString::new("unknown_option=foobar").unwrap();
    let arg2 = CString::new("").unwrap();
    let args = vec![arg1.as_c_str(), arg2.as_c_str()];

    let config = parse_cstrs(args);
    assert_eq!(config.timeout_ms, 1000);
    assert_eq!(config.event, None);
    assert_eq!(config.socket_path, PathBuf::from("/run/soos/daemon.sock"));
    assert_eq!(config.service, "pam_soos");
}
