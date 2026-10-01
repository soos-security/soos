//! `soos-gui` applies the PAM response staleness guard to daemon `Response` frames
//! (GitHub #289, rows DGP2-DGP3).
//!
//! The GUI never asks the daemon for an authentication verdict; the only `Response` it
//! consumes is the refusal of a `PreviewFrame` request. Like `pam_soos.so` and
//! `soos-admin test-pam`, it checks `Response::check_freshness` with the shared
//! `MAX_RESPONSE_FUTURE_SKEW_NS`: a stale or unstamped `Response` echoing the nonce is a
//! protocol error, never a usable verdict and never an authorized preview.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

#[path = "common/stamps.rs"]
mod stamps;

use soos_gui::{IpcCameraManager, IpcPreviewError};
use soos_protocol::codec::{decode, encode};
use soos_protocol::types::{
    ReasonClass, Request, Response, Verdict, CURRENT_VERSION, MAX_RESPONSE_FUTURE_SKEW_NS,
};
use stamps::{monotonic_now_ns, FIXTURE_VALIDITY_NS};
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};

fn read_request(stream: &mut UnixStream) -> Option<Request> {
    let mut len_bytes = [0u8; 4];
    stream.read_exact(&mut len_bytes).ok()?;
    let len = u32::from_be_bytes(len_bytes) as usize;
    let mut buf = vec![0u8; 4 + len];
    buf[..4].copy_from_slice(&len_bytes);
    stream.read_exact(&mut buf[4..]).ok()?;
    decode::<Request>(&buf).ok()
}

/// Probes a one-shot fake daemon answering `verdict` / `reason` bound to the request nonce,
/// with the stamps returned by `stamps_at(now)` (`now` = CLOCK_MONOTONIC at build time).
fn probe_with(
    verdict: Verdict,
    reason_class: ReasonClass,
    stamps_at: fn(u64) -> (u64, u64),
) -> Result<(), IpcPreviewError> {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("daemon.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let req = read_request(&mut stream).expect("probe request");
        let (issued, expires) = stamps_at(monotonic_now_ns());
        let resp = Response {
            version: CURRENT_VERSION,
            request_id: req.request_id,
            verdict,
            reason_class,
            issued_monotonic_ns: issued,
            expires_monotonic_ns: expires,
        };
        stream.write_all(&encode(&resp).unwrap()).unwrap();
        stream.flush().unwrap();
    });
    let result = IpcCameraManager::probe_preview(&sock);
    server.join().unwrap();
    result
}

fn expired(now: u64) -> (u64, u64) {
    (now - 3_000_000_000, now - 1_000_000_000)
}

fn fresh(now: u64) -> (u64, u64) {
    (now, now + FIXTURE_VALIDITY_NS)
}

/// DGP2: an expired refusal is not trusted as the daemon's answer.
#[test]
fn test_dgp_gui_expired_refusal_is_a_protocol_error() {
    assert_eq!(
        probe_with(Verdict::ProtocolError, ReasonClass::UidMismatch, expired),
        Err(IpcPreviewError::Protocol)
    );
    assert_eq!(
        probe_with(
            Verdict::Unavailable,
            ReasonClass::CameraUnavailable,
            expired
        ),
        Err(IpcPreviewError::Protocol)
    );
}

/// DGP2: unstamped and future-dated refusals are rejected with the same shared rule.
#[test]
fn test_dgp_gui_unstamped_or_future_refusal_is_a_protocol_error() {
    assert_eq!(
        probe_with(Verdict::ProtocolError, ReasonClass::RateLimited, |_| (0, 0)),
        Err(IpcPreviewError::Protocol)
    );
    assert_eq!(
        probe_with(Verdict::ProtocolError, ReasonClass::UidMismatch, |now| {
            let issued = now + MAX_RESPONSE_FUTURE_SKEW_NS + 1_000_000_000;
            (issued, issued + FIXTURE_VALIDITY_NS)
        }),
        Err(IpcPreviewError::Protocol)
    );
}

/// DGP3: a stale `Allow` is never shown as success (a fresh one is a protocol error too: a
/// preview request is never answered with an authorization verdict).
#[test]
fn test_dgp_gui_stale_allow_is_never_success() {
    assert_eq!(
        probe_with(Verdict::Allow, ReasonClass::FaceMatch, expired),
        Err(IpcPreviewError::Protocol)
    );
    assert_eq!(
        probe_with(Verdict::Allow, ReasonClass::FaceMatch, fresh),
        Err(IpcPreviewError::Protocol)
    );
}

/// DGP2 (control): fresh refusals keep their specific meaning.
#[test]
fn test_dgp_gui_fresh_refusals_keep_their_meaning() {
    assert_eq!(
        probe_with(Verdict::ProtocolError, ReasonClass::UidMismatch, fresh),
        Err(IpcPreviewError::Unauthorized)
    );
    assert_eq!(
        probe_with(Verdict::ProtocolError, ReasonClass::RateLimited, fresh),
        Err(IpcPreviewError::RateLimited)
    );
    assert_eq!(
        probe_with(Verdict::Unavailable, ReasonClass::CameraUnavailable, fresh),
        Err(IpcPreviewError::Unavailable)
    );
}
