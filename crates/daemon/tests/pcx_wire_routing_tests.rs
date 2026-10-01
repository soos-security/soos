//! Regression contract for GitHub #224 (review finding PAM-12, matrix row PCX4): the exact
//! frame the removed UID heuristic mis-routed (a Request whose body also decodes as an
//! Event, with `uid_hint` different from the peer UID) is served as a Request when tagged.

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
use soos_protocol::codec::decode;
use soos_protocol::message::encode_request;
use soos_protocol::types::{
    Request, RequestKind, Response, Verdict, CURRENT_VERSION, REQUEST_ID_LEN,
};

struct Server {
    sock_path: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

fn spawn_server() -> Server {
    let dir = tempdir().expect("tempdir");
    let sock_path = dir.path().join("pcx_wire.sock");
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

/// Unprivileged UID the test process switches to when it is started as root.
const UNPRIVILEGED_TEST_UID: u32 = 65_534;

/// Restores root when dropped (see [`run_as_unprivileged_peer_when_root`]).
struct RootRestore(bool);

impl Drop for RootRestore {
    fn drop(&mut self) {
        if self.0 {
            let root = nix::unistd::Uid::from_raw(0);
            // The saved set-user-ID stayed 0, so returning to root is always permitted.
            nix::unistd::setresuid(root, root, root).expect("restore root identity");
        }
    }
}

/// Root-safe setup (GitHub #287): the scenario needs a peer whose kernel UID differs from
/// `uid_hint` 0, which a root test process can never be. When started as root, the whole
/// test process switches its real and effective UID to an unprivileged UID (saved UID kept
/// at 0) BEFORE any socket or temporary directory is created, so the server, the client
/// and the `SO_PEERCRED` check see exactly what a developer account sees. Non-root runs
/// are unchanged. The single test of this binary runs on a current-thread runtime.
fn run_as_unprivileged_peer_when_root() -> RootRestore {
    if !nix::unistd::getuid().is_root() {
        return RootRestore(false);
    }
    let unprivileged = nix::unistd::Uid::from_raw(UNPRIVILEGED_TEST_UID);
    nix::unistd::setresuid(unprivileged, unprivileged, nix::unistd::Uid::from_raw(0))
        .expect("switch to an unprivileged UID");
    RootRestore(true)
}

async fn send(client: &mut UnixStream, frame: &[u8]) {
    client.write_all(frame).await.expect("write");
    client.flush().await.expect("flush");
}

#[tokio::test]
async fn test_pcx_tagged_ambiguous_request_with_foreign_uid_hint_gets_a_response() {
    // Precondition: the peer UID differs from `uid_hint`, which the removed heuristic
    // treated as "this must be an Event" (no response, client timeout).
    let _identity = run_as_unprivileged_peer_when_root();
    assert_ne!(nix::unistd::getuid().as_raw(), 0, "run as a non-root peer");
    let server = spawn_server();
    let mut client = UnixStream::connect(&server.sock_path)
        .await
        .expect("connect");
    let req = ambiguous_request();
    send(&mut client, &encode_request(&req).expect("encode")).await;
    let frame = read_frame(&mut client)
        .await
        .expect("a tagged Request is always answered, never consumed as an Event");
    let resp: Response = decode(&frame).expect("decode");
    assert_eq!(resp.request_id, req.request_id);
    assert_ne!(resp.verdict, Verdict::Allow, "fail closed");
}
