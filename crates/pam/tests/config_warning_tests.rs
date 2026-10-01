//! Contract tests for rejected PAM arguments (review PAM-17, GitHub #265, rows PHY3–PHY5).
//!
//! A rejected argument keeps the safe default AND produces one [`ConfigWarning`] (logged
//! at `LOG_AUTHPRIV | LOG_WARNING` by `parse_argv` / `parse_cstrs`). A warning names the
//! key only, never the rejected value.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use std::ffi::{CStr, CString};

use pam_soos::config::{
    parse_argv_with_warnings, parse_cstrs_with_warnings, ConfigWarning, DEFAULT_SOCKET_PATH,
    DEFAULT_TIMEOUT_MS, MAX_TIMEOUT_MS,
};

fn argv_of(args: &[CString]) -> Vec<*const u8> {
    args.iter().map(|a| a.as_ptr().cast::<u8>()).collect()
}

fn parse(args: &[&str]) -> (pam_soos::config::PamConfig, Vec<ConfigWarning>) {
    let owned: Vec<CString> = args.iter().map(|a| CString::new(*a).unwrap()).collect();
    let argv = argv_of(&owned);
    let argc = i32::try_from(argv.len()).unwrap();
    // SAFETY: argv holds argc valid NUL-terminated strings kept alive by `owned`.
    unsafe { parse_argv_with_warnings(argc, argv.as_ptr()) }
}

#[test]
fn test_valid_arguments_produce_no_warning() {
    let (config, warnings) = parse(&[
        "timeout_ms=250",
        "socket=/run/soos/daemon.sock",
        "service=sudo",
        "uid=1000",
        "event=password-failed",
        "disabled",
        "disable_if_file=/etc/soos/x",
    ]);
    assert_eq!(config.timeout_ms, 250);
    assert!(warnings.is_empty(), "got {warnings:?}");
}

#[test]
fn test_unparsable_timeout_keeps_default_and_warns() {
    let (config, warnings) = parse(&["timeout_ms=abc"]);
    assert_eq!(config.timeout_ms, DEFAULT_TIMEOUT_MS);
    assert_eq!(
        warnings,
        vec![ConfigWarning::InvalidValue { key: "timeout_ms" }]
    );
}

#[test]
fn test_out_of_range_timeout_is_clamped_and_warns() {
    let (config, warnings) = parse(&["timeout_ms=60000"]);
    assert_eq!(config.timeout_ms, MAX_TIMEOUT_MS);
    assert_eq!(
        warnings,
        vec![ConfigWarning::TimeoutClamped {
            requested: 60000,
            applied: MAX_TIMEOUT_MS
        }]
    );
}

#[test]
fn test_unparsable_uid_keeps_none_and_warns() {
    let (config, warnings) = parse(&["uid=-1"]);
    assert_eq!(config.uid, None);
    assert_eq!(warnings, vec![ConfigWarning::InvalidValue { key: "uid" }]);
}

#[test]
fn test_empty_values_warn() {
    let (config, warnings) = parse(&["socket_path=", "service= ", "disable_if_file="]);
    assert_eq!(config.socket_path.to_str(), Some(DEFAULT_SOCKET_PATH));
    assert_eq!(
        warnings,
        vec![
            ConfigWarning::EmptyValue { key: "socket_path" },
            ConfigWarning::EmptyValue { key: "service" },
            ConfigWarning::EmptyValue {
                key: "disable_if_file"
            },
        ]
    );
}

#[test]
fn test_unknown_event_value_warns() {
    let (config, warnings) = parse(&["event=password-ok"]);
    assert_eq!(config.event, None);
    assert_eq!(warnings, vec![ConfigWarning::InvalidValue { key: "event" }]);
}

#[test]
fn test_argument_of_max_len_or_more_is_dropped_and_warns() {
    let long_socket = format!("socket_path=/{}", "s".repeat(300));
    let (config, warnings) = parse(&["timeout_ms=300", &long_socket]);
    assert_eq!(config.timeout_ms, 300);
    assert_eq!(config.socket_path.to_str(), Some(DEFAULT_SOCKET_PATH));
    assert_eq!(warnings, vec![ConfigWarning::ArgumentTooLong { index: 1 }]);
}

#[test]
fn test_null_argument_pointer_warns() {
    let first = CString::new("uid=42").unwrap();
    let argv = [first.as_ptr().cast::<u8>(), std::ptr::null()];
    // SAFETY: argv holds two pointers, the second null.
    let (config, warnings) = unsafe { parse_argv_with_warnings(2, argv.as_ptr()) };
    assert_eq!(config.uid, Some(42));
    assert_eq!(warnings, vec![ConfigWarning::NullArgument { index: 1 }]);
}

#[test]
fn test_too_many_arguments_warns_once() {
    let arg = CString::new("timeout_ms=100").unwrap();
    let argv: Vec<*const u8> = (0..70).map(|_| arg.as_ptr().cast::<u8>()).collect();
    // SAFETY: argv holds 70 valid pointers.
    let (config, warnings) = unsafe { parse_argv_with_warnings(70, argv.as_ptr()) };
    assert_eq!(config.timeout_ms, 100);
    assert_eq!(
        warnings,
        vec![ConfigWarning::TooManyArguments { ignored: 6 }]
    );
}

#[test]
fn test_unknown_argument_reports_key_but_never_value() {
    let (_, warnings) = parse(&["sockt=/secret/value", "garbage"]);
    assert_eq!(
        warnings,
        vec![
            ConfigWarning::UnknownArgument {
                key: Some("sockt".to_owned())
            },
            ConfigWarning::UnknownArgument {
                key: Some("garbage".to_owned())
            },
        ]
    );
    for w in &warnings {
        let text = w.to_string();
        assert!(!text.contains("/secret/value"), "value leaked: {text}");
    }
}

#[test]
fn test_unknown_argument_with_unprintable_key_is_not_echoed() {
    let weird = format!("{}=x", "k".repeat(40));
    let (_, warnings) = parse(&["pa ss=hunter2", &weird]);
    assert_eq!(
        warnings,
        vec![
            ConfigWarning::UnknownArgument { key: None },
            ConfigWarning::UnknownArgument { key: None },
        ]
    );
    for w in &warnings {
        let text = w.to_string();
        assert!(
            !text.contains("hunter2") && !text.contains("pa ss"),
            "{text}"
        );
    }
}

#[test]
fn test_non_utf8_argument_warns_through_cstr_parser() {
    let bad: &CStr = c"service=\xff\xfe";
    let (config, warnings) = parse_cstrs_with_warnings(vec![bad, c"uid=7"]);
    assert_eq!(config.uid, Some(7));
    assert_eq!(config.service, "pam_soos");
    assert_eq!(warnings, vec![ConfigWarning::NotUtf8 { index: 0 }]);
}

#[test]
fn test_cstr_parser_enforces_the_same_length_bound() {
    let long = CString::new(format!("socket=/{}", "s".repeat(300))).unwrap();
    let (config, warnings) = parse_cstrs_with_warnings(vec![long.as_c_str()]);
    assert_eq!(config.socket_path.to_str(), Some(DEFAULT_SOCKET_PATH));
    assert_eq!(warnings, vec![ConfigWarning::ArgumentTooLong { index: 0 }]);
}

#[test]
fn test_long_service_is_truncated_and_warns() {
    let name = "s".repeat(soos_protocol::MAX_SERVICE_LEN + 10);
    let (config, warnings) = parse(&[&format!("service={name}")]);
    assert_eq!(config.service, "s".repeat(soos_protocol::MAX_SERVICE_LEN));
    assert!(config.service_from_args);
    assert_eq!(warnings, vec![ConfigWarning::ServiceTruncated]);
}

#[test]
fn test_every_warning_display_is_single_line_and_value_free() {
    let all = [
        ConfigWarning::ArgumentTooLong { index: 3 },
        ConfigWarning::NullArgument { index: 1 },
        ConfigWarning::TooManyArguments { ignored: 2 },
        ConfigWarning::NotUtf8 { index: 0 },
        ConfigWarning::InvalidValue { key: "timeout_ms" },
        ConfigWarning::EmptyValue { key: "service" },
        ConfigWarning::TimeoutClamped {
            requested: 1,
            applied: 10,
        },
        ConfigWarning::ServiceTruncated,
        ConfigWarning::UnknownArgument { key: None },
        ConfigWarning::UnknownArgument {
            key: Some("foo".to_owned()),
        },
    ];
    for w in &all {
        let text = w.to_string();
        assert!(!text.is_empty() && !text.contains('\n'), "{text:?}");
    }
}
