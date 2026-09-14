#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use soos_enrollment_cli::error::EnrollmentCliError;
use soos_enrollment_cli::service::check_privileges;

#[test]
fn test_root_check_bypassed_when_flag_disabled() {
    // When require_root is false, check_privileges must always succeed regardless of actual EUID
    let res = check_privileges(false);
    assert!(res.is_ok());
}

#[test]
fn test_root_check_enforced_against_euid() {
    let current_euid = nix::unistd::geteuid().as_raw();
    let res = check_privileges(true);

    if current_euid == 0 {
        assert!(res.is_ok());
    } else {
        match res {
            Err(EnrollmentCliError::RootRequired) => {}
            other => panic!("Expected Err(EnrollmentCliError::RootRequired), got: {other:?}"),
        }
    }
}

#[test]
fn test_root_required_error_message() {
    let err = EnrollmentCliError::RootRequired;
    let msg = err.to_string();
    assert!(msg.to_lowercase().contains("root"));
}
