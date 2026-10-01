//! PAM client enforcement of the daemon `Response` expiry (GitHub #287 owner decision,
//! matrix rows PRE3-PRE5).
//!
//! The daemon stamps every response from CLOCK_MONOTONIC (`issued > 0`,
//! `expires = issued + 2 s`). `pam_soos.so` reads the same clock after the response
//! arrived and treats an expired, unstamped, inverted or future-dated response as
//! not-Allow: the module returns `PAM_IGNORE` (password fallback), never `PAM_SUCCESS`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

#[path = "common/stamps.rs"]
mod stamps;

use std::ffi::CString;
use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::ptr;
use std::thread;

use pam_soos::config::PamConfig;
use pam_soos::ipc::{authenticate, IpcError, MAX_RESPONSE_FUTURE_SKEW_NS};
use pam_soos::pam_sm_authenticate;
use soos_protocol::codec::{decode, encode};
use soos_protocol::types::{
    ReasonClass, Request, Response, ResponseFreshnessError, Verdict, CURRENT_VERSION,
};
use stamps::{monotonic_now_ns, FIXTURE_VALIDITY_NS};
use tempfile::tempdir;

const PAM_SUCCESS: i32 = 0;
const PAM_IGNORE: i32 = 25;

/// How the one-shot mock daemon stamps its `Allow` response, relative to the
/// CLOCK_MONOTONIC instant it builds the response at.
#[derive(Clone, Copy, Debug)]
enum Stamps {
    /// `issued = now`, `expires = now + 2 s` (what `soos-daemon` sends).
    Fresh,
    /// `issued = now - 3 s`, `expires = now - 1 s`.
    Expired,
    /// `issued = expires = 0` (the legacy fixture and old `mock_daemon.py` value).
    Zero,
    /// `issued = now`, `expires = now - 1` (window closes before it opens).
    Inverted,
    /// `issued = now + offset`, `expires = issued + 2 s`.
    FutureBy(u64),
}

fn stamps_for(kind: Stamps) -> (u64, u64) {
    let now = monotonic_now_ns();
    match kind {
        Stamps::Fresh => (now, now + FIXTURE_VALIDITY_NS),
        Stamps::Expired => (now - 3_000_000_000, now - 1_000_000_000),
        Stamps::Zero => (0, 0),
        Stamps::Inverted => (now, now - 1),
        Stamps::FutureBy(offset) => (now + offset, now + offset + FIXTURE_VALIDITY_NS),
    }
}

/// One-shot mock daemon answering `Allow` bound to the request nonce with `kind` stamps.
fn spawn_allow_daemon(listener: UnixListener, kind: Stamps) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let mut len_buf = [0u8; 4];
        if stream.read_exact(&mut len_buf).is_err() {
            return;
        }
        let size = u32::from_be_bytes(len_buf) as usize;
        let mut frame = len_buf.to_vec();
        let mut body = vec![0u8; size];
        if stream.read_exact(&mut body).is_err() {
            return;
        }
        frame.extend_from_slice(&body);
        let req: Request = decode(&frame).expect("tagged request decodes strictly");
        let (issued, expires) = stamps_for(kind);
        let resp = Response {
            version: CURRENT_VERSION,
            request_id: req.request_id,
            verdict: Verdict::Allow,
            reason_class: ReasonClass::FaceMatch,
            issued_monotonic_ns: issued,
            expires_monotonic_ns: expires,
        };
        let _ = stream.write_all(&encode(&resp).expect("encode"));
    })
}

/// Runs `pam_sm_authenticate` (null handle, detached flow) against a mock daemon.
fn pam_code_with(kind: Stamps) -> i32 {
    let tmp = tempdir().expect("tempdir");
    let sock = tmp.path().join("expiry.sock");
    let listener = UnixListener::bind(&sock).expect("bind");
    let daemon = spawn_allow_daemon(listener, kind);
    let args: Vec<CString> = [
        format!("socket_path={}", sock.display()),
        "timeout_ms=1000".to_string(),
        "uid=1000".to_string(),
    ]
    .into_iter()
    .map(|s| CString::new(s).expect("cstring"))
    .collect();
    let ptrs: Vec<*const u8> = args.iter().map(|c| c.as_ptr().cast::<u8>()).collect();
    let code = pam_sm_authenticate(ptr::null_mut(), 0, ptrs.len() as i32, ptrs.as_ptr());
    daemon.join().expect("daemon thread");
    code
}

/// Runs the IPC exchange directly and returns its result.
fn ipc_result_with(kind: Stamps) -> Result<(Verdict, ReasonClass), IpcError> {
    let tmp = tempdir().expect("tempdir");
    let sock = tmp.path().join("expiry_ipc.sock");
    let listener = UnixListener::bind(&sock).expect("bind");
    let daemon = spawn_allow_daemon(listener, kind);
    let result = authenticate(&config_for(&sock), 1000);
    daemon.join().expect("daemon thread");
    result
}

fn config_for(sock: &Path) -> PamConfig {
    PamConfig {
        socket_path: sock.to_path_buf(),
        timeout_ms: 1000,
        uid: Some(1000),
        ..Default::default()
    }
}

fn assert_stale(
    result: Result<(Verdict, ReasonClass), IpcError>,
    expected: ResponseFreshnessError,
) {
    match result {
        Err(IpcError::StaleResponse(reason)) => assert_eq!(reason, expected),
        other => panic!("expected StaleResponse({expected:?}), got {other:?}"),
    }
}

/// PRE3 (mandatory PAM_IGNORE pathway): an `Allow` whose `expires` is already past
/// returns `PAM_IGNORE`, never `PAM_SUCCESS`.
#[test]
fn test_pre_expired_allow_response_returns_ignore() {
    assert_eq!(pam_code_with(Stamps::Expired), PAM_IGNORE);
}

/// PRE3: the IPC client reports the expired response as `StaleResponse(Expired)`.
#[test]
fn test_pre_expired_allow_response_is_rejected_by_the_ipc_client() {
    assert_stale(
        ipc_result_with(Stamps::Expired),
        ResponseFreshnessError::Expired,
    );
}

/// PRE3: an unstamped `Allow` (`issued = expires = 0`) returns `PAM_IGNORE`.
#[test]
fn test_pre_unstamped_allow_response_returns_ignore() {
    assert_eq!(pam_code_with(Stamps::Zero), PAM_IGNORE);
    assert_stale(
        ipc_result_with(Stamps::Zero),
        ResponseFreshnessError::Unstamped,
    );
}

/// PRE3: an `Allow` with `expires < issued` returns `PAM_IGNORE`.
#[test]
fn test_pre_inverted_allow_response_returns_ignore() {
    assert_eq!(pam_code_with(Stamps::Inverted), PAM_IGNORE);
    assert_stale(
        ipc_result_with(Stamps::Inverted),
        ResponseFreshnessError::Inverted,
    );
}

/// PRE4: an `Allow` issued further in the future than the skew bound returns
/// `PAM_IGNORE`.
#[test]
fn test_pre_future_dated_allow_response_returns_ignore() {
    let offset = MAX_RESPONSE_FUTURE_SKEW_NS + 1_000_000_000;
    assert_eq!(pam_code_with(Stamps::FutureBy(offset)), PAM_IGNORE);
    assert_stale(
        ipc_result_with(Stamps::FutureBy(offset)),
        ResponseFreshnessError::FutureDated,
    );
}

/// PRE4: the tolerated future skew is small and documented (10 ms, same clock on both
/// sides of the socket).
#[test]
fn test_pre_future_skew_bound_is_ten_milliseconds() {
    assert_eq!(MAX_RESPONSE_FUTURE_SKEW_NS, 10_000_000);
}

/// PRE5 (control): a response stamped like `soos-daemon` stamps it still authenticates,
/// so the rejections above are caused by the stamps alone.
#[test]
fn test_pre_fresh_allow_response_returns_success() {
    assert_eq!(pam_code_with(Stamps::Fresh), PAM_SUCCESS);
    assert!(matches!(
        ipc_result_with(Stamps::Fresh),
        Ok((Verdict::Allow, ReasonClass::FaceMatch))
    ));
}
