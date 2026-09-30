//! Response timestamp contract (GitHub #258, review finding DMN-18).
//!
//! Every `Response` the daemon renders is stamped from the dispatcher's monotonic clock:
//! `issued_monotonic_ns > 0` and `expires_monotonic_ns == issued + RESPONSE_VALIDITY_NS`
//! on every verdict path. When the clock fails the response carries `issued == expires
//! == 0` (already expired) and is never `Allow`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use soos_daemon::config::DispatcherConfig;
use soos_daemon::dispatcher::{ConnectionDispatcher, RESPONSE_VALIDITY_NS};
use soos_daemon::health::HealthState;
use soos_daemon::DaemonError;
use soos_protocol::codec::{decode, encode};
use soos_protocol::types::{ReasonClass, Request, RequestKind, Response, Verdict, CURRENT_VERSION};

const FIXED_NOW_NS: u64 = 7_000_000_000;

fn fixed_clock() -> Result<u64, DaemonError> {
    Ok(FIXED_NOW_NS)
}

fn failing_clock() -> Result<u64, DaemonError> {
    Err(DaemonError::Clock("simulated clock failure".to_string()))
}

fn config() -> DispatcherConfig {
    DispatcherConfig {
        max_concurrent_connections: 4,
        connection_timeout: Duration::from_millis(500),
        enforce_active_session: false,
        logind_sessions_dir: PathBuf::from("/run/systemd/sessions"),
    }
}

fn request(kind: RequestKind, uid: u32) -> Request {
    Request {
        version: CURRENT_VERSION,
        kind,
        request_id: [9u8; 32],
        uid_hint: uid,
        service: "sudo".to_string(),
        deadline_monotonic_ns: u64::MAX,
    }
}

/// Sends `req` through a fresh dispatcher using `clock` and returns the decoded response.
async fn roundtrip(clock: fn() -> Result<u64, DaemonError>, req: Request) -> Response {
    let dir = tempdir().expect("tempdir");
    let sock_path = dir.path().join("ts.sock");
    let listener = UnixListener::bind(&sock_path).expect("bind");
    let health = Arc::new(HealthState::new());
    health.set_socket_ready(true);
    let dispatcher = Arc::new(ConnectionDispatcher::new(config(), health).with_clock_fn(clock));
    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let _ = dispatcher.handle_connection(stream).await;
        }
    });

    let mut client = UnixStream::connect(&sock_path).await.expect("connect");
    client
        .write_all(&encode(&req).expect("encode"))
        .await
        .expect("write");
    let mut len = [0u8; 4];
    client.read_exact(&mut len).await.expect("read len");
    let body_len = u32::from_be_bytes(len) as usize;
    assert!(body_len <= 4096);
    let mut buf = len.to_vec();
    buf.resize(4 + body_len, 0);
    client.read_exact(&mut buf[4..]).await.expect("read body");
    decode(&buf).expect("decode response")
}

fn assert_stamped(resp: &Response, path: &str) {
    assert_eq!(
        resp.issued_monotonic_ns, FIXED_NOW_NS,
        "{path}: issued_monotonic_ns must come from the monotonic clock"
    );
    assert!(resp.issued_monotonic_ns > 0, "{path}: issued must be > 0");
    assert_eq!(
        resp.expires_monotonic_ns,
        FIXED_NOW_NS + RESPONSE_VALIDITY_NS,
        "{path}: expires must be issued + RESPONSE_VALIDITY_NS"
    );
    assert!(resp.issued_monotonic_ns <= resp.expires_monotonic_ns);
}

#[test]
fn test_response_validity_window_is_two_seconds() {
    assert_eq!(RESPONSE_VALIDITY_NS, 2_000_000_000);
}

#[tokio::test]
async fn test_wire_validation_rejection_is_stamped() {
    let mut req = request(RequestKind::Auth, nix::unistd::getuid().as_raw());
    req.service = "s".repeat(65);
    let resp = roundtrip(fixed_clock, req).await;
    assert_eq!(resp.verdict, Verdict::ProtocolError);
    assert_eq!(resp.reason_class, ReasonClass::MalformedRequest);
    assert_stamped(&resp, "wire validation");
}

#[tokio::test]
async fn test_uid_mismatch_rejection_is_stamped() {
    let uid = nix::unistd::getuid().as_raw();
    if uid == 0 {
        // A root peer may request any UID; the mismatch path is unreachable as root.
        return;
    }
    let resp = roundtrip(fixed_clock, request(RequestKind::Auth, uid + 1)).await;
    assert_eq!(resp.verdict, Verdict::ProtocolError);
    assert_eq!(resp.reason_class, ReasonClass::UidMismatch);
    assert_stamped(&resp, "uid mismatch");
}

#[tokio::test]
async fn test_preview_authorization_refusal_is_stamped() {
    let uid = nix::unistd::getuid().as_raw();
    if uid == 0 {
        // Root is always authorized for preview and receives a PreviewResponse instead.
        return;
    }
    let resp = roundtrip(fixed_clock, request(RequestKind::PreviewFrame, uid)).await;
    assert_eq!(resp.verdict, Verdict::ProtocolError);
    assert_stamped(&resp, "preview refusal");
}

#[tokio::test]
async fn test_no_pipeline_fallback_is_stamped() {
    let uid = nix::unistd::getuid().as_raw();
    let resp = roundtrip(fixed_clock, request(RequestKind::Auth, uid)).await;
    assert_eq!(resp.verdict, Verdict::Unavailable);
    assert_stamped(&resp, "no pipeline fallback");
}

#[tokio::test]
async fn test_clock_failure_yields_expired_non_allow_response() {
    let uid = nix::unistd::getuid().as_raw();
    let resp = roundtrip(failing_clock, request(RequestKind::Auth, uid)).await;
    assert_ne!(resp.verdict, Verdict::Allow);
    assert_eq!(resp.verdict, Verdict::Unavailable);
    assert_eq!(resp.issued_monotonic_ns, 0);
    assert_eq!(
        resp.expires_monotonic_ns, 0,
        "a response rendered without a clock must be already expired"
    );
}
