//! PAM IPC client hardening of the 2026-10-02 review (GitHub #311; matrix rows PUR9-PUR11).
//!
//! - PAM-NEW-6: the request nonce comes from `getrandom(2)` with `GRND_NONBLOCK`; an
//!   uninitialized kernel CRNG (`EAGAIN`) is `IpcError::Random` (so `PAM_IGNORE`) instead
//!   of blocking the PAM host past its deadline. `EINTR` is retried a bounded number of
//!   times.
//! - PAM-NEW-7: the security-relevant rejections (`RequestIdMismatch`, `StaleResponse`,
//!   `UnsupportedVersion`) produce a value-free syslog line; other errors stay unlogged.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use std::io;

use pam_soos::ipc::{fill_random_nonblocking, fill_random_with, IpcError, MAX_RANDOM_ATTEMPTS};
use soos_protocol::types::ResponseFreshnessError;

/// PUR9: `EAGAIN` (CRNG not yet initialized) fails at once with `IpcError::Random`.
#[test]
fn test_pur_random_eagain_is_random_error_without_retry() {
    let mut calls = 0usize;
    let mut buf = [0u8; 32];
    let result = fill_random_with(&mut buf, |_| {
        calls += 1;
        Err(io::Error::from_raw_os_error(libc::EAGAIN))
    });
    assert!(
        matches!(result, Err(IpcError::Random(ref e)) if e.raw_os_error() == Some(libc::EAGAIN)),
        "got {result:?}"
    );
    assert_eq!(
        calls, 1,
        "EAGAIN must not be retried (it would spin until the deadline)"
    );
}

/// PUR9: `EINTR` and short reads are retried, but at most `MAX_RANDOM_ATTEMPTS` times.
#[test]
fn test_pur_random_eintr_and_short_reads_are_bounded() {
    let mut buf = [0u8; 32];
    let mut step = 0usize;
    let result = fill_random_with(&mut buf, |chunk| {
        step += 1;
        match step {
            1 => Err(io::Error::from_raw_os_error(libc::EINTR)),
            2 => {
                chunk[..16].fill(0xAB);
                Ok(16)
            }
            _ => {
                chunk.fill(0xCD);
                Ok(chunk.len())
            }
        }
    });
    assert!(result.is_ok(), "got {result:?}");
    assert!(buf[..16].iter().all(|b| *b == 0xAB));
    assert!(buf[16..].iter().all(|b| *b == 0xCD));

    let mut calls = 0usize;
    let result = fill_random_with(&mut buf, |_| {
        calls += 1;
        Err(io::Error::from_raw_os_error(libc::EINTR))
    });
    assert!(matches!(result, Err(IpcError::Random(_))), "got {result:?}");
    assert_eq!(calls, MAX_RANDOM_ATTEMPTS);

    let result = fill_random_with(&mut buf, |_| Ok(0));
    assert!(
        matches!(result, Err(IpcError::Random(_))),
        "a zero-byte read must fail, got {result:?}"
    );
}

/// PUR9: the production source fills a 256-bit nonce.
#[test]
fn test_pur_random_nonblocking_fills_the_nonce() {
    let mut a = [0u8; 32];
    let mut b = [0u8; 32];
    fill_random_nonblocking(&mut a).expect("initialized CRNG on a booted host");
    fill_random_nonblocking(&mut b).expect("initialized CRNG on a booted host");
    assert_ne!(a, [0u8; 32]);
    assert_ne!(a, b);
}

/// PUR10: the three security-relevant rejections are logged, value-free.
#[test]
fn test_pur_security_rejections_have_value_free_log_lines() {
    let cases = [
        IpcError::RequestIdMismatch,
        IpcError::UnsupportedVersion { version: 77 },
        IpcError::StaleResponse(ResponseFreshnessError::Expired),
        IpcError::StaleResponse(ResponseFreshnessError::FutureDated),
    ];
    for err in &cases {
        let line = err
            .security_log_message()
            .unwrap_or_else(|| panic!("{err:?} must be logged"));
        assert!(
            !line.chars().any(|c| c.is_ascii_digit()),
            "{err:?}: the log line must carry no value, got {line:?}"
        );
        assert!(line.starts_with("daemon response rejected"), "{line:?}");
    }
}

/// PUR10: ordinary unavailability (daemon absent, timeout) is not a security event.
#[test]
fn test_pur_ordinary_errors_are_not_security_log_lines() {
    for err in [
        IpcError::Timeout,
        IpcError::EmptyResponse,
        IpcError::Connect(io::Error::from_raw_os_error(libc::ENOENT)),
    ] {
        assert_eq!(err.security_log_message(), None, "{err:?}");
    }
}
