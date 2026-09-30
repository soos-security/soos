//! Contract tests for GitHub #201 (review finding DMN-12): the diagnostic `Status` response
//! reports whether `mlockall` swap protection is active, so operators can see it through
//! `soos-admin` instead of a startup log line only.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    reason = "Contract tests use assertions, unwrap and indexing"
)]

use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use soos_daemon::config::DispatcherConfig;
use soos_daemon::dispatcher::ConnectionDispatcher;
use soos_daemon::health::HealthState;
use soos_protocol::codec::decode;
use soos_protocol::message::encode_request;
use soos_protocol::types::{Request, RequestKind, StatusResponse, CURRENT_VERSION, REQUEST_ID_LEN};

async fn query_status(memory_locked: Option<bool>) -> StatusResponse {
    let dir = tempdir().expect("tempdir");
    let sock_path = dir.path().join("status_memory_locked.sock");
    let listener = UnixListener::bind(&sock_path).expect("bind");
    let health = Arc::new(HealthState::new());
    health.set_socket_ready(true);
    if let Some(locked) = memory_locked {
        health.set_memory_locked(locked);
    }
    let dispatcher = Arc::new(ConnectionDispatcher::new(
        DispatcherConfig {
            max_concurrent_connections: 4,
            connection_timeout: Duration::from_secs(3),
            enforce_active_session: false,
            logind_sessions_dir: dir.path().to_path_buf(),
        },
        health,
    ));
    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let _ = dispatcher.handle_connection(stream).await;
        }
    });

    let request = Request {
        version: CURRENT_VERSION,
        kind: RequestKind::Status,
        request_id: [0x5A; REQUEST_ID_LEN],
        uid_hint: nix::unistd::getuid().as_raw(),
        service: "soos-admin".into(),
        deadline_monotonic_ns: 0,
    };
    let mut client = UnixStream::connect(&sock_path).await.expect("connect");
    client
        .write_all(&encode_request(&request).expect("encode"))
        .await
        .expect("write");
    let mut len_bytes = [0u8; 4];
    tokio::time::timeout(Duration::from_secs(2), client.read_exact(&mut len_bytes))
        .await
        .expect("daemon must answer before the test timeout")
        .expect("read length");
    let len = u32::from_be_bytes(len_bytes) as usize;
    let mut frame = vec![0u8; 4 + len];
    frame[..4].copy_from_slice(&len_bytes);
    client.read_exact(&mut frame[4..]).await.expect("read body");
    decode(&frame).expect("decode StatusResponse")
}

#[tokio::test]
async fn test_201_status_reports_memory_locked_after_mlockall_success() {
    assert!(query_status(Some(true)).await.memory_locked);
}

#[tokio::test]
async fn test_201_status_reports_memory_unlocked_after_mlockall_refusal() {
    assert!(!query_status(Some(false)).await.memory_locked);
}

#[tokio::test]
async fn test_201_status_never_claims_memory_locked_by_default() {
    assert!(
        !query_status(None).await.memory_locked,
        "Status must not claim swap protection before mlockall succeeded"
    );
}
