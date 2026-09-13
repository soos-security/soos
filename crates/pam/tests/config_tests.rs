//! Contractual unit tests for PAM argv parsing and configuration extraction.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use pam_soos::config::{parse_argv, PamEvent};
use std::ffi::CString;
use std::path::PathBuf;

#[test]
fn test_default_config_on_null_argv() {
    // SAFETY: null pointer passed with argc 0
    let config = unsafe { parse_argv(0, std::ptr::null()) };
    assert_eq!(config.timeout_ms, 250);
    assert_eq!(config.event, None);
    assert_eq!(config.socket_path, PathBuf::from("/run/soos/daemon.sock"));
    assert_eq!(config.service, "pam_soos");
}

#[test]
fn test_negative_or_zero_argc_returns_default() {
    let ptr = std::ptr::null();
    // SAFETY: null pointer with negative or zero argc
    let config_neg = unsafe { parse_argv(-5, ptr) };
    assert_eq!(config_neg.timeout_ms, 250);
    assert_eq!(config_neg.event, None);

    // SAFETY: null pointer with zero argc
    let config_zero = unsafe { parse_argv(0, ptr) };
    assert_eq!(config_zero.timeout_ms, 250);
}

#[test]
fn test_parse_timeout_ms() {
    let arg1 = CString::new("timeout_ms=180").unwrap();
    let args = [arg1.as_ptr().cast::<u8>()];

    // SAFETY: args contains 1 valid C string pointer
    let config = unsafe { parse_argv(1, args.as_ptr()) };
    assert_eq!(config.timeout_ms, 180);
    assert_eq!(config.event, None);
}

#[test]
fn test_parse_event_password_failed() {
    let arg1 = CString::new("event=password-failed").unwrap();
    let arg2 = CString::new("timeout_ms=20").unwrap();
    let args = [arg1.as_ptr().cast::<u8>(), arg2.as_ptr().cast::<u8>()];

    // SAFETY: args contains 2 valid C string pointers
    let config = unsafe { parse_argv(2, args.as_ptr()) };
    assert_eq!(config.event, Some(PamEvent::PasswordFailed));
    assert_eq!(config.timeout_ms, 20);
}

#[test]
fn test_parse_custom_socket_and_service() {
    let arg1 = CString::new("socket_path=/tmp/custom.sock").unwrap();
    let arg2 = CString::new("service=sudo").unwrap();
    let args = [arg1.as_ptr().cast::<u8>(), arg2.as_ptr().cast::<u8>()];

    // SAFETY: args contains 2 valid C string pointers
    let config = unsafe { parse_argv(2, args.as_ptr()) };
    assert_eq!(config.socket_path, PathBuf::from("/tmp/custom.sock"));
    assert_eq!(config.service, "sudo");
}

#[test]
fn test_parse_timeout_clamping() {
    // Under minimum (10ms)
    let arg_low = CString::new("timeout_ms=5").unwrap();
    let args_low = [arg_low.as_ptr().cast::<u8>()];
    // SAFETY: args_low contains 1 valid pointer
    let config_low = unsafe { parse_argv(1, args_low.as_ptr()) };
    assert_eq!(config_low.timeout_ms, 10);

    // Over maximum (5000ms)
    let arg_high = CString::new("timeout_ms=99999").unwrap();
    let args_high = [arg_high.as_ptr().cast::<u8>()];
    // SAFETY: args_high contains 1 valid pointer
    let config_high = unsafe { parse_argv(1, args_high.as_ptr()) };
    assert_eq!(config_high.timeout_ms, 5000);
}

#[test]
fn test_safely_ignores_unknown_or_corrupted_arguments() {
    let arg1 = CString::new("invalid_arg").unwrap();
    let arg2 = CString::new("timeout_ms=not_a_number").unwrap();
    let arg3 = CString::new("something=other").unwrap();
    let args = [
        arg1.as_ptr().cast::<u8>(),
        std::ptr::null(), // Null pointer embedded inside array!
        arg2.as_ptr().cast::<u8>(),
        arg3.as_ptr().cast::<u8>(),
    ];

    // SAFETY: args array has length 4 with valid pointers and deliberate null
    let config = unsafe { parse_argv(4, args.as_ptr()) };
    assert_eq!(config.timeout_ms, 250); // Kept default
    assert_eq!(config.event, None);
}
