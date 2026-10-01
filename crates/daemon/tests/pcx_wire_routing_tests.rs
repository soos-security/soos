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

/// Unprivileged UID the test thread switches to when the suite is started as root.
const UNPRIVILEGED_TEST_UID: libc::uid_t = 65_534;

/// Per-thread `setresuid` syscall number taking 32-bit UIDs.
///
/// On legacy 32-bit x86 (and 32-bit ARM) `SYS_setresuid` is the historical 16-bit-UID call;
/// the 32-bit-UID variant is `SYS_setresuid32` (GitHub #289). The 64-bit targets (`x86_64`,
/// `aarch64`, ...) have a single 32-bit-UID `SYS_setresuid`.
#[cfg(any(target_arch = "x86", target_arch = "arm"))]
const SETRESUID_SYSCALL: libc::c_long = libc::SYS_setresuid32;
#[cfg(not(any(target_arch = "x86", target_arch = "arm")))]
const SETRESUID_SYSCALL: libc::c_long = libc::SYS_setresuid;

/// Sets the real, effective and saved UID of the CALLING THREAD only.
///
/// The raw syscall is used on purpose: glibc's `setresuid` wrapper (and `nix`) broadcast the
/// change to every thread of the process, so concurrently running tests of this binary (the
/// libtest harness runs tests on parallel threads) would execute as the unprivileged UID.
/// The kernel keeps credentials per thread, so the raw call changes only this test's thread
/// (and threads it creates afterwards).
fn set_thread_uids(ruid: libc::uid_t, euid: libc::uid_t, suid: libc::uid_t) -> std::io::Result<()> {
    // SAFETY: SETRESUID_SYSCALL (SYS_setresuid, or SYS_setresuid32 on 32-bit x86 / ARM) takes three
    // 32-bit uid_t values and reads or writes no memory of this process; the return value is
    // checked below.
    let ret = unsafe { libc::syscall(SETRESUID_SYSCALL, ruid, euid, suid) };
    if ret == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Restores root on the test thread when dropped (see [`run_as_unprivileged_peer_when_root`]).
struct RootRestore(bool);

impl Drop for RootRestore {
    fn drop(&mut self) {
        if self.0 {
            // The saved set-user-ID stayed 0, so returning to root is always permitted.
            set_thread_uids(0, 0, 0).expect("restore root identity");
        }
    }
}

/// Root-safe setup (GitHub #287): the scenario needs a peer whose kernel UID differs from
/// `uid_hint` 0, which a root test thread can never be. When started as root, the test
/// thread alone switches its real and effective UID to an unprivileged UID (saved UID kept
/// at 0) BEFORE any socket or temporary directory is created. `#[tokio::test]` runs a
/// current-thread runtime on this same thread, so the listener, the dispatcher tasks, the
/// client `connect` and therefore `SO_PEERCRED` all see what a developer account sees.
/// Other tests of the binary keep their own credentials. Non-root runs are unchanged.
fn run_as_unprivileged_peer_when_root() -> RootRestore {
    if !nix::unistd::getuid().is_root() {
        return RootRestore(false);
    }
    set_thread_uids(UNPRIVILEGED_TEST_UID, UNPRIVILEGED_TEST_UID, 0)
        .expect("switch the test thread to an unprivileged UID");
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
