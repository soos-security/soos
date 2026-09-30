//! Contractual tests for the default enrollment target under `sudo` / `pkexec`
//! (GitHub #184 / STO-11): root is never an implicit target.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "Contractual test suite utilizes direct assertions and unwrap"
)]

use soos_enrollment_cli::args::{resolve_default_target_uid, resolve_target_uid};
use soos_enrollment_cli::error::EnrollmentCliError;

#[test]
fn test_default_target_under_sudo_uses_sudo_uid() {
    assert_eq!(
        resolve_default_target_uid(0, Some("1000"), None).unwrap(),
        1000
    );
}

#[test]
fn test_default_target_under_pkexec_uses_pkexec_uid() {
    assert_eq!(
        resolve_default_target_uid(0, None, Some("1001")).unwrap(),
        1001
    );
}

#[test]
fn test_default_target_prefers_sudo_uid_over_pkexec_uid() {
    assert_eq!(
        resolve_default_target_uid(0, Some("1002"), Some("1003")).unwrap(),
        1002
    );
}

#[test]
fn test_default_target_never_silently_root() {
    match resolve_default_target_uid(0, None, None) {
        Err(EnrollmentCliError::TargetUserRequired) => {}
        other => panic!("Expected TargetUserRequired, got {other:?}"),
    }
}

#[test]
fn test_default_target_sudo_uid_zero_is_not_an_implicit_root_target() {
    match resolve_default_target_uid(0, Some("0"), None) {
        Err(EnrollmentCliError::TargetUserRequired) => {}
        other => panic!("Expected TargetUserRequired, got {other:?}"),
    }
    match resolve_default_target_uid(0, None, Some("0")) {
        Err(EnrollmentCliError::TargetUserRequired) => {}
        other => panic!("Expected TargetUserRequired, got {other:?}"),
    }
}

#[test]
fn test_default_target_malformed_sudo_uid_fails_closed() {
    for bad in ["", "abc", "-1", "1000x", "4294967296", "99999999999"] {
        match resolve_default_target_uid(0, Some(bad), None) {
            Err(EnrollmentCliError::InvalidInvokerUid(_)) => {}
            other => panic!("Expected InvalidInvokerUid for {bad:?}, got {other:?}"),
        }
    }
}

#[test]
fn test_default_target_unprivileged_caller_is_real_uid() {
    // Environment variables are only consulted when the real UID is root.
    assert_eq!(resolve_default_target_uid(1000, None, None).unwrap(), 1000);
    assert_eq!(
        resolve_default_target_uid(1000, Some("2000"), None).unwrap(),
        1000
    );
}

#[test]
fn test_explicit_root_target_is_allowed() {
    assert_eq!(resolve_target_uid(Some(0), None).unwrap(), 0);
}
