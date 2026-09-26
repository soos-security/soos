//! Contractual integration tests for connection dispatcher.

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
use soos_daemon::dispatcher::ConnectionDispatcher;
use soos_daemon::health::HealthState;
use soos_protocol::codec::{decode, decode_preview, encode};
use soos_protocol::types::{
    PreviewResponse, ReasonClass, Request, RequestKind, Response, Verdict, CURRENT_VERSION,
};

fn test_dispatcher_config(
    max_concurrent_connections: usize,
    connection_timeout: Duration,
) -> DispatcherConfig {
    DispatcherConfig {
        max_concurrent_connections,
        connection_timeout,
        enforce_active_session: false,
        logind_sessions_dir: PathBuf::from("/run/systemd/sessions"),
    }
}

fn make_auth_request(uid: u32) -> Request {
    Request {
        version: CURRENT_VERSION,
        kind: RequestKind::Auth,
        request_id: [1u8; 32],
        uid_hint: uid,
        service: "sudo".to_string(),
        deadline_monotonic_ns: u64::MAX,
    }
}

#[tokio::test]
async fn test_dispatcher_nominal_roundtrip() {
    let dir = tempdir().expect("Failed to create tempdir");
    let sock_path = dir.path().join("dispatch.sock");

    let listener = UnixListener::bind(&sock_path).expect("Bind failed");
    let health = Arc::new(HealthState::new());
    health.set_socket_ready(true);

    let config = test_dispatcher_config(4, Duration::from_millis(500));
    let dispatcher = Arc::new(ConnectionDispatcher::new(config, health));

    // Spawn server loop accepting 1 connection
    let disp_clone = dispatcher.clone();
    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let _ = disp_clone.handle_connection(stream).await;
        }
    });

    // Client connection
    let mut client = UnixStream::connect(&sock_path)
        .await
        .expect("Connect failed");

    let current_uid = nix::unistd::getuid().as_raw();
    let req = make_auth_request(current_uid);
    let framed = encode(&req).expect("Encoding failed");

    // encode() in soos_protocol already includes 4-byte BE length prefix
    client
        .write_all(&framed)
        .await
        .expect("Write framed request failed");
    client.flush().await.expect("Flush failed");

    // Read response length prefix
    let mut resp_len_bytes = [0u8; 4];
    client
        .read_exact(&mut resp_len_bytes)
        .await
        .expect("Read resp len failed");
    let resp_len = u32::from_be_bytes(resp_len_bytes) as usize;
    assert!(resp_len <= 4096);

    let mut full_resp_buf = Vec::with_capacity(4 + resp_len);
    full_resp_buf.extend_from_slice(&resp_len_bytes);
    let mut resp_payload = vec![0u8; resp_len];
    client
        .read_exact(&mut resp_payload)
        .await
        .expect("Read resp payload failed");
    full_resp_buf.extend_from_slice(&resp_payload);

    let resp: Response = decode(&full_resp_buf).expect("Decode response failed");
    assert_eq!(resp.request_id, [1u8; 32]);
    assert!(matches!(
        resp.verdict,
        Verdict::Allow | Verdict::Deny | Verdict::Unavailable
    ));
}

#[tokio::test]
async fn test_dispatcher_rejects_spoofed_uid() {
    let dir = tempdir().expect("Failed to create tempdir");
    let sock_path = dir.path().join("dispatch_spoof.sock");

    let listener = UnixListener::bind(&sock_path).expect("Bind failed");
    let health = Arc::new(HealthState::new());
    health.set_socket_ready(true);

    let config = test_dispatcher_config(4, Duration::from_millis(500));
    let dispatcher = Arc::new(ConnectionDispatcher::new(config, health));

    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let _ = dispatcher.handle_connection(stream).await;
        }
    });

    let mut client = UnixStream::connect(&sock_path)
        .await
        .expect("Connect failed");

    // Request target UID 99999 (which is different from client's real UID unless caller is root)
    let current_uid = nix::unistd::getuid().as_raw();
    if current_uid != 0 {
        let spoofed_uid = 99999;
        let req = make_auth_request(spoofed_uid);
        let framed = encode(&req).expect("Encoding failed");

        client
            .write_all(&framed)
            .await
            .expect("Write framed request failed");
        client.flush().await.expect("Flush failed");

        let mut resp_len_bytes = [0u8; 4];
        client
            .read_exact(&mut resp_len_bytes)
            .await
            .expect("Read resp len failed");
        let resp_len = u32::from_be_bytes(resp_len_bytes) as usize;
        let mut full_resp_buf = Vec::with_capacity(4 + resp_len);
        full_resp_buf.extend_from_slice(&resp_len_bytes);
        let mut resp_payload = vec![0u8; resp_len];
        client
            .read_exact(&mut resp_payload)
            .await
            .expect("Read resp payload failed");
        full_resp_buf.extend_from_slice(&resp_payload);

        let resp: Response = decode(&full_resp_buf).expect("Decode response failed");
        assert_eq!(resp.verdict, Verdict::ProtocolError);
        assert_eq!(resp.reason_class, ReasonClass::UidMismatch);
    }
}

#[tokio::test]
async fn test_dispatcher_timeout_on_idle_connection() {
    let dir = tempdir().expect("Failed to create tempdir");
    let sock_path = dir.path().join("dispatch_timeout.sock");

    let listener = UnixListener::bind(&sock_path).expect("Bind failed");
    let health = Arc::new(HealthState::new());

    let config = test_dispatcher_config(4, Duration::from_millis(50));
    let dispatcher = Arc::new(ConnectionDispatcher::new(config, health));

    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let _ = dispatcher.handle_connection(stream).await;
        }
    });

    let mut client = UnixStream::connect(&sock_path)
        .await
        .expect("Connect failed");

    // Connect but do not send any data; wait past timeout
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Stream should be closed by daemon
    let mut buf = [0u8; 1];
    let n = client.read(&mut buf).await.expect("Read failed");
    assert_eq!(n, 0, "Server must close stream after connection timeout");
}

#[tokio::test]
async fn test_dispatcher_rejects_oversized_payload() {
    let dir = tempdir().expect("Failed to create tempdir");
    let sock_path = dir.path().join("dispatch_oversized.sock");

    let listener = UnixListener::bind(&sock_path).expect("Bind failed");
    let health = Arc::new(HealthState::new());

    let config = test_dispatcher_config(4, Duration::from_millis(500));
    let dispatcher = Arc::new(ConnectionDispatcher::new(config, health));

    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let _ = dispatcher.handle_connection(stream).await;
        }
    });

    let mut client = UnixStream::connect(&sock_path)
        .await
        .expect("Connect failed");

    // Send length prefix > 4096 bytes (e.g. 5000 bytes)
    let bad_len: u32 = 5000;
    client
        .write_all(&bad_len.to_be_bytes())
        .await
        .expect("Write len failed");
    client.flush().await.expect("Flush failed");

    // Daemon should immediately close the connection without reading 5000 bytes
    let mut buf = [0u8; 1];
    let n = client.read(&mut buf).await.expect("Read failed");
    assert_eq!(n, 0, "Server must close connection on oversized frame");
}

#[tokio::test]
async fn test_dispatcher_concurrency_bounding() {
    let dir = tempdir().expect("Failed to create tempdir");
    let sock_path = dir.path().join("dispatch_concurrency.sock");

    let _listener = UnixListener::bind(&sock_path).expect("Bind failed");
    let health = Arc::new(HealthState::new());

    let config = test_dispatcher_config(2, Duration::from_millis(200));
    let dispatcher = ConnectionDispatcher::new(config, health);

    // Semaphore permits should be 2
    assert_eq!(dispatcher.available_permits(), 2);
}

#[tokio::test]
async fn test_dispatcher_no_pipeline_returns_unavailable_not_allow() {
    let dir = tempdir().expect("Failed to create tempdir");
    let sock_path = dir.path().join("dispatch_no_pipe.sock");

    let listener = UnixListener::bind(&sock_path).expect("Bind failed");
    let health = Arc::new(HealthState::new());
    health.set_socket_ready(true);

    let config = test_dispatcher_config(4, Duration::from_millis(500));
    // Initialize dispatcher without pipeline
    let dispatcher = Arc::new(ConnectionDispatcher::new(config, health));

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
    let framed = encode(&req).expect("Encoding failed");

    client
        .write_all(&framed)
        .await
        .expect("Write framed request failed");
    client.flush().await.expect("Flush failed");

    let mut resp_len_bytes = [0u8; 4];
    client
        .read_exact(&mut resp_len_bytes)
        .await
        .expect("Read resp len failed");
    let resp_len = u32::from_be_bytes(resp_len_bytes) as usize;

    let mut full_resp_buf = Vec::with_capacity(4 + resp_len);
    full_resp_buf.extend_from_slice(&resp_len_bytes);
    let mut resp_payload = vec![0u8; resp_len];
    client
        .read_exact(&mut resp_payload)
        .await
        .expect("Read resp payload failed");
    full_resp_buf.extend_from_slice(&resp_payload);

    let resp: Response = decode(&full_resp_buf).expect("Decode response failed");
    assert_eq!(resp.request_id, [1u8; 32]);
    // Strict invariant: fail-closed fallback must NEVER return Verdict::Allow
    assert_ne!(
        resp.verdict,
        Verdict::Allow,
        "Dispatcher with no pipeline must never grant Allow"
    );
    assert_eq!(
        resp.verdict,
        Verdict::Unavailable,
        "Dispatcher with no pipeline must return Unavailable"
    );
    assert_eq!(
        resp.reason_class,
        ReasonClass::InternalError,
        "Dispatcher with no pipeline must report InternalError"
    );
}

#[tokio::test]
async fn test_dispatcher_rejects_invalid_protocol_version() {
    let dir = tempdir().expect("Failed to create tempdir");
    let sock_path = dir.path().join("dispatch_ver.sock");

    let listener = UnixListener::bind(&sock_path).expect("Bind failed");
    let health = Arc::new(HealthState::new());
    health.set_socket_ready(true);

    let config = test_dispatcher_config(4, Duration::from_millis(500));
    let dispatcher = Arc::new(ConnectionDispatcher::new(config, health));

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
    let mut req = make_auth_request(current_uid);
    req.version = 99; // Invalid version != CURRENT_VERSION
    let framed = encode(&req).expect("Encoding failed");

    client
        .write_all(&framed)
        .await
        .expect("Write framed request failed");
    client.flush().await.expect("Flush failed");

    let mut resp_len_bytes = [0u8; 4];
    client
        .read_exact(&mut resp_len_bytes)
        .await
        .expect("Read resp len failed");
    let resp_len = u32::from_be_bytes(resp_len_bytes) as usize;

    let mut full_resp_buf = Vec::with_capacity(4 + resp_len);
    full_resp_buf.extend_from_slice(&resp_len_bytes);
    let mut resp_payload = vec![0u8; resp_len];
    client
        .read_exact(&mut resp_payload)
        .await
        .expect("Read resp payload failed");
    full_resp_buf.extend_from_slice(&resp_payload);

    let resp: Response = decode(&full_resp_buf).expect("Decode response failed");
    assert_eq!(
        resp.verdict,
        Verdict::ProtocolError,
        "Acceptance 20.3: Requests with invalid protocol version must be rejected with ProtocolError"
    );
    assert_eq!(
        resp.reason_class,
        ReasonClass::MalformedRequest,
        "Acceptance 20.3: Requests with invalid protocol version must report MalformedRequest"
    );
}

#[tokio::test]
async fn test_dispatcher_rejects_oversized_service_name() {
    let dir = tempdir().expect("Failed to create tempdir");
    let sock_path = dir.path().join("dispatch_svc.sock");

    let listener = UnixListener::bind(&sock_path).expect("Bind failed");
    let health = Arc::new(HealthState::new());
    health.set_socket_ready(true);

    let config = test_dispatcher_config(4, Duration::from_millis(500));
    let dispatcher = Arc::new(ConnectionDispatcher::new(config, health));

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
    let mut req = make_auth_request(current_uid);
    // Exceeds MAX_SERVICE_LEN (64)
    req.service = "a".repeat(128);
    let framed = encode(&req).expect("Encoding failed");

    client
        .write_all(&framed)
        .await
        .expect("Write framed request failed");
    client.flush().await.expect("Flush failed");

    let mut resp_len_bytes = [0u8; 4];
    client
        .read_exact(&mut resp_len_bytes)
        .await
        .expect("Read resp len failed");
    let resp_len = u32::from_be_bytes(resp_len_bytes) as usize;

    let mut full_resp_buf = Vec::with_capacity(4 + resp_len);
    full_resp_buf.extend_from_slice(&resp_len_bytes);
    let mut resp_payload = vec![0u8; resp_len];
    client
        .read_exact(&mut resp_payload)
        .await
        .expect("Read resp payload failed");
    full_resp_buf.extend_from_slice(&resp_payload);

    let resp: Response = decode(&full_resp_buf).expect("Decode response failed");
    assert_eq!(
        resp.verdict,
        Verdict::ProtocolError,
        "Acceptance 20.3: Requests with oversized service name must be rejected with ProtocolError"
    );
    assert_eq!(
        resp.reason_class,
        ReasonClass::MalformedRequest,
        "Acceptance 20.3: Requests with oversized service name must report MalformedRequest"
    );
}

/// Sub-issue #21.1 TDD Contract: Partial response writes never reach the PAM client upon timeout.
///
/// Asserts:
/// 1. When a connection timeout expires on the daemon side, no partial or corrupted response bytes reach the client.
/// 2. Client receives either a complete, valid decodable response, or an immediate clean EOF (0 bytes).
#[tokio::test]
async fn test_timeout_during_write_does_not_corrupt_response() {
    let dir = tempdir().expect("Failed to create tempdir");
    let sock_path = dir.path().join("dispatch_cancel_safety.sock");

    let listener = UnixListener::bind(&sock_path).expect("Bind failed");
    let health = Arc::new(HealthState::new());
    health.set_socket_ready(true);

    // Tight timeout of 30ms to exercise async cancellation during request handling
    let config = test_dispatcher_config(4, Duration::from_millis(30));
    let dispatcher = Arc::new(ConnectionDispatcher::new(config, health));

    let disp_clone = dispatcher.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let d = disp_clone.clone();
            tokio::spawn(async move {
                let _ = d.handle_connection(stream).await;
            });
        }
    });

    // Test 1: Incomplete request where client stalls mid-stream
    let mut client1 = UnixStream::connect(&sock_path)
        .await
        .expect("Connect failed");
    // Write only length prefix claiming 100 bytes, then stall
    let partial_req = 100u32.to_be_bytes();
    client1.write_all(&partial_req).await.expect("Write prefix");
    tokio::time::sleep(Duration::from_millis(60)).await;

    let mut buf1 = Vec::new();
    let n1 = client1.read_to_end(&mut buf1).await.expect("Read to end");
    assert_eq!(
        n1, 0,
        "Server must close stream with zero bytes on client stall/timeout; partial response forbidden"
    );

    // Test 2: Valid request roundtrip completes cleanly without corruption
    let mut client2 = UnixStream::connect(&sock_path)
        .await
        .expect("Connect failed");
    let current_uid = nix::unistd::getuid().as_raw();
    let req = make_auth_request(current_uid);
    let framed = encode(&req).expect("Encoding failed");
    client2
        .write_all(&framed)
        .await
        .expect("Write framed request");
    client2.flush().await.expect("Flush failed");

    let mut buf2 = Vec::new();
    let _ = client2
        .read_to_end(&mut buf2)
        .await
        .expect("Read response bytes");
    if !buf2.is_empty() {
        // If response was sent, it MUST be valid and uncorrupted!
        let resp: Result<Response, _> = decode(&buf2);
        assert!(
            resp.is_ok(),
            "Delivered response must decode cleanly into a valid Response without byte corruption"
        );
    }
}

#[tokio::test]
async fn test_dispatcher_preview_frame_roundtrip() {
    let dir = tempdir().expect("Failed to create tempdir");
    let sock_path = dir.path().join("preview_dispatch.sock");

    let listener = UnixListener::bind(&sock_path).expect("Bind failed");
    let health = Arc::new(HealthState::new());
    health.set_socket_ready(true);

    let config = test_dispatcher_config(4, Duration::from_millis(500));
    let dispatcher = Arc::new(ConnectionDispatcher::new(config, health));

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
    let req = Request {
        version: CURRENT_VERSION,
        kind: RequestKind::PreviewFrame,
        request_id: [77u8; 32],
        uid_hint: current_uid,
        service: "soos-gui".to_string(),
        deadline_monotonic_ns: u64::MAX,
    };
    let framed = encode(&req).expect("Encoding failed");
    client.write_all(&framed).await.expect("Write request");
    client.flush().await.expect("Flush client");

    let mut len_bytes = [0u8; 4];
    client.read_exact(&mut len_bytes).await.expect("Read len");
    let resp_len = u32::from_be_bytes(len_bytes) as usize;
    let mut buf = vec![0u8; 4 + resp_len];
    buf[..4].copy_from_slice(&len_bytes);
    client.read_exact(&mut buf[4..]).await.expect("Read body");

    let preview_resp: PreviewResponse = decode_preview(&buf).expect("Decode preview response");
    assert_eq!(preview_resp.version, CURRENT_VERSION);
    assert_eq!(preview_resp.format, 255);
    assert!(preview_resp.data.is_empty());
}
