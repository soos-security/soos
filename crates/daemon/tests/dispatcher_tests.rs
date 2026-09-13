//! Contractual integration tests for connection dispatcher.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use soos_daemon::config::DispatcherConfig;
use soos_daemon::dispatcher::ConnectionDispatcher;
use soos_daemon::health::HealthState;
use soos_protocol::codec::{decode, encode};
use soos_protocol::types::{ReasonClass, Request, RequestKind, Response, Verdict, CURRENT_VERSION};

fn make_auth_request(uid: u32) -> Request {
    Request {
        version: CURRENT_VERSION,
        kind: RequestKind::Auth,
        request_id: [1u8; 32],
        uid_hint: uid,
        service: "sudo".to_string(),
        deadline_monotonic_ns: 1_000_000_000,
    }
}

#[tokio::test]
async fn test_dispatcher_nominal_roundtrip() {
    let dir = tempdir().expect("Failed to create tempdir");
    let sock_path = dir.path().join("dispatch.sock");

    let listener = UnixListener::bind(&sock_path).expect("Bind failed");
    let health = Arc::new(HealthState::new());
    health.set_socket_ready(true);

    let config = DispatcherConfig {
        max_concurrent_connections: 4,
        connection_timeout: Duration::from_millis(500),
    };
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

    let config = DispatcherConfig {
        max_concurrent_connections: 4,
        connection_timeout: Duration::from_millis(500),
    };
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

    let config = DispatcherConfig {
        max_concurrent_connections: 4,
        connection_timeout: Duration::from_millis(50),
    };
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

    let config = DispatcherConfig {
        max_concurrent_connections: 4,
        connection_timeout: Duration::from_millis(500),
    };
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

    let config = DispatcherConfig {
        max_concurrent_connections: 2,
        connection_timeout: Duration::from_millis(200),
    };
    let dispatcher = ConnectionDispatcher::new(config, health);

    // Semaphore permits should be 2
    assert_eq!(dispatcher.available_permits(), 2);
}
