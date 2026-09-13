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

#[test]
fn test_parse_uid_override() {
    let arg1 = CString::new("uid=1001").unwrap();
    let args = [arg1.as_ptr().cast::<u8>()];
    // SAFETY: args contains 1 valid C string pointer
    let config = unsafe { parse_argv(1, args.as_ptr()) };
    assert_eq!(config.uid, Some(1001));
}

#[test]
fn test_parse_whitespace_padded_arguments() {
    let arg1 = CString::new("  timeout_ms=150  ").unwrap();
    let arg2 = CString::new("  event=password-failed  ").unwrap();
    let arg3 = CString::new("  socket_path=/tmp/test.sock  ").unwrap();
    let arg4 = CString::new("  service=custom_svc  ").unwrap();
    let args = [
        arg1.as_ptr().cast::<u8>(),
        arg2.as_ptr().cast::<u8>(),
        arg3.as_ptr().cast::<u8>(),
        arg4.as_ptr().cast::<u8>(),
    ];

    // SAFETY: args contains 4 valid C string pointers with padding
    let config = unsafe { parse_argv(4, args.as_ptr()) };
    assert_eq!(config.timeout_ms, 150);
    assert_eq!(config.event, Some(PamEvent::PasswordFailed));
    assert_eq!(config.socket_path, PathBuf::from("/tmp/test.sock"));
    assert_eq!(config.service, "custom_svc");
}

#[test]
fn test_parse_zero_timeout_clamps_to_min() {
    let arg = CString::new("timeout_ms=0").unwrap();
    let args = [arg.as_ptr().cast::<u8>()];
    // SAFETY: args contains 1 valid pointer
    let config = unsafe { parse_argv(1, args.as_ptr()) };
    assert_eq!(config.timeout_ms, 10);
}

#[test]
fn test_parse_empty_socket_and_service_preserves_default() {
    let arg1 = CString::new("socket_path=").unwrap();
    let arg2 = CString::new("service=").unwrap();
    let args = [arg1.as_ptr().cast::<u8>(), arg2.as_ptr().cast::<u8>()];

    // SAFETY: args contains 2 valid pointers
    let config = unsafe { parse_argv(2, args.as_ptr()) };
    assert_eq!(config.socket_path, PathBuf::from("/run/soos/daemon.sock"));
    assert_eq!(config.service, "pam_soos");
}

#[test]
fn test_parse_argc_exceeding_max_argc_is_bounded() {
    let arg = CString::new("timeout_ms=100").unwrap();
    let mut args: Vec<*const u8> = (0..128).map(|_| arg.as_ptr().cast::<u8>()).collect();

    // SAFETY: args has 128 valid pointers, argc passed is 128 (> MAX_ARGC 64)
    let config = unsafe { parse_argv(128, args.as_mut_ptr()) };
    assert_eq!(config.timeout_ms, 100);
}

#[test]
fn test_parse_unterminated_string_exceeding_max_arg_len() {
    // 300 non-null bytes without terminating null
    let long_bytes = vec![b'a'; 300];
    let args = [long_bytes.as_ptr()];

    // SAFETY: pointer has 300 non-null bytes; extract_bounded_str must safely stop at MAX_ARG_LEN without out-of-bounds read
    let config = unsafe { parse_argv(1, args.as_ptr()) };
    assert_eq!(config.timeout_ms, 250); // Kept default
}
