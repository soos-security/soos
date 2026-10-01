//! Contract tests for the fail-quiet decision of GitHub #221 (user decision 2026-10-01):
//! when the daemon socket cannot be connected (not installed or stopped) the module sends
//! no conversation message at all and still returns `PAM_IGNORE`, through the REAL C ABI
//! entry point with a REAL Linux-PAM handle.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

mod common;

use std::ffi::CString;
use std::os::unix::net::UnixListener;
use std::path::Path;

use common::with_pam_handle;
use pam_soos::{pam_sm_authenticate, PAM_IGNORE};
use tempfile::tempdir;

/// Unique service name so no host flag file (`/etc/soos/<service>.disable`) applies.
const SERVICE: &str = "soos-fail-quiet-contract";

fn call_c_entry(socket: &Path) -> (i32, Vec<String>) {
    let args = [
        CString::new(format!("socket={}", socket.display())).unwrap(),
        CString::new("timeout_ms=1000").unwrap(),
        CString::new("uid=1000").unwrap(),
    ];
    let argv: Vec<*const u8> = args.iter().map(|a| a.as_ptr().cast::<u8>()).collect();
    let argc = i32::try_from(argv.len()).unwrap();
    with_pam_handle(SERVICE, |h| {
        pam_sm_authenticate(std::ptr::from_mut(h), 0, argc, argv.as_ptr())
    })
}

#[test]
fn test_221_daemon_not_installed_sends_no_message() {
    let tmp = tempdir().unwrap();
    let (code, messages) = call_c_entry(&tmp.path().join("absent.sock"));
    assert_eq!(code, PAM_IGNORE);
    assert!(messages.is_empty(), "got {messages:?}");
}

#[test]
fn test_221_daemon_stopped_sends_no_message() {
    let tmp = tempdir().unwrap();
    let sock = tmp.path().join("stale.sock");
    drop(UnixListener::bind(&sock).unwrap());
    let (code, messages) = call_c_entry(&sock);
    assert_eq!(code, PAM_IGNORE);
    assert!(messages.is_empty(), "got {messages:?}");
}
