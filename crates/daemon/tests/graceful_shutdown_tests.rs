//! Graceful shutdown contract (GitHub #259, review finding DMN-19).
//!
//! Connection handlers live in a tracked task set: finished handlers are reaped while the
//! accept loop runs (bounded memory), a shutdown signal stops accepting, and the remaining
//! handlers are drained within a bounded budget; stragglers are aborted (fail-closed EOF).
//! A panicking handler is reported through `tracing`, never with its panic payload.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tracing_subscriber::fmt::MakeWriter;

use soos_daemon::config::DispatcherConfig;
use soos_daemon::dispatcher::ConnectionDispatcher;
use soos_daemon::health::HealthState;
use soos_daemon::shutdown::{accept_until_shutdown, ConnectionTasks};
use soos_protocol::codec::encode;
use soos_protocol::types::{Request, RequestKind, CURRENT_VERSION};

/// In-memory log sink shared by a test subscriber.
#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    fn contents(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

impl Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for LogBuffer {
    type Writer = Self;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn capture_logs() -> (LogBuffer, tracing::subscriber::DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buffer.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    (buffer, guard)
}

fn dispatcher(connection_timeout: Duration) -> Arc<ConnectionDispatcher> {
    let health = Arc::new(HealthState::new());
    health.set_socket_ready(true);
    let config = DispatcherConfig {
        max_concurrent_connections: 8,
        connection_timeout,
        enforce_active_session: false,
        logind_sessions_dir: PathBuf::from("/run/systemd/sessions"),
    };
    Arc::new(ConnectionDispatcher::new(config, health))
}

#[tokio::test]
async fn test_drain_waits_for_in_flight_handler_within_budget() {
    let mut tasks = ConnectionTasks::new();
    tasks.spawn(async {
        tokio::time::sleep(Duration::from_millis(100)).await;
    });
    assert_eq!(tasks.len(), 1);

    let started = Instant::now();
    let report = tasks.drain(Duration::from_secs(5)).await;
    assert_eq!(report.completed, 1);
    assert_eq!(report.aborted, 0);
    assert_eq!(report.panicked, 0);
    assert!(report.is_clean());
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn test_drain_aborts_handlers_that_exceed_the_budget() {
    let mut tasks = ConnectionTasks::new();
    tasks.spawn(std::future::pending::<()>());
    tasks.spawn(async {});

    let started = Instant::now();
    let report = tasks.drain(Duration::from_millis(150)).await;
    let elapsed = started.elapsed();
    assert_eq!(
        report.aborted, 1,
        "the never-ending handler must be aborted"
    );
    assert_eq!(report.completed, 1);
    assert!(!report.is_clean());
    assert!(
        elapsed < Duration::from_secs(2),
        "drain must be bounded by its budget, took {elapsed:?}"
    );
}

#[tokio::test]
async fn test_handler_panic_is_logged_via_tracing_without_payload() {
    let (logs, _guard) = capture_logs();
    let mut tasks = ConnectionTasks::new();
    tasks.spawn(async {
        panic!("DMN19-SENSITIVE-PANIC-PAYLOAD");
    });

    let report = tasks.drain(Duration::from_secs(5)).await;
    assert_eq!(report.panicked, 1);
    assert!(!report.is_clean());

    let text = logs.contents();
    assert!(
        text.contains("Connection handler panicked"),
        "handler panic must be reported through tracing, got: {text}"
    );
    assert!(
        text.contains("ERROR"),
        "handler panic must log at error level"
    );
    assert!(
        !text.contains("DMN19-SENSITIVE-PANIC-PAYLOAD"),
        "the panic payload must never reach the logs"
    );
}

#[tokio::test]
async fn test_drain_logs_its_outcome() {
    let (logs, _guard) = capture_logs();
    let mut tasks = ConnectionTasks::new();
    tasks.spawn(async {});
    let _ = tasks.drain(Duration::from_secs(1)).await;
    let text = logs.contents();
    assert!(
        text.contains("Connection drain finished"),
        "the drain must be logged, got: {text}"
    );
}

#[tokio::test]
async fn test_accept_loop_reaps_finished_handlers() {
    let dir = tempdir().expect("tempdir");
    let sock_path = dir.path().join("reap.sock");
    let listener = UnixListener::bind(&sock_path).expect("bind");
    let disp = dispatcher(Duration::from_millis(500));
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();

    let server = tokio::spawn(async move {
        let mut tasks = ConnectionTasks::new();
        accept_until_shutdown(&listener, disp, &mut tasks, rx).await;
        tasks
    });

    let uid = nix::unistd::getuid().as_raw();
    for i in 0..5u8 {
        let mut client = UnixStream::connect(&sock_path).await.expect("connect");
        let req = Request {
            version: CURRENT_VERSION,
            kind: RequestKind::Auth,
            request_id: [i; 32],
            uid_hint: uid,
            service: "sudo".to_string(),
            deadline_monotonic_ns: u64::MAX,
        };
        client
            .write_all(&encode(&req).expect("encode"))
            .await
            .expect("write");
        let mut sink = Vec::new();
        // Auth is one-shot: the daemon answers and closes the connection.
        client.read_to_end(&mut sink).await.expect("read");
        assert!(!sink.is_empty());
    }
    tokio::time::sleep(Duration::from_millis(200)).await;

    tx.send(()).expect("signal shutdown");
    let tasks = server.await.expect("accept loop task");
    assert_eq!(
        tasks.len(),
        0,
        "finished handlers must be reaped while the accept loop runs"
    );
}

#[tokio::test]
async fn test_shutdown_stops_accepting_and_drains_in_flight_connection() {
    let dir = tempdir().expect("tempdir");
    let sock_path = dir.path().join("drain.sock");
    let listener = UnixListener::bind(&sock_path).expect("bind");
    let connection_timeout = Duration::from_millis(300);
    let disp = dispatcher(connection_timeout);
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();

    let server = tokio::spawn(async move {
        let mut tasks = ConnectionTasks::new();
        accept_until_shutdown(&listener, disp, &mut tasks, rx).await;
        // The accept loop returned: dropping the listener stops accepting.
        drop(listener);
        tasks
    });

    // An in-flight client that never sends its request.
    let mut in_flight = UnixStream::connect(&sock_path).await.expect("connect");
    tokio::time::sleep(Duration::from_millis(100)).await;

    tx.send(()).expect("signal shutdown");
    let tasks = server.await.expect("accept loop task");
    assert_eq!(
        tasks.len(),
        1,
        "the in-flight handler must still be tracked"
    );

    assert!(
        UnixStream::connect(&sock_path).await.is_err(),
        "no connection may be accepted after shutdown"
    );

    let started = Instant::now();
    let report = tasks.drain(connection_timeout).await;
    let elapsed = started.elapsed();
    assert_eq!(report.completed + report.aborted, 1);
    assert_eq!(report.panicked, 0);
    assert!(
        elapsed < connection_timeout + Duration::from_secs(1),
        "drain must complete within connection_timeout, took {elapsed:?}"
    );

    // The client observes a closed connection (fail-closed EOF), never a verdict.
    let mut buf = Vec::new();
    let read = tokio::time::timeout(Duration::from_secs(2), in_flight.read_to_end(&mut buf))
        .await
        .expect("client read must not hang after drain");
    assert!(read.is_ok());
    assert!(
        buf.is_empty(),
        "no response bytes are sent to an idle client"
    );
}

#[test]
fn test_production_main_wires_tracked_handlers_and_bounded_drain() {
    let main_rs = include_str!("../src/main.rs");
    assert!(
        main_rs.contains("install_panic_hook();"),
        "soos-daemon must install the tracing panic hook"
    );
    assert!(
        main_rs.contains("accept_until_shutdown("),
        "connection handlers must be tracked, not detached"
    );
    let unlink = main_rs
        .find("drop(socket_guard);")
        .expect("socket guard dropped at shutdown");
    let drain = main_rs
        .find(".drain(drain_budget)")
        .expect("bounded drain at shutdown");
    assert!(
        unlink < drain,
        "the socket must be unlinked before draining in-flight connections"
    );
    assert!(
        main_rs.contains("let drain_budget = config.dispatcher.connection_timeout;"),
        "the drain budget is one connection_timeout"
    );
}
