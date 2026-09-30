//! `service=` truncation on a UTF-8 character boundary (review PAM-17, GitHub #265, PHY4).
//!
//! A service name longer than `MAX_SERVICE_LEN` bytes whose byte `MAX_SERVICE_LEN` falls
//! inside a multibyte character used to fall back to `pam_soos`; it is now cut on the
//! previous character boundary, like the `PAM_SERVICE` item.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use std::ffi::CString;

use pam_soos::config::{parse_argv, parse_cstrs};

fn multibyte_service() -> String {
    // 63 ASCII bytes followed by a 2-byte character straddling byte 64.
    format!("{}é-suffix", "a".repeat(soos_protocol::MAX_SERVICE_LEN - 1))
}

#[test]
fn test_argv_service_cut_inside_multibyte_char_keeps_prefix() {
    let arg = CString::new(format!("service={}", multibyte_service())).unwrap();
    let argv = [arg.as_ptr().cast::<u8>()];
    // SAFETY: one valid NUL-terminated argument.
    let config = unsafe { parse_argv(1, argv.as_ptr()) };
    assert_eq!(
        config.service,
        "a".repeat(soos_protocol::MAX_SERVICE_LEN - 1)
    );
    assert!(config.service_from_args);
}

#[test]
fn test_cstr_service_cut_inside_multibyte_char_keeps_prefix() {
    let arg = CString::new(format!("service={}", multibyte_service())).unwrap();
    let config = parse_cstrs(vec![arg.as_c_str()]);
    assert_eq!(
        config.service,
        "a".repeat(soos_protocol::MAX_SERVICE_LEN - 1)
    );
    assert!(config.service_from_args);
}
