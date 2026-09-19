//! Contractual integration tests for Issue #28:
//! Policy engine lock contention, monotonic clock fallback, and logind session validation.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use std::fs;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::RwLock;

use nix::time::ClockId;
use soos_daemon::config::DispatcherConfig;
use soos_daemon::dispatcher::ConnectionDispatcher;
use soos_daemon::error::DaemonError;
use soos_daemon::health::HealthState;
use soos_daemon::pipeline::{current_monotonic_nanos, current_monotonic_nanos_from_clock};
use soos_daemon::session::SessionValidator;
use soos_policy::{AuthorizationEngine, RateLimitConfig, RateLimiter, ThresholdConfig};
use soos_protocol::codec::{decode, encode};
use soos_protocol::types::{ReasonClass, Request, RequestKind, Response, Verdict, CURRENT_VERSION};

fn make_auth_request(uid: u32) -> Request {
    Request {
        version: CURRENT_VERSION,
        kind: RequestKind::Auth,
        request_id: [42u8; 32],
        uid_hint: uid,
        service: "sudo".to_string(),
        deadline_monotonic_ns: u64::MAX,
    }
}

async fn read_response(client: &mut UnixStream) -> Response {
    let mut resp_len_bytes = [0u8; 4];
    client
        .read_exact(&mut resp_len_bytes)
        .await
        .expect("Read response length prefix failed");
    let resp_len = u32::from_be_bytes(resp_len_bytes) as usize;
    assert!(resp_len <= 4096);

    let mut full_resp_buf = Vec::with_capacity(4 + resp_len);
    full_resp_buf.extend_from_slice(&resp_len_bytes);
    let mut resp_payload = vec![0u8; resp_len];
    client
        .read_exact(&mut resp_payload)
        .await
        .expect("Read response payload failed");
    full_resp_buf.extend_from_slice(&resp_payload);

    decode(&full_resp_buf).expect("Decode response failed")
}

// ---------------------------------------------------------------------------
// Sub-issue #28.1: Policy engine lock contention (RwLock and check_allowed)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_concurrent_auth_requests_no_lock_starvation() {
    let threshold_cfg = ThresholdConfig::default();
    let rate_cfg = RateLimitConfig::new(10, 60_000_000_000);
    let rate_limiter = RateLimiter::new(rate_cfg);
    let policy = Arc::new(RwLock::new(AuthorizationEngine::with_rate_limiter(
        threshold_cfg,
        rate_limiter,
    )));

    // Assert that 8 concurrent tasks can acquire read locks and call check_allowed simultaneously
    // without lock starvation.
    let now_ns = current_monotonic_nanos().expect("Clock read must succeed");
    let mut handles = Vec::new();

    for i in 0..8 {
        let policy_clone = policy.clone();
        let uid = 1000 + (i % 2);
        handles.push(tokio::spawn(async move {
            let engine = policy_clone.read().await;
            engine.check_allowed(uid, now_ns)
        }));
    }

    for handle in handles {
        let res = handle.await.expect("Task failed");
        assert!(
            res.is_ok(),
            "check_allowed should succeed for concurrent readers"
        );
    }
}

// ---------------------------------------------------------------------------
// Sub-issue #28.2: Monotonic clock failure fallback
// ---------------------------------------------------------------------------

#[test]
fn test_monotonic_clock_invalid_clock_id_returns_error() {
    // Calling clock_gettime with an invalid clock ID must return Err(DaemonError::Clock)
    let invalid_id = ClockId::from_raw(999_999);
    let res = current_monotonic_nanos_from_clock(invalid_id);
    assert!(res.is_err(), "Invalid clock ID must return Err");
    match res {
        Err(DaemonError::Clock(msg)) => {
            assert!(!msg.is_empty());
        }
        other => panic!("Expected DaemonError::Clock, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_monotonic_clock_failure_returns_unavailable() {
    let dir = tempdir().expect("Failed to create tempdir");
    let sock_path = dir.path().join("clock_fail.sock");
    let listener = UnixListener::bind(&sock_path).expect("Bind failed");

    let health = Arc::new(HealthState::new());
    health.set_socket_ready(true);

    let config = DispatcherConfig {
        max_concurrent_connections: 4,
        connection_timeout: Duration::from_millis(500),
        enforce_active_session: false,
        logind_sessions_dir: dir.path().to_path_buf(),
    };

    // Construct dispatcher with simulated failing clock function
    fn failing_clock() -> Result<u64, DaemonError> {
        Err(DaemonError::Clock(
            "Simulated monotonic clock failure".to_string(),
        ))
    }

    let dispatcher =
        Arc::new(ConnectionDispatcher::new(config, health).with_clock_fn(failing_clock));

    let disp_clone = dispatcher.clone();
    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let _ = disp_clone.handle_connection(stream).await;
        }
    });

    let mut client = UnixStream::connect(&sock_path)
        .await
        .expect("Connect failed");
    let current_uid = nix::unistd::getuid().as_raw();
    let req = make_auth_request(current_uid);
    let framed = encode(&req).expect("Encode failed");
    client.write_all(&framed).await.expect("Write failed");
    client.flush().await.expect("Flush failed");

    let resp = read_response(&mut client).await;
    assert_eq!(resp.request_id, req.request_id);
    assert_eq!(
        resp.verdict,
        Verdict::Unavailable,
        "Clock failure must return Unavailable verdict (fail-closed)"
    );
    assert_eq!(
        resp.reason_class,
        ReasonClass::InternalError,
        "Clock failure reason class must be InternalError"
    );
}

// ---------------------------------------------------------------------------
// Sub-issue #28.3: logind session validation
// ---------------------------------------------------------------------------

#[test]
fn test_session_validator_parses_active_session() {
    let dir = tempdir().expect("Failed to create tempdir");
    let sessions_dir = dir.path();

    // Create an inactive session for UID 1001
    let session_inactive = sessions_dir.join("1");
    fs::write(
        &session_inactive,
        "UID=1001\nUSER=testuser\nACTIVE=0\nSTATE=closing\n",
    )
    .expect("Write inactive session failed");

    // Create an active session for UID 1002
    let session_active = sessions_dir.join("2");
    fs::write(
        &session_active,
        "UID=1002\nUSER=alice\nACTIVE=1\nSTATE=active\nTYPE=wayland\n",
    )
    .expect("Write active session failed");

    // Create a fifo / ref file that should be ignored
    let ref_file = sessions_dir.join("2.ref");
    fs::write(&ref_file, "").expect("Write ref file failed");

    let validator = SessionValidator::with_sessions_dir(sessions_dir.to_path_buf());

    // UID 1001 has only an inactive session
    assert!(!validator.is_active_session(1001));

    // UID 1002 has an active session
    assert!(validator.is_active_session(1002));

    // UID 9999 has no session files
    assert!(!validator.is_active_session(9999));
}

#[tokio::test]
async fn test_auth_rejected_for_uid_without_active_session() {
    let dir = tempdir().expect("Failed to create tempdir");
    let sock_path = dir.path().join("session_test.sock");
    let listener = UnixListener::bind(&sock_path).expect("Bind failed");

    let sessions_dir = dir.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("Failed to create sessions dir");

    let health = Arc::new(HealthState::new());
    health.set_socket_ready(true);

    let config = DispatcherConfig {
        max_concurrent_connections: 4,
        connection_timeout: Duration::from_millis(500),
        enforce_active_session: true,
        logind_sessions_dir: sessions_dir.clone(),
    };

    let dispatcher = Arc::new(ConnectionDispatcher::new(config, health));

    // 1. First test: Connecting UID has no active session in sessions_dir -> Rejected with ProtocolError / UidMismatch
    let disp_clone = dispatcher.clone();
    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let _ = disp_clone.handle_connection(stream).await;
        }
    });

    let mut client = UnixStream::connect(&sock_path)
        .await
        .expect("Connect failed");
    let current_uid = nix::unistd::getuid().as_raw();
    let req = make_auth_request(current_uid);
    let framed = encode(&req).expect("Encode failed");
    client.write_all(&framed).await.expect("Write failed");
    client.flush().await.expect("Flush failed");

    let resp = read_response(&mut client).await;
    assert_eq!(
        resp.verdict,
        Verdict::ProtocolError,
        "Auth request for UID without active session must be rejected with ProtocolError"
    );
    assert_eq!(
        resp.reason_class,
        ReasonClass::UidMismatch,
        "Auth request for UID without active session must have UidMismatch reason class"
    );

    // 2. Now provision an active session file for current_uid
    let session_file = sessions_dir.join("10");
    let session_content = format!(
        "UID={}\nUSER=test\nACTIVE=1\nSTATE=active\nTYPE=wayland\n",
        current_uid
    );
    fs::write(&session_file, session_content).expect("Write session file failed");

    // Connect again and verify that session check passes and request proceeds to pipeline
    let listener_active = UnixListener::bind(dir.path().join("active.sock")).expect("Bind failed");
    let disp_clone2 = dispatcher.clone();
    tokio::spawn(async move {
        if let Ok((stream, _)) = listener_active.accept().await {
            let _ = disp_clone2.handle_connection(stream).await;
        }
    });

    let mut client2 = UnixStream::connect(dir.path().join("active.sock"))
        .await
        .expect("Connect failed");
    client2.write_all(&framed).await.expect("Write failed");
    client2.flush().await.expect("Flush failed");

    let resp2 = read_response(&mut client2).await;
    // With active session, it passes step 6b and reaches pipeline check (which returns Unavailable because pipeline is None, NOT ProtocolError/UidMismatch)
    assert_ne!(
        resp2.verdict,
        Verdict::ProtocolError,
        "Auth request with active session must not be rejected at session check"
    );
    assert_eq!(
        resp2.verdict,
        Verdict::Unavailable,
        "Request proceeds to uninitialized pipeline, returning Unavailable"
    );
}
