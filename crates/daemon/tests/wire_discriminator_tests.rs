//! Contract tests for GitHub #204 (review finding DMN-15): the dispatcher classifies every
//! client frame by protocol rule (message tag trailer, or an unambiguous legacy frame),
//! never by comparing `uid_hint` with the peer UID.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
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
use soos_protocol::codec::{decode, encode};
use soos_protocol::message::{encode_event, encode_request};
use soos_protocol::types::{
    Event, EventKind, ReasonClass, Request, RequestKind, Response, StatusResponse, Verdict,
    CURRENT_VERSION, REQUEST_ID_LEN,
};

struct Server {
    sock_path: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

fn spawn_server() -> Server {
    let dir = tempdir().expect("tempdir");
    let sock_path = dir.path().join("wire_discriminator.sock");
    let listener = UnixListener::bind(&sock_path).expect("bind");
    let health = Arc::new(HealthState::new());
    health.set_socket_ready(true);
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
        while let Ok((stream, _)) = listener.accept().await {
            let d = dispatcher.clone();
            tokio::spawn(async move {
                let _ = d.handle_connection(stream).await;
            });
        }
    });
    Server {
        sock_path,
        _dir: dir,
    }
}

fn status_request(tag: u8) -> Request {
    Request {
        version: CURRENT_VERSION,
        kind: RequestKind::Status,
        request_id: [tag; REQUEST_ID_LEN],
        uid_hint: nix::unistd::getuid().as_raw(),
        service: "soos-admin".into(),
        deadline_monotonic_ns: 0,
    }
}

/// Legacy (untagged) request that also decodes exactly as an `Event` (see the protocol
/// contract `client_message_tests::ambiguous_request`).
fn ambiguous_request() -> Request {
    let mut request_id = [b'a'; REQUEST_ID_LEN];
    request_id[0] = 0x00;
    request_id[1] = 0x01;
    request_id[2] = 0x05;
    request_id[3] = 30;
    Request {
        version: CURRENT_VERSION,
        kind: RequestKind::Auth,
        request_id,
        uid_hint: 0,
        service: String::new(),
        deadline_monotonic_ns: 0,
    }
}

/// Reads one framed reply; `None` when the daemon closed the connection instead.
async fn read_frame(client: &mut UnixStream) -> Option<Vec<u8>> {
    let mut len_bytes = [0u8; 4];
    tokio::time::timeout(Duration::from_secs(2), client.read_exact(&mut len_bytes))
        .await
        .expect("daemon must answer or close before the test timeout")
        .ok()?;
    let len = u32::from_be_bytes(len_bytes) as usize;
    let mut buf = vec![0u8; 4 + len];
    buf[..4].copy_from_slice(&len_bytes);
    client.read_exact(&mut buf[4..]).await.ok()?;
    Some(buf)
}

async fn send(client: &mut UnixStream, frame: &[u8]) {
    client.write_all(frame).await.expect("write");
    client.flush().await.expect("flush");
}

#[tokio::test]
async fn test_204_ambiguous_legacy_frame_is_rejected_and_connection_closed() {
    let server = spawn_server();
    let mut client = UnixStream::connect(&server.sock_path)
        .await
        .expect("connect");
    send(&mut client, &encode(&ambiguous_request()).expect("encode")).await;
    // Neither handler runs: no Auth response, and the connection is closed so the
    // following (legacy, unambiguous) Status request is never served either.
    let _ = client
        .write_all(&encode(&status_request(7)).expect("encode"))
        .await;
    assert!(
        read_frame(&mut client).await.is_none(),
        "an ambiguous frame must be rejected, never dispatched"
    );
}

#[tokio::test]
async fn test_204_tagged_status_request_is_served() {
    let server = spawn_server();
    let mut client = UnixStream::connect(&server.sock_path)
        .await
        .expect("connect");
    send(
        &mut client,
        &encode_request(&status_request(1)).expect("encode"),
    )
    .await;
    let frame = read_frame(&mut client).await.expect("status response");
    let status: StatusResponse = decode(&frame).expect("decode");
    assert_eq!(status.version, CURRENT_VERSION);
    assert!(status.socket_ready);
}

#[tokio::test]
async fn test_204_tagged_auth_request_is_served_as_request() {
    let server = spawn_server();
    let mut client = UnixStream::connect(&server.sock_path)
        .await
        .expect("connect");
    let mut req = status_request(3);
    req.kind = RequestKind::Auth;
    req.service = "sudo".into();
    req.deadline_monotonic_ns = u64::MAX;
    send(&mut client, &encode_request(&req).expect("encode")).await;
    let frame = read_frame(&mut client).await.expect("auth response");
    let resp: Response = decode(&frame).expect("decode");
    assert_eq!(resp.request_id, [3; REQUEST_ID_LEN]);
    // No pipeline in this harness: fail closed.
    assert_eq!(
        (resp.verdict, resp.reason_class),
        (Verdict::Unavailable, ReasonClass::InternalError)
    );
}

#[tokio::test]
async fn test_204_tagged_event_is_consumed_without_response() {
    let server = spawn_server();
    let mut client = UnixStream::connect(&server.sock_path)
        .await
        .expect("connect");
    let event = Event {
        version: CURRENT_VERSION,
        kind: EventKind::PasswordFailed,
        request_id: None,
        uid: Some(nix::unistd::getuid().as_raw()),
        service: "sudo".into(),
        timestamp_monotonic_ns: 1,
    };
    send(&mut client, &encode_event(&event).expect("encode")).await;
    send(
        &mut client,
        &encode_request(&status_request(2)).expect("encode"),
    )
    .await;
    // The event produced no reply: the first frame on the wire is the Status response.
    let frame = read_frame(&mut client).await.expect("status response");
    let status: StatusResponse = decode(&frame).expect("decode");
    assert!(status.socket_ready);
}

#[tokio::test]
async fn test_204_unknown_message_tag_is_rejected() {
    let server = spawn_server();
    let mut client = UnixStream::connect(&server.sock_path)
        .await
        .expect("connect");
    let mut frame = encode_request(&status_request(4)).expect("encode");
    *frame.last_mut().unwrap() = 0xFE;
    send(&mut client, &frame).await;
    assert!(read_frame(&mut client).await.is_none());
}

#[tokio::test]
async fn test_204_legacy_untagged_status_request_still_served() {
    let server = spawn_server();
    let mut client = UnixStream::connect(&server.sock_path)
        .await
        .expect("connect");
    send(&mut client, &encode(&status_request(5)).expect("encode")).await;
    let frame = read_frame(&mut client).await.expect("status response");
    let status: StatusResponse = decode(&frame).expect("decode");
    assert!(status.socket_ready);
}
