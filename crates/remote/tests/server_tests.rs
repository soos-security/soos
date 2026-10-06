//! End-to-end contract tests of GitHub #339 for `soos-remote::server::serve` (spec §2.1
//! D3–D9, §2.5 route table, §2.8, §4, §7 mandatory paused-time tests, RC-2…RC-5).
//!
//! Raw HTTP/1.1 is written over a `tokio::net::UnixStream` to a listener bound in a
//! `TempDir`; logind is [`MockSource`] (scripted answers, held calls, call counters); the
//! Unix clock is injected through `ServerState::with_unix_clock` and follows the paused
//! tokio clock (R3-1: never `SystemTime::now`).
//!
//! # Deterministic virtual time
//!
//! Every test runs under `#[tokio::test(start_paused = true)]`, but the paused clock is
//! **frozen**, not auto-advanced: the harness keeps one `spawn_blocking` task alive, which
//! makes tokio inhibit auto-advance (otherwise the current-thread scheduler advances the
//! clock to the next timer whenever an I/O wake-up arrives during a park, and a 5 s head
//! timer would fire in the middle of a request). Time moves only through
//! [`Harness::advance_ms`] (`tokio::time::advance`), so "no event for 14 s" and "a
//! keep-alive at 15 s" are exact. I/O is real and takes no virtual time. As a last resort
//! against a server that never answers, the inhibitor gives up after
//! [`WALL_CLOCK_FALLBACK`] of real time, auto-advance resumes and the virtual
//! [`IO_BOUND`] timeouts fail the test cleanly.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::future::Future;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{oneshot, watch};
use tokio::task::{yield_now, JoinHandle};
use tokio::time::{timeout, Instant};
use tracing_subscriber::fmt::MakeWriter;

use soos_remote::config::{RemoteConfig, TailscaleLogin};
use soos_remote::logind::{SessionSource, SourceError};
use soos_remote::server::{serve, ServeError, ServerState};
use soos_remote::session::SessionProps;
use soos_remote::{
    DEFAULT_POLL_INTERVAL_MS, LOCK_FLOW_DEADLINE_MS, MAX_CONNECTIONS, MAX_HEADERS, MAX_PATH_LEN,
    MAX_REQUEST_HEAD_BYTES, MAX_SSE_STREAMS, MAX_SSE_STREAM_MS, MIN_LOCK_INTERVAL_MS,
    MIN_UNLOCK_INTERVAL_MS, REQUEST_HEAD_TIMEOUT_MS, SNAPSHOT_DEADLINE_MS, SSE_KEEPALIVE_MS,
    UNLOCK_FLOW_DEADLINE_MS,
};

const HOST: &str = "pc.tail1234.ts.net";
const LOGIN: &str = "owner@example.com";
const UID: u32 = 1000;
const SESSION_ID: &str = "c0ffee42";
const BASE_UNIX_MS: u64 = 1_700_000_000_000;
/// Virtual bound of a client I/O step; it can only fire once auto-advance resumes.
const IO_BOUND: Duration = Duration::from_secs(120);
/// Real time after which the frozen clock is released (a stalled server, not a slow CI).
const WALL_CLOCK_FALLBACK: Duration = Duration::from_secs(20);
/// Scheduler rounds granted to the server between two client observations.
const SETTLE_ROUNDS: usize = 8;
/// Scheduler rounds granted before declaring "no event will come without time passing".
const POLL_ROUNDS: usize = 64;
/// Virtual step of [`SseClient::await_event`].
const STEP_MS: u64 = 250;
const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self'; \
                   connect-src 'self'; manifest-src 'self'; base-uri 'none'; \
                   form-action 'none'; frame-ancestors 'none'";

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

/// Lets every runnable task (server included) run and the I/O driver deliver events,
/// without moving the clock.
async fn settle() {
    for _ in 0..SETTLE_ROUNDS {
        yield_now().await;
    }
}

// ---------------------------------------------------------------------------------------
// Test doubles
// ---------------------------------------------------------------------------------------

/// Unix clock bound to the paused tokio clock plus a settable offset.
struct TestClock {
    start: Instant,
    offset_ms: AtomicI64,
}

impl TestClock {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            start: Instant::now(),
            offset_ms: AtomicI64::new(0),
        })
    }

    fn now_ms(&self) -> u64 {
        let elapsed = self.start.elapsed().as_millis() as i64;
        (BASE_UNIX_MS as i64 + elapsed + self.offset_ms.load(Ordering::SeqCst)) as u64
    }

    fn shift_ms(&self, delta: i64) {
        self.offset_ms.fetch_add(delta, Ordering::SeqCst);
    }
}

fn session(locked: bool) -> SessionProps {
    SessionProps {
        id: SESSION_ID.to_string(),
        uid: Some(UID),
        active: true,
        remote: Some(false),
        seat: Some("seat0".to_string()),
        class: Some("user".to_string()),
        locked,
        idle: false,
        idle_since_unix_s: None,
    }
}

struct MockInner {
    answer: Mutex<Result<Vec<SessionProps>, SourceError>>,
    lock_answer: Mutex<Result<(), SourceError>>,
    unlock_answer: Mutex<Result<(), SourceError>>,
    hold_remaining: AtomicUsize,
    hold_lock_remaining: AtomicUsize,
    hold_unlock_remaining: AtomicUsize,
    release: watch::Sender<u64>,
    started: watch::Sender<u64>,
    lock_ids: Mutex<Vec<String>>,
    unlock_ids: Mutex<Vec<String>>,
    uids: Mutex<Vec<u32>>,
}

/// Scripted logind: a settable `own_sessions` answer (snapshotted when a call starts), a
/// settable `lock_session` answer, "hold the next N calls" gates, and spies.
#[derive(Clone)]
struct MockSource(Arc<MockInner>);

impl MockSource {
    fn unlocked() -> Self {
        Self(Arc::new(MockInner {
            answer: Mutex::new(Ok(vec![session(false)])),
            lock_answer: Mutex::new(Ok(())),
            unlock_answer: Mutex::new(Ok(())),
            hold_remaining: AtomicUsize::new(0),
            hold_lock_remaining: AtomicUsize::new(0),
            hold_unlock_remaining: AtomicUsize::new(0),
            release: watch::channel(0).0,
            started: watch::channel(0).0,
            lock_ids: Mutex::new(Vec::new()),
            unlock_ids: Mutex::new(Vec::new()),
            uids: Mutex::new(Vec::new()),
        }))
    }

    fn set_sessions(&self, sessions: Vec<SessionProps>) {
        *self.0.answer.lock().unwrap() = Ok(sessions);
    }

    fn set_unlocked(&self) {
        self.set_sessions(vec![session(false)]);
    }

    fn set_locked(&self) {
        self.set_sessions(vec![session(true)]);
    }

    fn set_no_session(&self) {
        self.set_sessions(Vec::new());
    }

    fn set_error(&self, error: SourceError) {
        *self.0.answer.lock().unwrap() = Err(error);
    }

    fn set_lock_result(&self, result: Result<(), SourceError>) {
        *self.0.lock_answer.lock().unwrap() = result;
    }

    fn set_unlock_result(&self, result: Result<(), SourceError>) {
        *self.0.unlock_answer.lock().unwrap() = result;
    }

    /// The next `n` `unlock_session` calls block until [`Self::release`].
    fn hold_unlock_next(&self, n: usize) {
        self.0.hold_unlock_remaining.store(n, Ordering::SeqCst);
    }

    fn unlock_ids(&self) -> Vec<String> {
        self.0.unlock_ids.lock().unwrap().clone()
    }

    /// The next `n` `own_sessions` calls block (after snapshotting the answer) until
    /// [`Self::release`].
    fn hold_next(&self, n: usize) {
        self.0.hold_remaining.store(n, Ordering::SeqCst);
    }

    /// The next `n` `lock_session` calls block until [`Self::release`].
    fn hold_lock_next(&self, n: usize) {
        self.0.hold_lock_remaining.store(n, Ordering::SeqCst);
    }

    fn release(&self) {
        self.0.release.send_modify(|g| *g += 1);
    }

    /// Number of `own_sessions` calls started so far.
    fn reads(&self) -> u64 {
        *self.0.started.borrow()
    }

    async fn wait_reads_at_least(&self, n: u64) {
        let mut rx = self.0.started.subscribe();
        timeout(IO_BOUND, rx.wait_for(|c| *c >= n))
            .await
            .expect("a read must start within the bound")
            .expect("mock alive");
    }

    fn lock_ids(&self) -> Vec<String> {
        self.0.lock_ids.lock().unwrap().clone()
    }

    fn uids(&self) -> Vec<u32> {
        self.0.uids.lock().unwrap().clone()
    }

    fn take_hold(counter: &AtomicUsize) -> bool {
        counter
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
    }
}

impl SessionSource for MockSource {
    fn own_sessions(
        &self,
        uid: u32,
    ) -> impl Future<Output = Result<Vec<SessionProps>, SourceError>> + Send {
        let inner = Arc::clone(&self.0);
        async move {
            inner.uids.lock().unwrap().push(uid);
            // The answer is captured when the read starts (a slow read reports the state
            // it observed at its start).
            let answer = inner.answer.lock().unwrap().clone();
            let generation = *inner.release.borrow();
            let held = Self::take_hold(&inner.hold_remaining);
            inner.started.send_modify(|c| *c += 1);
            if held {
                let mut rx = inner.release.subscribe();
                rx.wait_for(|g| *g > generation).await.ok();
            }
            answer
        }
    }

    fn lock_session(&self, id: &str) -> impl Future<Output = Result<(), SourceError>> + Send {
        let inner = Arc::clone(&self.0);
        let id = id.to_string();
        async move {
            inner.lock_ids.lock().unwrap().push(id);
            let generation = *inner.release.borrow();
            if Self::take_hold(&inner.hold_lock_remaining) {
                let mut rx = inner.release.subscribe();
                rx.wait_for(|g| *g > generation).await.ok();
            }
            inner.lock_answer.lock().unwrap().clone()
        }
    }

    fn unlock_session(&self, id: &str) -> impl Future<Output = Result<(), SourceError>> + Send {
        let inner = Arc::clone(&self.0);
        let id = id.to_string();
        async move {
            inner.unlock_ids.lock().unwrap().push(id);
            let generation = *inner.release.borrow();
            if Self::take_hold(&inner.hold_unlock_remaining) {
                let mut rx = inner.release.subscribe();
                rx.wait_for(|g| *g > generation).await.ok();
            }
            inner.unlock_answer.lock().unwrap().clone()
        }
    }
}

/// Captures every `tracing` line emitted on the test thread.
#[derive(Clone, Default)]
struct LogCapture(Arc<Mutex<Vec<u8>>>);

impl LogCapture {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

struct LogWriter(Arc<Mutex<Vec<u8>>>);

impl Write for LogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for LogCapture {
    type Writer = LogWriter;

    fn make_writer(&'a self) -> Self::Writer {
        LogWriter(Arc::clone(&self.0))
    }
}

// ---------------------------------------------------------------------------------------
// HTTP client helpers
// ---------------------------------------------------------------------------------------

struct HttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl std::fmt::Debug for HttpResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "HttpResponse({}, {} headers, {} body bytes)",
            self.status,
            self.headers.len(),
            self.body.len()
        )
    }
}

impl HttpResponse {
    fn header(&self, name: &str) -> Option<&str> {
        let values = self.headers_named(name);
        assert!(values.len() <= 1, "header {name} repeated: {values:?}");
        values.first().copied()
    }

    fn headers_named(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| {
            panic!(
                "body is JSON: {e}: {:?}",
                String::from_utf8_lossy(&self.body)
            )
        })
    }

    fn result(&self) -> String {
        self.json()["result"]
            .as_str()
            .expect("result field")
            .to_string()
    }

    fn assert_mandatory_headers(&self) {
        let status = self.status;
        assert_eq!(
            self.header("cache-control"),
            Some("no-store"),
            "status {status}"
        );
        assert_eq!(
            self.header("content-security-policy"),
            Some(CSP),
            "status {status}"
        );
        assert_eq!(
            self.header("x-content-type-options"),
            Some("nosniff"),
            "status {status}"
        );
        assert_eq!(
            self.header("referrer-policy"),
            Some("no-referrer"),
            "status {status}"
        );
        assert_eq!(
            self.header("x-frame-options"),
            Some("DENY"),
            "status {status}"
        );
        assert_eq!(self.header("connection"), Some("close"), "status {status}");
    }

    fn assert_json_body(&self) {
        assert_eq!(self.header("content-type"), Some("application/json"));
        assert_eq!(
            self.header("content-length"),
            Some(self.body.len().to_string().as_str())
        );
    }
}

/// Splits `bytes` at the first blank line into a parsed head and the body offset.
fn parse_head(bytes: &[u8]) -> Option<(HttpResponse, usize)> {
    let end = bytes.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&bytes[..end]).expect("ASCII head");
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap();
    assert!(status_line.starts_with("HTTP/1.1 "), "{status_line:?}");
    let status: u16 = status_line[9..12].parse().expect("status code");
    let headers = lines
        .map(|l| {
            let (name, value) = l.split_once(':').unwrap_or_else(|| panic!("header {l:?}"));
            (name.trim().to_ascii_lowercase(), value.trim().to_string())
        })
        .collect();
    Some((
        HttpResponse {
            status,
            headers,
            body: Vec::new(),
        },
        end + 4,
    ))
}

fn parse_response(bytes: &[u8]) -> HttpResponse {
    let (mut response, body_start) = parse_head(bytes)
        .unwrap_or_else(|| panic!("no head terminator in {:?}", String::from_utf8_lossy(bytes)));
    response.body = bytes[body_start..].to_vec();
    response
}

fn raw_request(method: &str, target: &str, headers: &[(&str, &str)]) -> Vec<u8> {
    let mut out = format!("{method} {target} HTTP/1.1\r\n");
    for (name, value) in headers {
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    out.push_str("\r\n");
    out.into_bytes()
}

fn with_identity<'a>(extra: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
    let mut headers = vec![("Host", HOST), ("Tailscale-User-Login", LOGIN)];
    headers.extend_from_slice(extra);
    headers
}

/// What the client observed on a connection that may or may not answer.
#[derive(Debug)]
enum Outcome {
    Response(HttpResponse),
    /// Closed (EOF or reset) with no bytes.
    Closed,
    /// Nothing within the bound.
    Silent,
}

async fn connect(path: &Path) -> UnixStream {
    timeout(IO_BOUND, UnixStream::connect(path))
        .await
        .expect("connect within the bound")
        .expect("connect")
}

/// One request: connect, write `bytes`, read to EOF. Real I/O; no virtual time passes.
async fn exchange(path: &Path, bytes: &[u8]) -> Outcome {
    let mut stream = connect(path).await;
    timeout(IO_BOUND, stream.write_all(bytes))
        .await
        .expect("write within the bound")
        .expect("write");
    let mut buf = Vec::new();
    match timeout(IO_BOUND, stream.read_to_end(&mut buf)).await {
        Err(_) => Outcome::Silent,
        Ok(Ok(0)) => Outcome::Closed,
        Ok(Ok(_)) => Outcome::Response(parse_response(&buf)),
        Ok(Err(_)) if buf.is_empty() => Outcome::Closed,
        Ok(Err(e)) => panic!("read failed after {} bytes: {e}", buf.len()),
    }
}

/// Runs `exchange` as a task so the test can move the clock while the request is pending.
fn spawn_exchange(path: &Path, bytes: Vec<u8>) -> JoinHandle<Outcome> {
    let path = path.to_path_buf();
    tokio::spawn(async move { exchange(&path, &bytes).await })
}

/// Waits (scheduler rounds only, no clock movement) until the task is finished.
async fn finished_soon<T>(task: &JoinHandle<T>) -> bool {
    for _ in 0..POLL_ROUNDS {
        if task.is_finished() {
            return true;
        }
        yield_now().await;
    }
    task.is_finished()
}

enum Polled {
    Status(Value),
    Eof,
    Pending,
}

struct SseClient {
    stream: UnixStream,
    buf: Vec<u8>,
    head: HttpResponse,
}

impl SseClient {
    /// Non-blocking: a buffered or immediately readable event, EOF, or nothing yet.
    fn poll_event(&mut self) -> Polled {
        loop {
            if let Some(pos) = self.buf.windows(2).position(|w| w == b"\n\n") {
                let frame: Vec<u8> = self.buf.drain(..pos + 2).collect();
                return Polled::Status(parse_event(&frame));
            }
            let mut chunk = [0u8; 4096];
            match self.stream.try_read(&mut chunk) {
                Ok(0) => return Polled::Eof,
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Polled::Pending,
                Err(_) => return Polled::Eof,
            }
        }
    }

    /// An event that needs no time to pass (the server only needs to run).
    async fn expect_event_now(&mut self) -> Value {
        for _ in 0..POLL_ROUNDS {
            match self.poll_event() {
                Polled::Status(v) => return v,
                Polled::Eof => panic!("stream ended while an event was expected"),
                Polled::Pending => yield_now().await,
            }
        }
        panic!("no event arrived without time passing");
    }

    async fn expect_state_now(&mut self, state: &str) -> Value {
        let event = self.expect_event_now().await;
        assert_eq!(event["state"], state, "{event}");
        event
    }

    /// No event arrives without time passing.
    async fn expect_silence_now(&mut self) {
        settle().await;
        match self.poll_event() {
            Polled::Pending => {}
            Polled::Status(v) => panic!("unexpected event {v}"),
            Polled::Eof => panic!("unexpected end of stream"),
        }
    }

    async fn expect_eof_now(&mut self) {
        for _ in 0..POLL_ROUNDS {
            match self.poll_event() {
                Polled::Eof => return,
                Polled::Status(v) => panic!("unexpected event {v} while EOF was expected"),
                Polled::Pending => yield_now().await,
            }
        }
        panic!("stream still open");
    }

    /// Moves the clock in [`STEP_MS`] steps until an event arrives or `max_ms` elapsed;
    /// returns the event and the virtual time consumed.
    async fn await_event(&mut self, h: &Harness, max_ms: u64) -> Option<(Value, u64)> {
        let mut elapsed = 0;
        loop {
            for _ in 0..SETTLE_ROUNDS {
                match self.poll_event() {
                    Polled::Status(v) => return Some((v, elapsed)),
                    Polled::Eof => {
                        panic!("stream ended after {elapsed} ms while waiting for an event")
                    }
                    Polled::Pending => yield_now().await,
                }
            }
            if elapsed >= max_ms {
                return None;
            }
            h.advance_ms(STEP_MS).await;
            elapsed += STEP_MS;
        }
    }

    async fn expect_state_within(&mut self, h: &Harness, state: &str, max_ms: u64) -> Value {
        let (event, elapsed) = self
            .await_event(h, max_ms)
            .await
            .unwrap_or_else(|| panic!("no event within {max_ms} ms (expected {state})"));
        assert_eq!(event["state"], state, "{event} after {elapsed} ms");
        event
    }

    /// Moves the clock by `total_ms` in [`STEP_MS`] steps and asserts no event arrives.
    async fn expect_silence_for(&mut self, h: &Harness, total_ms: u64) {
        if let Some((event, elapsed)) = self.await_event(h, total_ms.saturating_sub(STEP_MS)).await
        {
            panic!(
                "unexpected event {event} after {elapsed} ms (silence expected for {total_ms} ms)"
            );
        }
    }
}

fn parse_event(frame: &[u8]) -> Value {
    let text = std::str::from_utf8(frame).expect("UTF-8 event");
    let mut data = None;
    let mut event_name = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("data: ") {
            assert!(data.is_none(), "one data line per event: {text:?}");
            data = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("event: ") {
            event_name = Some(rest.to_string());
        } else if !line.is_empty() && !line.starts_with(':') {
            panic!("unexpected SSE line {line:?} in {text:?}");
        }
    }
    assert_eq!(event_name.as_deref(), Some("status"), "{text:?}");
    let data = data.unwrap_or_else(|| panic!("event without data: {text:?}"));
    serde_json::from_str(&data).unwrap_or_else(|e| panic!("event data is JSON: {e}: {data:?}"))
}

fn checked(event: &Value) -> u64 {
    event["checked_unix_ms"].as_u64().expect("checked_unix_ms")
}

fn assert_status_shape(json: &Value) {
    let mut keys: Vec<&str> = json
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "active",
            "checked_unix_ms",
            "idle",
            "idle_since_unix_s",
            "state"
        ]
    );
    let text = json.to_string();
    for forbidden in [SESSION_ID, "seat0", "1000", LOGIN, "alice"] {
        assert!(!text.contains(forbidden), "{text} leaks {forbidden}");
    }
}

// ---------------------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------------------

/// Keeps a blocking task alive so tokio never auto-advances the paused clock (see the
/// module documentation); released on drop or after [`WALL_CLOCK_FALLBACK`].
struct FrozenClock {
    release: Option<mpsc::Sender<()>>,
}

impl FrozenClock {
    fn hold() -> Self {
        let (tx, rx) = mpsc::channel::<()>();
        drop(tokio::task::spawn_blocking(move || {
            let _ = rx.recv_timeout(WALL_CLOCK_FALLBACK);
        }));
        Self { release: Some(tx) }
    }
}

impl Drop for FrozenClock {
    fn drop(&mut self) {
        if let Some(tx) = self.release.take() {
            let _ = tx.send(());
        }
    }
}

#[derive(Default)]
struct Options {
    poll_interval_ms: Option<u64>,
    allowed_hosts: Vec<String>,
    seq_start: Option<u64>,
    allow_unlock: bool,
}

struct Harness {
    _dir: TempDir,
    path: PathBuf,
    source: MockSource,
    clock: Arc<TestClock>,
    shutdown: Option<oneshot::Sender<()>>,
    server: Option<JoinHandle<Result<(), ServeError>>>,
    _frozen: FrozenClock,
}

impl Harness {
    async fn start() -> Self {
        Self::start_with(Options::default()).await
    }

    async fn start_with(options: Options) -> Self {
        let frozen = FrozenClock::hold();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("remote.sock");
        let config = RemoteConfig {
            allowed_logins: vec![TailscaleLogin::parse(LOGIN).unwrap()],
            socket_path: path.clone(),
            poll_interval_ms: options.poll_interval_ms.unwrap_or(DEFAULT_POLL_INTERVAL_MS),
            allowed_hosts: options.allowed_hosts,
            allow_unlock: options.allow_unlock,
        };
        let source = MockSource::unlocked();
        let clock = TestClock::new();
        let clock_fn = Arc::clone(&clock);
        let mut state = ServerState::new(config, UID, source.clone())
            .with_unix_clock(Arc::new(move || clock_fn.now_ms()));
        if let Some(seq) = options.seq_start {
            state = state.with_seq_start(seq);
        }
        let listener = UnixListener::bind(&path).unwrap();
        let (tx, rx) = oneshot::channel::<()>();
        let server = tokio::spawn(serve(listener, Arc::new(state), async move {
            let _ = rx.await;
        }));
        settle().await;
        Self {
            _dir: dir,
            path,
            source,
            clock,
            shutdown: Some(tx),
            server: Some(server),
            _frozen: frozen,
        }
    }

    /// Moves the virtual clock by `n` ms in one jump, then lets every task run.
    async fn advance_ms(&self, n: u64) {
        tokio::time::advance(ms(n)).await;
        settle().await;
    }

    /// Moves the virtual clock by `total` ms in `step` ms steps (periodic timers fire at
    /// every step, as they would in real time).
    async fn step_ms(&self, step: u64, total: u64) {
        let mut done = 0;
        while done < total {
            let n = step.min(total - done);
            self.advance_ms(n).await;
            done += n;
        }
    }

    async fn raw(&self, bytes: &[u8]) -> HttpResponse {
        match exchange(&self.path, bytes).await {
            Outcome::Response(r) => r,
            Outcome::Closed => panic!("server closed the connection without a response"),
            Outcome::Silent => panic!("no response within {IO_BOUND:?}"),
        }
    }

    async fn request(&self, method: &str, target: &str, extra: &[(&str, &str)]) -> HttpResponse {
        self.raw(&raw_request(method, target, &with_identity(extra)))
            .await
    }

    async fn get(&self, target: &str) -> HttpResponse {
        self.request("GET", target, &[]).await
    }

    async fn lock(&self, extra: &[(&str, &str)]) -> HttpResponse {
        let mut headers = vec![("X-Soos-Action", "lock")];
        headers.extend_from_slice(extra);
        self.request("POST", "/api/lock", &headers).await
    }

    async fn unlock(&self, extra: &[(&str, &str)]) -> HttpResponse {
        let mut headers = vec![("X-Soos-Action", "unlock")];
        headers.extend_from_slice(extra);
        self.request("POST", "/api/unlock", &headers).await
    }

    async fn start_unlock_enabled() -> Self {
        Self::start_with(Options {
            allow_unlock: true,
            ..Options::default()
        })
        .await
    }

    async fn try_open_stream(&self, headers: &[(&str, &str)]) -> Result<SseClient, Outcome> {
        let mut stream = connect(&self.path).await;
        timeout(
            IO_BOUND,
            stream.write_all(&raw_request("GET", "/api/events", headers)),
        )
        .await
        .expect("write within the bound")
        .expect("write");
        let mut buf = Vec::new();
        let deadline = Instant::now() + IO_BOUND;
        loop {
            if let Some((head, body_start)) = parse_head(&buf) {
                if head.status == 200 {
                    let rest = buf[body_start..].to_vec();
                    return Ok(SseClient {
                        stream,
                        buf: rest,
                        head,
                    });
                }
                let mut tail = Vec::new();
                let _ = timeout(IO_BOUND, stream.read_to_end(&mut tail)).await;
                buf.extend_from_slice(&tail);
                return Err(Outcome::Response(parse_response(&buf)));
            }
            let mut chunk = [0u8; 4096];
            match tokio::time::timeout_at(deadline, stream.read(&mut chunk)).await {
                Err(_) => return Err(Outcome::Silent),
                Ok(Ok(0)) | Ok(Err(_)) => {
                    assert!(
                        buf.is_empty(),
                        "partial head {:?}",
                        String::from_utf8_lossy(&buf)
                    );
                    return Err(Outcome::Closed);
                }
                Ok(Ok(n)) => buf.extend_from_slice(&chunk[..n]),
            }
        }
    }

    async fn open_stream(&self) -> SseClient {
        match self.try_open_stream(&with_identity(&[])).await {
            Ok(client) => client,
            Err(Outcome::Response(r)) => panic!("stream refused with status {}", r.status),
            Err(Outcome::Closed) => panic!("stream closed without a head"),
            Err(Outcome::Silent) => panic!("no stream head within the bound"),
        }
    }

    async fn shutdown(mut self) -> Result<(), ServeError> {
        let tx = self.shutdown.take().unwrap();
        let _ = tx.send(());
        let server = self.server.take().unwrap();
        timeout(IO_BOUND, server)
            .await
            .expect("serve returns after shutdown")
            .expect("serve task not panicked")
    }

    async fn await_server(&mut self) -> Result<(), ServeError> {
        let server = self.server.take().unwrap();
        timeout(IO_BOUND, server)
            .await
            .expect("serve returns within the bound")
            .expect("serve task not panicked")
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        if let Some(server) = self.server.take() {
            server.abort();
        }
    }
}

fn assert_sse_head(head: &HttpResponse) {
    assert_eq!(head.status, 200);
    assert_eq!(head.header("content-type"), Some("text/event-stream"));
    assert!(
        head.header("content-length").is_none(),
        "no Content-Length on a stream"
    );
    head.assert_mandatory_headers();
}

// ---------------------------------------------------------------------------------------
// Host, identity, HTTP errors, headers
// ---------------------------------------------------------------------------------------

/// D5a: the Host is checked before identity and routing; every refusal is `421`.
#[tokio::test(start_paused = true)]
async fn test_rmc_host_is_checked_before_identity_and_routing() {
    let h = Harness::start().await;
    let no_host = h
        .raw(&raw_request(
            "GET",
            "/api/status",
            &[("Tailscale-User-Login", LOGIN)],
        ))
        .await;
    assert_eq!(no_host.status, 421);
    no_host.assert_mandatory_headers();
    for bad in [
        "evil.example.com",
        "pc.tail1234.ts.net:8443",
        "100.64.0.1",
        "ts.net",
    ] {
        let r = h
            .raw(&raw_request(
                "GET",
                "/api/status",
                &[("Host", bad), ("Tailscale-User-Login", LOGIN)],
            ))
            .await;
        assert_eq!(r.status, 421, "{bad}");
    }
    let repeated = h
        .raw(&raw_request(
            "GET",
            "/api/status",
            &[
                ("Host", HOST),
                ("Host", HOST),
                ("Tailscale-User-Login", LOGIN),
            ],
        ))
        .await;
    assert_eq!(repeated.status, 421);
    // Before identity (no login) and before routing (unknown path).
    let r = h
        .raw(&raw_request(
            "GET",
            "/nope",
            &[("Host", "evil.example.com")],
        ))
        .await;
    assert_eq!(r.status, 421);
    // Normalised hosts pass.
    let r = h
        .raw(&raw_request(
            "GET",
            "/api/status",
            &[
                ("Host", "PC.Tail1234.TS.NET:443"),
                ("Tailscale-User-Login", LOGIN),
            ],
        ))
        .await;
    assert_eq!(r.status, 200);
    assert_eq!(
        h.source.reads(),
        1,
        "only the accepted request reached logind"
    );
}

/// D5a / RC-2: `allowed_hosts` restricts the Host end to end.
#[tokio::test(start_paused = true)]
async fn test_rmc_allowed_hosts_config_is_enforced_end_to_end() {
    let h = Harness::start_with(Options {
        allowed_hosts: vec!["mypc.tail1234.ts.net".to_string()],
        ..Options::default()
    })
    .await;
    let r = h.get("/api/status").await;
    assert_eq!(
        r.status, 421,
        "another *.ts.net name is refused once a list exists"
    );
    let r = h
        .raw(&raw_request(
            "GET",
            "/api/status",
            &[
                ("Host", "MYPC.tail1234.ts.net:443"),
                ("Tailscale-User-Login", LOGIN),
            ],
        ))
        .await;
    assert_eq!(r.status, 200);
}

/// RC-2 / §4: every route, assets included, is `403 {"result":"forbidden"}` without exactly
/// one allowlisted identity, and the refusal comes before routing (no 404/405 leak).
#[tokio::test(start_paused = true)]
async fn test_rmc_identity_is_required_before_routing() {
    let h = Harness::start().await;
    for (method, target) in [
        ("GET", "/"),
        ("GET", "/app.js"),
        ("GET", "/api/status"),
        ("GET", "/api/events"),
        ("POST", "/api/lock"),
        ("GET", "/nope"),
        ("POST", "/api/status"),
        ("HEAD", "/api/status"),
    ] {
        let r = h
            .raw(&raw_request(
                method,
                target,
                &[("Host", HOST), ("X-Soos-Action", "lock")],
            ))
            .await;
        assert_eq!(r.status, 403, "{method} {target} without identity");
        r.assert_mandatory_headers();
        if method != "HEAD" {
            r.assert_json_body();
            assert_eq!(r.result(), "forbidden", "{method} {target}");
        }
    }
    for headers in [
        vec![("Host", HOST), ("Tailscale-User-Login", "evil@example.org")],
        vec![
            ("Host", HOST),
            ("Tailscale-User-Login", LOGIN),
            ("Tailscale-User-Login", LOGIN),
        ],
        vec![
            ("Host", HOST),
            ("Tailscale-User-Login", "owner@example.com.evil"),
        ],
        vec![("Host", HOST), ("Tailscale-User-Login", "")],
        vec![("Host", HOST), ("Tailscale-User-Name", LOGIN)],
    ] {
        let r = h.raw(&raw_request("GET", "/api/status", &headers)).await;
        assert_eq!(r.status, 403, "{headers:?}");
        assert_eq!(r.result(), "forbidden");
    }
    assert_eq!(h.source.reads(), 0, "no refused request reaches logind");
    assert!(h.source.lock_ids().is_empty());
    let r = h
        .raw(&raw_request(
            "GET",
            "/api/status",
            &[
                ("Host", HOST),
                ("TAILSCALE-USER-LOGIN", " Owner@Example.COM "),
            ],
        ))
        .await;
    assert_eq!(
        r.status, 200,
        "case-insensitive name and value, OWS trimmed"
    );
}

/// §4: parse errors map to their statuses before any Host or identity check.
#[tokio::test(start_paused = true)]
async fn test_rmc_http_errors_map_to_statuses() {
    let h = Harness::start().await;
    let r = h
        .raw(b"GET /api/status HTTP/1.0\r\nHost: pc.tail1234.ts.net\r\n\r\n")
        .await;
    assert_eq!(r.status, 400);
    r.assert_mandatory_headers();
    let r = h.raw(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n").await;
    assert_eq!(r.status, 400);
    let r = h.raw(b"\x16\x03\x01\x02\x00garbage\r\n\r\n").await;
    assert_eq!(r.status, 400);
    let r = h
        .raw(&raw_request(
            "POST",
            "/api/lock",
            &[("Transfer-Encoding", "chunked")],
        ))
        .await;
    assert_eq!(r.status, 400);
    let mut with_body = raw_request(
        "POST",
        "/api/lock",
        &with_identity(&[("Content-Length", "5")]),
    );
    with_body.extend_from_slice(b"hello");
    let r = h.raw(&with_body).await;
    assert_eq!(r.status, 413);
    let names: Vec<String> = (0..=MAX_HEADERS).map(|i| format!("X-H{i}")).collect();
    let many: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), "v")).collect();
    let r = h.raw(&raw_request("GET", "/api/status", &many)).await;
    assert_eq!(r.status, 431);
    let padding = "p".repeat(MAX_REQUEST_HEAD_BYTES);
    let r = h
        .raw(&raw_request("GET", "/", &[("X-Pad", &padding)]))
        .await;
    assert_eq!(r.status, 431);
    let long_path = format!("/{}", "p".repeat(MAX_PATH_LEN));
    let r = h.raw(&raw_request("GET", &long_path, &[])).await;
    assert_eq!(r.status, 414);
    assert_eq!(h.source.reads(), 0);
    assert!(h.source.lock_ids().is_empty());
}

/// §2.8: every response, error responses included, carries the mandatory headers and a
/// `Content-Length`, and the connection is closed after it.
#[tokio::test(start_paused = true)]
async fn test_rmc_every_response_carries_the_mandatory_headers() {
    let h = Harness::start().await;
    let responses = vec![
        (200, h.get("/api/status").await),
        (200, h.get("/").await),
        (
            403,
            h.raw(&raw_request("GET", "/api/status", &[("Host", HOST)]))
                .await,
        ),
        (404, h.get("/nope").await),
        (405, h.request("POST", "/api/status", &[]).await),
        (
            421,
            h.raw(&raw_request("GET", "/", &[("Host", "evil.example.com")]))
                .await,
        ),
        (400, h.raw(b"GET / HTTP/1.0\r\n\r\n").await),
        (
            414,
            h.raw(&raw_request(
                "GET",
                &format!("/{}", "p".repeat(MAX_PATH_LEN)),
                &[],
            ))
            .await,
        ),
        (
            413,
            h.request(
                "POST",
                "/api/lock",
                &[("Content-Length", "1"), ("X-Soos-Action", "lock")],
            )
            .await,
        ),
        (202, h.lock(&[]).await),
        (429, h.lock(&[]).await),
    ];
    for (expected, response) in responses {
        assert_eq!(response.status, expected);
        response.assert_mandatory_headers();
        assert_eq!(
            response.header("content-length"),
            Some(response.body.len().to_string().as_str()),
            "status {expected}"
        );
        assert!(
            response.header("content-type").is_some(),
            "status {expected}"
        );
    }
}

// ---------------------------------------------------------------------------------------
// /api/status, routing, assets
// ---------------------------------------------------------------------------------------

/// D8 / RC-3 / RC-5: `/api/status` is one fresh `own_sessions(uid)` read; a failed read is
/// `unavailable`, never `unlocked`; the body has the documented shape and no identity.
#[tokio::test(start_paused = true)]
async fn test_rmc_status_is_a_fresh_read_and_fails_safe() {
    let h = Harness::start().await;
    let r = h.get("/api/status").await;
    assert_eq!(r.status, 200);
    r.assert_json_body();
    let json = r.json();
    assert_status_shape(&json);
    assert_eq!(
        json,
        serde_json::json!({
            "state": "unlocked",
            "active": true,
            "idle": false,
            "idle_since_unix_s": null,
            "checked_unix_ms": h.clock.now_ms()
        })
    );
    assert_eq!(h.source.reads(), 1);
    assert_eq!(h.source.uids(), vec![UID]);

    h.clock.shift_ms(5_000);
    h.source.set_locked();
    let json = h.get("/api/status").await.json();
    assert_eq!(json["state"], "locked");
    assert_eq!(checked(&json), h.clock.now_ms());
    assert_eq!(h.source.reads(), 2, "exactly one read per request");

    let mut idle = session(false);
    idle.idle = true;
    idle.idle_since_unix_s = Some(1_700_000_000);
    idle.active = false;
    h.source.set_sessions(vec![idle]);
    let json = h.get("/api/status").await.json();
    assert_eq!(json["state"], "unlocked");
    assert_eq!(json["active"], false);
    assert_eq!(json["idle"], true);
    assert_eq!(json["idle_since_unix_s"], 1_700_000_000u64);
    assert_status_shape(&json);

    h.source.set_no_session();
    let json = h.get("/api/status").await.json();
    assert_eq!(json["state"], "no_session");
    assert_eq!(json["active"], false);
    assert_eq!(json["idle"], false);
    assert_eq!(json["idle_since_unix_s"], Value::Null);

    for error in [
        SourceError::Timeout,
        SourceError::BusUnavailable,
        SourceError::Call,
        SourceError::Malformed,
        SourceError::TooManySessions,
    ] {
        h.source.set_error(error.clone());
        let r = h.get("/api/status").await;
        assert_eq!(r.status, 200, "{error:?}");
        let json = r.json();
        assert_eq!(json["state"], "unavailable", "{error:?}");
        assert_eq!(json["active"], false);
        assert_eq!(json["idle"], false);
        assert_eq!(checked(&json), h.clock.now_ms());
    }
    assert_eq!(h.source.reads(), 9);
    assert!(h.source.uids().iter().all(|&u| u == UID));
}

/// §2.5: `HEAD` returns the same headers as `GET` with an empty body.
#[tokio::test(start_paused = true)]
async fn test_rmc_head_returns_headers_without_body() {
    let h = Harness::start().await;
    let get = h.get("/api/status").await;
    let head = h.request("HEAD", "/api/status", &[]).await;
    assert_eq!(head.status, 200);
    assert!(
        head.body.is_empty(),
        "{:?}",
        String::from_utf8_lossy(&head.body)
    );
    assert_eq!(head.header("content-type"), get.header("content-type"));
    assert_eq!(head.header("content-length"), get.header("content-length"));
    head.assert_mandatory_headers();
    let get = h.get("/app.js").await;
    let head = h.request("HEAD", "/app.js", &[]).await;
    assert_eq!(head.status, 200);
    assert!(head.body.is_empty());
    assert_eq!(head.header("content-length"), get.header("content-length"));
    assert_eq!(
        head.header("content-type"),
        Some("text/javascript; charset=utf-8")
    );
}

/// §2.5: unknown paths are `404`; known paths with a wrong method are `405` with `Allow`.
#[tokio::test(start_paused = true)]
async fn test_rmc_unknown_route_and_wrong_method() {
    let h = Harness::start().await;
    for target in [
        "/nope",
        "/api",
        "/api/status/",
        "/API/STATUS",
        "/index.html/",
        "/api/unlock/",
    ] {
        let r = h.get(target).await;
        assert_eq!(r.status, 404, "{target}");
        r.assert_mandatory_headers();
    }
    let r = h.request("DELETE", "/nope", &[]).await;
    assert_eq!(r.status, 404);
    for target in ["/api/status", "/api/events", "/", "/app.js"] {
        let r = h.request("POST", target, &[]).await;
        assert_eq!(r.status, 405, "{target}");
        assert_eq!(r.header("allow"), Some("GET, HEAD"), "{target}");
        r.assert_mandatory_headers();
    }
    let r = h.request("DELETE", "/api/status", &[]).await;
    assert_eq!(r.status, 405);
    for method in ["GET", "HEAD", "PUT"] {
        let r = h
            .request(method, "/api/lock", &[("X-Soos-Action", "lock")])
            .await;
        assert_eq!(r.status, 405, "{method}");
        assert_eq!(r.header("allow"), Some("POST"));
    }
    assert!(h.source.lock_ids().is_empty());
    assert_eq!(h.source.reads(), 0);
}

/// D11 / §2.5: every asset with its content type; `/` is `index.html`; the icon is a PNG.
#[tokio::test(start_paused = true)]
async fn test_rmc_assets_are_served_with_content_types() {
    let h = Harness::start().await;
    let index = h.get("/").await;
    assert_eq!(index.status, 200);
    assert_eq!(
        index.header("content-type"),
        Some("text/html; charset=utf-8")
    );
    assert!(!index.body.is_empty());
    let explicit = h.get("/index.html").await;
    assert_eq!(explicit.body, index.body);
    let html = String::from_utf8(index.body.clone()).unwrap();
    assert!(
        html.contains("app.js") && html.contains("style.css"),
        "{html}"
    );
    assert!(html.contains("manifest.webmanifest"), "{html}");
    assert!(html.contains("apple-touch-icon.png"), "{html}");
    for (target, content_type) in [
        ("/app.js", "text/javascript; charset=utf-8"),
        ("/style.css", "text/css; charset=utf-8"),
        ("/manifest.webmanifest", "application/manifest+json"),
        ("/icon.svg", "image/svg+xml"),
        ("/apple-touch-icon.png", "image/png"),
    ] {
        let r = h.get(target).await;
        assert_eq!(r.status, 200, "{target}");
        assert_eq!(r.header("content-type"), Some(content_type), "{target}");
        assert!(!r.body.is_empty(), "{target}");
        assert_eq!(
            r.header("content-length"),
            Some(r.body.len().to_string().as_str()),
            "{target}"
        );
        r.assert_mandatory_headers();
    }
    let png = h.get("/apple-touch-icon.png").await.body;
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    let js = String::from_utf8(h.get("/app.js").await.body).unwrap();
    assert!(
        js.contains("/api/events") && js.contains("/api/status") && js.contains("/api/lock"),
        "{js}"
    );
    assert!(js.contains("X-Soos-Action"), "{js}");
    assert!(
        !js.contains("innerHTML"),
        "server data goes through textContent only"
    );
    assert_eq!(h.source.reads(), 0, "assets never touch logind");
}

// ---------------------------------------------------------------------------------------
// /api/lock
// ---------------------------------------------------------------------------------------

/// D9 / §2.8: fresh snapshot → `LockSession(id)` → `202`; `429` within
/// `MIN_LOCK_INTERVAL_MS` of an accepted lock without a logind call; `409`/`503` do not
/// record the interval; a failed `LockSession` does.
#[tokio::test(start_paused = true)]
async fn test_rmc_lock_flow_and_rate_limit() {
    let h = Harness::start().await;
    let r = h.lock(&[]).await;
    assert_eq!(r.status, 202);
    r.assert_json_body();
    assert_eq!(r.result(), "lock_requested");
    assert_eq!(h.source.lock_ids(), vec![SESSION_ID.to_string()]);
    let reads_after_first = h.source.reads();
    assert_eq!(
        reads_after_first, 1,
        "one fresh snapshot before the lock call"
    );

    let r = h.lock(&[]).await;
    assert_eq!(r.status, 429);
    assert_eq!(r.result(), "rate_limited");
    assert_eq!(h.source.lock_ids().len(), 1);
    assert_eq!(
        h.source.reads(),
        reads_after_first,
        "a rate-limited lock never reads logind"
    );

    h.advance_ms(MIN_LOCK_INTERVAL_MS - 1).await;
    let r = h.lock(&[]).await;
    assert_eq!(r.status, 429, "still inside the interval");
    h.advance_ms(2).await;
    let r = h.lock(&[]).await;
    assert_eq!(r.status, 202);
    assert_eq!(h.source.lock_ids().len(), 2);

    // already_locked: no lock call, interval not recorded.
    h.advance_ms(MIN_LOCK_INTERVAL_MS + 1).await;
    h.source.set_locked();
    let r = h.lock(&[]).await;
    assert_eq!(r.status, 409);
    assert_eq!(r.result(), "already_locked");
    assert_eq!(h.source.lock_ids().len(), 2);
    h.source.set_unlocked();
    let r = h.lock(&[]).await;
    assert_eq!(r.status, 202, "a 409 recorded no interval");
    assert_eq!(h.source.lock_ids().len(), 3);

    // no_session.
    h.advance_ms(MIN_LOCK_INTERVAL_MS + 1).await;
    h.source.set_no_session();
    let r = h.lock(&[]).await;
    assert_eq!(r.status, 409);
    assert_eq!(r.result(), "no_session");
    let mut remote = session(false);
    remote.remote = Some(true);
    h.source.set_sessions(vec![remote]);
    let r = h.lock(&[]).await;
    assert_eq!(r.result(), "no_session", "a remote session is never locked");
    assert_eq!(h.source.lock_ids().len(), 3);
    h.source.set_unlocked();
    let r = h.lock(&[]).await;
    assert_eq!(r.status, 202);
    assert_eq!(h.source.lock_ids().len(), 4);

    // Snapshot failure: 503, no lock call, interval not recorded.
    h.advance_ms(MIN_LOCK_INTERVAL_MS + 1).await;
    h.source.set_error(SourceError::Timeout);
    let r = h.lock(&[]).await;
    assert_eq!(r.status, 503);
    assert_eq!(r.result(), "unavailable");
    assert_eq!(h.source.lock_ids().len(), 4);
    h.source.set_unlocked();
    let r = h.lock(&[]).await;
    assert_eq!(r.status, 202);
    assert_eq!(h.source.lock_ids().len(), 5);

    // LockSession failure: 503, the interval is recorded because the call was made.
    h.advance_ms(MIN_LOCK_INTERVAL_MS + 1).await;
    h.source.set_lock_result(Err(SourceError::Call));
    let r = h.lock(&[]).await;
    assert_eq!(r.status, 503);
    assert_eq!(r.result(), "unavailable");
    assert_eq!(
        h.source.lock_ids().len(),
        6,
        "exactly one attempt, no retry"
    );
    h.source.set_lock_result(Ok(()));
    let r = h.lock(&[]).await;
    assert_eq!(
        r.status, 429,
        "a failed LockSession still counts for the interval"
    );
    assert_eq!(h.source.lock_ids().len(), 6);
    assert!(h.source.lock_ids().iter().all(|id| id == SESSION_ID));
}

/// D4: the CSRF rules end to end; a refused lock never reaches logind.
#[tokio::test(start_paused = true)]
async fn test_rmc_lock_requires_csrf_headers() {
    let h = Harness::start().await;
    for extra in [
        vec![],
        vec![("X-Soos-Action", "unlock")],
        vec![("X-Soos-Action", "lock"), ("Sec-Fetch-Site", "cross-site")],
        vec![("X-Soos-Action", "lock"), ("Sec-Fetch-Site", "same-site")],
        vec![
            ("X-Soos-Action", "lock"),
            ("Origin", "https://other.tail1234.ts.net"),
        ],
        vec![
            ("X-Soos-Action", "lock"),
            ("Origin", "http://pc.tail1234.ts.net"),
        ],
        vec![("X-Soos-Action", "lock"), ("Origin", "null")],
    ] {
        let r = h.request("POST", "/api/lock", &extra).await;
        assert_eq!(r.status, 403, "{extra:?}");
        assert_eq!(r.result(), "forbidden", "{extra:?}");
    }
    assert!(h.source.lock_ids().is_empty());
    assert_eq!(
        h.source.reads(),
        0,
        "CSRF is checked before any logind read"
    );

    let r = h
        .lock(&[
            ("Sec-Fetch-Site", "same-origin"),
            ("Origin", "https://pc.tail1234.ts.net"),
        ])
        .await;
    assert_eq!(r.status, 202);
    h.advance_ms(MIN_LOCK_INTERVAL_MS + 1).await;
    let r = h
        .lock(&[("Origin", "https://PC.Tail1234.TS.NET:443")])
        .await;
    assert_eq!(
        r.status, 202,
        "the Origin is compared to the normalised host"
    );
    assert_eq!(h.source.lock_ids().len(), 2);
}

// ---------------------------------------------------------------------------------------
// /api/unlock (ADR 2026-10-06 "Remote Unlock in soos-remote", matrix RMC22–RMC25)
// ---------------------------------------------------------------------------------------

/// RMC22: unlock is opt-in. With the default configuration a fully valid unlock request is
/// `403 unlock_disabled` and never reaches logind.
#[tokio::test(start_paused = true)]
async fn test_rmc_unlock_disabled_by_default_never_reaches_logind() {
    let h = Harness::start().await;
    h.source.set_locked();
    let r = h.unlock(&[]).await;
    assert_eq!(r.status, 403);
    r.assert_mandatory_headers();
    r.assert_json_body();
    assert_eq!(r.result(), "unlock_disabled");
    assert!(h.source.unlock_ids().is_empty());
    assert!(h.source.lock_ids().is_empty());
    assert_eq!(h.source.reads(), 0, "a disabled unlock never reads logind");
}

/// RMC23: fresh snapshot → `UnlockSession(id)` → `202 unlock_requested`; `409
/// already_unlocked` / `409 no_session` / `503` without an unlock call and without recording
/// the interval; `429` within `MIN_UNLOCK_INTERVAL_MS`; a failed call counts; no retry.
#[tokio::test(start_paused = true)]
async fn test_rmc_unlock_flow_and_rate_limit() {
    let h = Harness::start_unlock_enabled().await;
    h.source.set_locked();
    let r = h.unlock(&[]).await;
    assert_eq!(r.status, 202);
    r.assert_mandatory_headers();
    r.assert_json_body();
    assert_eq!(r.result(), "unlock_requested");
    assert_eq!(h.source.unlock_ids(), vec![SESSION_ID.to_string()]);
    assert!(h.source.lock_ids().is_empty(), "an unlock never locks");
    let reads_after_first = h.source.reads();
    assert_eq!(
        reads_after_first, 1,
        "one fresh snapshot before the unlock call"
    );

    let r = h.unlock(&[]).await;
    assert_eq!(r.status, 429);
    assert_eq!(r.result(), "rate_limited");
    assert_eq!(h.source.unlock_ids().len(), 1);
    assert_eq!(
        h.source.reads(),
        reads_after_first,
        "a rate-limited unlock never reads logind"
    );

    h.advance_ms(MIN_UNLOCK_INTERVAL_MS - 1).await;
    assert_eq!(h.unlock(&[]).await.status, 429, "still inside the interval");
    h.advance_ms(2).await;
    assert_eq!(h.unlock(&[]).await.status, 202);
    assert_eq!(h.source.unlock_ids().len(), 2);

    // already_unlocked: no call, interval not recorded.
    h.advance_ms(MIN_UNLOCK_INTERVAL_MS + 1).await;
    h.source.set_unlocked();
    let r = h.unlock(&[]).await;
    assert_eq!(r.status, 409);
    assert_eq!(r.result(), "already_unlocked");
    assert_eq!(h.source.unlock_ids().len(), 2);
    h.source.set_locked();
    assert_eq!(
        h.unlock(&[]).await.status,
        202,
        "a 409 recorded no interval"
    );
    assert_eq!(h.source.unlock_ids().len(), 3);

    // no_session, including a locked remote session (never unlocked).
    h.advance_ms(MIN_UNLOCK_INTERVAL_MS + 1).await;
    h.source.set_no_session();
    let r = h.unlock(&[]).await;
    assert_eq!(r.status, 409);
    assert_eq!(r.result(), "no_session");
    let mut remote = session(true);
    remote.remote = Some(true);
    h.source.set_sessions(vec![remote]);
    assert_eq!(
        h.unlock(&[]).await.result(),
        "no_session",
        "a remote session is never unlocked"
    );
    let mut greeter = session(true);
    greeter.class = Some("greeter".to_string());
    h.source.set_sessions(vec![greeter]);
    assert_eq!(
        h.unlock(&[]).await.result(),
        "no_session",
        "only a user-class session"
    );
    assert_eq!(h.source.unlock_ids().len(), 3);

    // Snapshot failure: 503, no call, interval not recorded.
    h.source.set_error(SourceError::Timeout);
    let r = h.unlock(&[]).await;
    assert_eq!(r.status, 503);
    assert_eq!(r.result(), "unavailable");
    assert_eq!(h.source.unlock_ids().len(), 3);
    h.source.set_locked();
    assert_eq!(h.unlock(&[]).await.status, 202);
    assert_eq!(h.source.unlock_ids().len(), 4);

    // UnlockSession failure: 503, exactly one attempt, the interval is recorded.
    h.advance_ms(MIN_UNLOCK_INTERVAL_MS + 1).await;
    h.source.set_unlock_result(Err(SourceError::Call));
    let r = h.unlock(&[]).await;
    assert_eq!(r.status, 503);
    assert_eq!(r.result(), "unavailable");
    assert_eq!(
        h.source.unlock_ids().len(),
        5,
        "exactly one attempt, no retry"
    );
    h.source.set_unlock_result(Ok(()));
    assert_eq!(
        h.unlock(&[]).await.status,
        429,
        "a failed UnlockSession still counts"
    );
    assert!(h.source.unlock_ids().iter().all(|id| id == SESSION_ID));
    assert!(h.source.lock_ids().is_empty());
}

/// RMC23: the unlock and lock rate limits are independent (locking right after an unlock,
/// or unlocking right after a lock, is never `429`).
#[tokio::test(start_paused = true)]
async fn test_rmc_unlock_and_lock_rate_limits_are_independent() {
    let h = Harness::start_unlock_enabled().await;
    assert_eq!(h.lock(&[]).await.status, 202);
    h.source.set_locked();
    assert_eq!(h.unlock(&[]).await.status, 202);
    h.source.set_unlocked();
    assert_eq!(
        h.lock(&[]).await.status,
        429,
        "the lock interval still applies"
    );
    assert_eq!(h.source.lock_ids().len(), 1);
    assert_eq!(h.source.unlock_ids().len(), 1);
}

/// RMC24: the unlock needs every identity, host and CSRF check, with its own action value
/// (`X-Soos-Action: unlock`); a refused request never reaches logind, enabled or not.
#[tokio::test(start_paused = true)]
async fn test_rmc_unlock_requires_identity_host_and_csrf() {
    let h = Harness::start_unlock_enabled().await;
    h.source.set_locked();
    for extra in [
        vec![],
        vec![("X-Soos-Action", "lock")],
        vec![("X-Soos-Action", "UNLOCK")],
        vec![("X-Soos-Action", "unlock"), ("X-Soos-Action", "unlock")],
        vec![
            ("X-Soos-Action", "unlock"),
            ("Sec-Fetch-Site", "cross-site"),
        ],
        vec![("X-Soos-Action", "unlock"), ("Sec-Fetch-Site", "same-site")],
        vec![
            ("X-Soos-Action", "unlock"),
            ("Origin", "https://other.tail1234.ts.net"),
        ],
        vec![
            ("X-Soos-Action", "unlock"),
            ("Origin", "http://pc.tail1234.ts.net"),
        ],
        vec![("X-Soos-Action", "unlock"), ("Origin", "null")],
    ] {
        let r = h.request("POST", "/api/unlock", &extra).await;
        assert_eq!(r.status, 403, "{extra:?}");
        assert_eq!(r.result(), "forbidden", "{extra:?}");
    }
    // The lock route never accepts the unlock action, and vice versa (no confusion).
    let r = h
        .request("POST", "/api/lock", &[("X-Soos-Action", "unlock")])
        .await;
    assert_eq!(r.status, 403);

    // Identity: unknown login, missing login, repeated login.
    let r = h
        .raw(&raw_request(
            "POST",
            "/api/unlock",
            &[
                ("Host", HOST),
                ("Tailscale-User-Login", "evil@example.org"),
                ("X-Soos-Action", "unlock"),
            ],
        ))
        .await;
    assert_eq!(r.status, 403);
    let r = h
        .raw(&raw_request(
            "POST",
            "/api/unlock",
            &[("Host", HOST), ("X-Soos-Action", "unlock")],
        ))
        .await;
    assert_eq!(r.status, 403);
    let r = h
        .raw(&raw_request(
            "POST",
            "/api/unlock",
            &[
                ("Host", HOST),
                ("Tailscale-User-Login", LOGIN),
                ("Tailscale-User-Login", LOGIN),
                ("X-Soos-Action", "unlock"),
            ],
        ))
        .await;
    assert_eq!(r.status, 403);
    // Host: not a tailnet name.
    let r = h
        .raw(&raw_request(
            "POST",
            "/api/unlock",
            &[
                ("Host", "evil.example.com"),
                ("Tailscale-User-Login", LOGIN),
                ("X-Soos-Action", "unlock"),
            ],
        ))
        .await;
    assert_eq!(r.status, 421);
    // A body is refused.
    let mut with_body = raw_request(
        "POST",
        "/api/unlock",
        &with_identity(&[("X-Soos-Action", "unlock"), ("Content-Length", "2")]),
    );
    with_body.extend_from_slice(b"{}");
    assert_eq!(h.raw(&with_body).await.status, 413);

    assert!(h.source.unlock_ids().is_empty());
    assert!(h.source.lock_ids().is_empty());
    assert_eq!(
        h.source.reads(),
        0,
        "every refusal happens before any logind read"
    );

    let r = h
        .unlock(&[
            ("Sec-Fetch-Site", "same-origin"),
            ("Origin", "https://PC.Tail1234.TS.NET:443"),
        ])
        .await;
    assert_eq!(r.status, 202);
    assert_eq!(h.source.unlock_ids(), vec![SESSION_ID.to_string()]);
}

/// RMC24: a disabled unlock still runs the CSRF check first (a cross-site request learns
/// `forbidden`, not whether unlock is enabled).
#[tokio::test(start_paused = true)]
async fn test_rmc_disabled_unlock_checks_csrf_first() {
    let h = Harness::start().await;
    let r = h
        .request("POST", "/api/unlock", &[("X-Soos-Action", "lock")])
        .await;
    assert_eq!(r.status, 403);
    assert_eq!(r.result(), "forbidden");
    assert_eq!(h.source.reads(), 0);
}

/// RMC22: `/api/unlock` is `POST` only (`405`, `Allow: POST`), like `/api/lock`.
#[tokio::test(start_paused = true)]
async fn test_rmc_unlock_route_is_post_only() {
    let h = Harness::start_unlock_enabled().await;
    for method in ["GET", "HEAD", "PUT", "DELETE"] {
        let r = h
            .request(method, "/api/unlock", &[("X-Soos-Action", "unlock")])
            .await;
        assert_eq!(r.status, 405, "{method}");
        assert_eq!(r.header("allow"), Some("POST"), "{method}");
        r.assert_mandatory_headers();
    }
    assert_eq!(h.source.reads(), 0);
    assert!(h.source.unlock_ids().is_empty());
}

/// RMC25: the whole unlock flow is bounded by `UNLOCK_FLOW_DEADLINE_MS`: a hung
/// `UnlockSession` answers `503 unavailable` at the deadline, never later.
#[tokio::test(start_paused = true)]
async fn test_rmc_unlock_flow_deadline() {
    let h = Harness::start_unlock_enabled().await;
    h.source.set_locked();
    h.source.hold_unlock_next(1);
    let unlock = spawn_exchange(
        &h.path,
        raw_request(
            "POST",
            "/api/unlock",
            &with_identity(&[("X-Soos-Action", "unlock")]),
        ),
    );
    h.source.wait_reads_at_least(1).await;
    settle().await;
    assert_eq!(
        h.source.unlock_ids().len(),
        1,
        "UnlockSession was called and hangs"
    );
    h.advance_ms(UNLOCK_FLOW_DEADLINE_MS - 1).await;
    assert!(
        !finished_soon(&unlock).await,
        "no answer before UNLOCK_FLOW_DEADLINE_MS"
    );
    h.advance_ms(1).await;
    assert!(
        finished_soon(&unlock).await,
        "the unlock flow deadline cuts the hung call"
    );
    match unlock.await.unwrap() {
        Outcome::Response(r) => {
            assert_eq!(r.status, 503);
            assert_eq!(r.result(), "unavailable");
        }
        other => panic!("expected a 503, got {other:?}"),
    }
    assert_eq!(h.source.unlock_ids().len(), 1, "one attempt, no retry");
    h.source.release();
    let r = h.get("/api/status").await;
    assert_eq!(r.json()["state"], "locked", "the service is still healthy");
}

/// RMC25: every accepted unlock leaves one `info` audit line without identity, session id
/// or request data; refused unlocks leave no such line.
#[tokio::test(start_paused = true)]
async fn test_rmc_unlock_is_audited_without_identity() {
    let capture = LogCapture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(capture.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let h = Harness::start_unlock_enabled().await;
    h.source.set_unlocked();
    let _ = h.unlock(&[]).await; // 409: no audit line
    let _ = h
        .request("POST", "/api/unlock", &[("X-Soos-Action", "lock")])
        .await; // 403
    assert!(
        !capture.text().contains("remote unlock requested"),
        "refused unlocks are not audited as requested"
    );
    h.source.set_locked();
    assert_eq!(h.unlock(&[]).await.status, 202);
    let text = capture.text();
    let audit: Vec<&str> = text
        .lines()
        .filter(|l| l.contains("remote unlock requested"))
        .collect();
    assert_eq!(audit.len(), 1, "{text}");
    assert!(audit[0].contains("INFO"), "{}", audit[0]);
    for forbidden in [
        LOGIN,
        SESSION_ID,
        HOST,
        "1000",
        "/api/unlock",
        "x-soos-action",
    ] {
        assert!(!text.contains(forbidden), "the log leaks {forbidden}");
    }
}

/// D5 / §4: a lock request with a body is refused before any logind call.
#[tokio::test(start_paused = true)]
async fn test_rmc_lock_with_a_body_is_refused_before_any_logind_call() {
    let h = Harness::start().await;
    let mut with_body = raw_request(
        "POST",
        "/api/lock",
        &with_identity(&[("X-Soos-Action", "lock"), ("Content-Length", "2")]),
    );
    with_body.extend_from_slice(b"{}");
    let r = h.raw(&with_body).await;
    assert_eq!(r.status, 413);
    let r = h
        .request(
            "POST",
            "/api/lock",
            &[("X-Soos-Action", "lock"), ("Transfer-Encoding", "chunked")],
        )
        .await;
    assert_eq!(r.status, 400);
    assert!(h.source.lock_ids().is_empty());
    assert_eq!(h.source.reads(), 0);
    let r = h
        .request(
            "POST",
            "/api/lock",
            &[("X-Soos-Action", "lock"), ("Content-Length", "0")],
        )
        .await;
    assert_eq!(r.status, 202, "Content-Length: 0 is not a body");
}

/// F8 / D9: a hung logind read is cut by exactly `SNAPSHOT_DEADLINE_MS` (status
/// `unavailable`) and the whole lock flow by `LOCK_FLOW_DEADLINE_MS` (`503`), with exactly
/// one attempt and no retry.
#[tokio::test(start_paused = true)]
async fn test_rmc_deadlines_bound_hung_logind_calls() {
    let h = Harness::start().await;
    h.source.hold_next(1);
    let status = spawn_exchange(
        &h.path,
        raw_request("GET", "/api/status", &with_identity(&[])),
    );
    h.source.wait_reads_at_least(1).await;
    h.advance_ms(SNAPSHOT_DEADLINE_MS - 1).await;
    assert!(
        !finished_soon(&status).await,
        "no answer before SNAPSHOT_DEADLINE_MS"
    );
    h.advance_ms(1).await;
    assert!(
        finished_soon(&status).await,
        "the snapshot deadline cuts the hung read"
    );
    match status.await.unwrap() {
        Outcome::Response(r) => {
            assert_eq!(r.status, 200);
            assert_eq!(r.json()["state"], "unavailable");
        }
        other => panic!("expected an unavailable status, got {other:?}"),
    }
    assert_eq!(h.source.reads(), 1, "the hung read is not retried");
    h.source.release();

    h.source.hold_next(1);
    let lock = spawn_exchange(
        &h.path,
        raw_request(
            "POST",
            "/api/lock",
            &with_identity(&[("X-Soos-Action", "lock")]),
        ),
    );
    h.source.wait_reads_at_least(2).await;
    h.advance_ms(LOCK_FLOW_DEADLINE_MS).await;
    assert!(
        finished_soon(&lock).await,
        "the lock flow deadline cuts the hung snapshot"
    );
    match lock.await.unwrap() {
        Outcome::Response(r) => {
            assert_eq!(r.status, 503);
            assert_eq!(r.result(), "unavailable");
        }
        other => panic!("expected a 503, got {other:?}"),
    }
    assert!(
        h.source.lock_ids().is_empty(),
        "no LockSession after a failed snapshot"
    );
    h.source.release();

    h.advance_ms(MIN_LOCK_INTERVAL_MS + 1).await;
    h.source.hold_lock_next(1);
    let lock = spawn_exchange(
        &h.path,
        raw_request(
            "POST",
            "/api/lock",
            &with_identity(&[("X-Soos-Action", "lock")]),
        ),
    );
    h.source.wait_reads_at_least(3).await;
    settle().await;
    assert_eq!(
        h.source.lock_ids().len(),
        1,
        "LockSession was called and hangs"
    );
    h.advance_ms(LOCK_FLOW_DEADLINE_MS).await;
    assert!(
        finished_soon(&lock).await,
        "the lock flow deadline cuts the hung LockSession"
    );
    match lock.await.unwrap() {
        Outcome::Response(r) => assert_eq!(r.status, 503),
        other => panic!("expected a 503, got {other:?}"),
    }
    assert_eq!(h.source.lock_ids().len(), 1, "one attempt, no retry");
    h.source.release();
    // The service is still healthy afterwards.
    let r = h.get("/api/status").await;
    assert_eq!(r.json()["state"], "unlocked");
}

// ---------------------------------------------------------------------------------------
// /api/events
// ---------------------------------------------------------------------------------------

/// D6: the SSE head and a first event that is the stream's own fresh read.
#[tokio::test(start_paused = true)]
async fn test_rmc_events_first_event_is_a_fresh_read() {
    let h = Harness::start().await;
    h.clock.shift_ms(1234);
    let expected_ms = h.clock.now_ms();
    let mut stream = h.open_stream().await;
    assert_sse_head(&stream.head);
    let first = stream.expect_event_now().await;
    assert_status_shape(&first);
    assert_eq!(
        first,
        serde_json::json!({
            "state": "unlocked",
            "active": true,
            "idle": false,
            "idle_since_unix_s": null,
            "checked_unix_ms": expected_ms
        })
    );
    assert!(h.source.reads() >= 1);
    assert!(h.source.uids().iter().all(|&u| u == UID));
}

/// D6: an event per change of the view, none while the view is unchanged (the poller keeps
/// reading every `poll_interval_ms`).
#[tokio::test(start_paused = true)]
async fn test_rmc_events_emit_on_change_only() {
    let h = Harness::start().await;
    let mut stream = h.open_stream().await;
    let first = stream.expect_state_now("unlocked").await;
    let change_bound = DEFAULT_POLL_INTERVAL_MS + SNAPSHOT_DEADLINE_MS;
    h.source.set_locked();
    let locked = stream.expect_state_within(&h, "locked", change_bound).await;
    assert!(checked(&locked) >= checked(&first));
    let reads_before = h.source.reads();
    stream.expect_silence_for(&h, 5000).await;
    assert!(
        h.source.reads() >= reads_before + 4,
        "the poller kept reading: {} after {reads_before}",
        h.source.reads()
    );
    h.source.set_no_session();
    let event = stream
        .expect_state_within(&h, "no_session", change_bound)
        .await;
    assert_eq!(event["active"], false);
    h.source.set_error(SourceError::BusUnavailable);
    stream
        .expect_state_within(&h, "unavailable", change_bound)
        .await;
    h.source.set_unlocked();
    stream
        .expect_state_within(&h, "unlocked", change_bound)
        .await;
    let mut idle = session(false);
    idle.idle = true;
    idle.idle_since_unix_s = Some(1_700_000_000);
    h.source.set_sessions(vec![idle]);
    let event = stream
        .expect_state_within(&h, "unlocked", change_bound)
        .await;
    assert_eq!(event["idle"], true, "an idle change is a change");
    assert_eq!(event["idle_since_unix_s"], 1_700_000_000u64);
}

/// R3 / R2-1: in a steady state the newest reading is re-sent every `SSE_KEEPALIVE_MS` with
/// a strictly greater `checked_unix_ms`, and nothing is sent in between.
#[tokio::test(start_paused = true)]
async fn test_rmc_events_keepalive_resends_with_advancing_timestamp() {
    let h = Harness::start().await;
    let mut stream = h.open_stream().await;
    let first = stream.expect_state_now("unlocked").await;
    let mut previous = checked(&first);
    for round in 0..3 {
        stream.expect_silence_for(&h, SSE_KEEPALIVE_MS - 1000).await;
        let (event, waited) = stream
            .await_event(&h, 2000)
            .await
            .unwrap_or_else(|| panic!("round {round}: no keep-alive within 16 s"));
        assert_eq!(event["state"], "unlocked", "round {round}: {event}");
        assert!(
            checked(&event) > previous,
            "round {round}: {} must exceed {previous}",
            checked(&event)
        );
        assert!(
            checked(&event) + SSE_KEEPALIVE_MS >= h.clock.now_ms(),
            "round {round}: the re-sent reading is a fresh poller read, not the first one"
        );
        assert!(
            waited <= 1000,
            "round {round}: keep-alive late by {waited} ms"
        );
        previous = checked(&event);
    }
    assert!(
        h.source.reads() >= 40,
        "the poller read every second: {}",
        h.source.reads()
    );
}

/// F2 / RC-3: a new stream never replays an older value; its first event is its own fresh
/// read (`unavailable` or `locked`), never the `unlocked` a previous stream saw.
#[tokio::test(start_paused = true)]
async fn test_rmc_events_new_stream_never_replays_a_stale_unlocked() {
    let h = Harness::start().await;
    let mut first_stream = h.open_stream().await;
    first_stream.expect_state_now("unlocked").await;
    h.step_ms(500, 2500).await;
    assert!(
        h.source.reads() >= 3,
        "the poller published unlocked readings"
    );
    drop(first_stream);
    settle().await;

    h.source.set_error(SourceError::BusUnavailable);
    let mut second = h.open_stream().await;
    let event = second.expect_event_now().await;
    assert_eq!(event["state"], "unavailable", "{event}");
    drop(second);
    settle().await;

    h.source.set_locked();
    let mut third = h.open_stream().await;
    let event = third.expect_event_now().await;
    assert_eq!(event["state"], "locked", "{event}");
    assert_eq!(checked(&event), h.clock.now_ms());
}

/// D6: zero D-Bus traffic without a stream; the poller runs only while one is open.
#[tokio::test(start_paused = true)]
async fn test_rmc_poller_runs_only_while_a_stream_is_open() {
    let h = Harness::start().await;
    h.step_ms(1000, 5000).await;
    assert_eq!(h.source.reads(), 0, "no stream, no read");
    let mut stream = h.open_stream().await;
    stream.expect_state_now("unlocked").await;
    let after_open = h.source.reads();
    assert!(after_open >= 1);
    h.step_ms(500, 3500).await;
    assert!(
        h.source.reads() >= after_open + 3,
        "about one read per second while open: {} after {after_open}",
        h.source.reads()
    );
    drop(stream);
    settle().await;
    let after_close = h.source.reads();
    h.step_ms(1000, 5000).await;
    assert!(
        h.source.reads() <= after_close + 1,
        "no new read after the last stream closed: {} vs {after_close}",
        h.source.reads()
    );
}

/// §1.1 config: `poll_interval_ms` drives the poller.
#[tokio::test(start_paused = true)]
async fn test_rmc_poll_interval_from_config_drives_the_poller() {
    let fast = Harness::start_with(Options {
        poll_interval_ms: Some(250),
        ..Options::default()
    })
    .await;
    let mut stream = fast.open_stream().await;
    stream.expect_state_now("unlocked").await;
    let r1 = fast.source.reads();
    fast.step_ms(250, 1100).await;
    assert!(
        fast.source.reads() >= r1 + 4,
        "{} after {r1}",
        fast.source.reads()
    );
    drop(stream);
    drop(fast);

    let slow = Harness::start_with(Options {
        poll_interval_ms: Some(10_000),
        ..Options::default()
    })
    .await;
    let mut stream = slow.open_stream().await;
    stream.expect_state_now("unlocked").await;
    let r1 = slow.source.reads();
    slow.step_ms(1000, 5000).await;
    assert!(
        slow.source.reads() <= r1 + 1,
        "{} after {r1}",
        slow.source.reads()
    );
}

/// §2.5 / F6: at most `MAX_SSE_STREAMS` streams (`503` beyond); a closed client frees its
/// slot at once, without waiting for a keep-alive.
#[tokio::test(start_paused = true)]
async fn test_rmc_events_stream_limit_and_slot_release() {
    let h = Harness::start().await;
    let mut streams = Vec::new();
    for i in 0..MAX_SSE_STREAMS {
        let mut stream = h.open_stream().await;
        stream.expect_state_now("unlocked").await;
        streams.push(stream);
        assert_eq!(streams.len(), i + 1);
    }
    match h.try_open_stream(&with_identity(&[])).await {
        Err(Outcome::Response(r)) => {
            assert_eq!(r.status, 503);
            r.assert_mandatory_headers();
        }
        Err(Outcome::Closed) => panic!("the fifth stream must get a 503, not a drop"),
        Err(Outcome::Silent) => panic!("the fifth stream got no answer"),
        Ok(_) => panic!("a fifth stream was accepted"),
    }
    let first = streams.remove(0);
    drop(first);
    let mut reopened = None;
    let mut waited = 0;
    for _ in 0..20 {
        settle().await;
        match h.try_open_stream(&with_identity(&[])).await {
            Ok(stream) => {
                reopened = Some(stream);
                break;
            }
            Err(Outcome::Response(r)) if r.status == 503 => {
                h.advance_ms(50).await;
                waited += 50;
            }
            Err(Outcome::Response(r)) => panic!("unexpected status {}", r.status),
            Err(_) => panic!("stream dropped without a head"),
        }
    }
    let mut reopened = reopened.unwrap_or_else(|| panic!("slot not released after {waited} ms"));
    assert!(
        waited <= 1000,
        "slot released only after {waited} ms (keep-alive is 15 s)"
    );
    reopened.expect_state_now("unlocked").await;
    assert_eq!(streams.len(), MAX_SSE_STREAMS - 1);
}

/// §3: a stream is closed after `MAX_SSE_STREAM_MS`, every event in between being a
/// keep-alive with an advancing timestamp.
#[tokio::test(start_paused = true)]
async fn test_rmc_events_stream_closes_after_max_stream_ms() {
    let h = Harness::start().await;
    let t0 = Instant::now();
    let mut stream = h.open_stream().await;
    let first = stream.expect_state_now("unlocked").await;
    let mut previous = checked(&first);
    let mut events = 0u64;
    let mut closed = false;
    for _ in 0..130 {
        h.advance_ms(SSE_KEEPALIVE_MS).await;
        loop {
            match stream.poll_event() {
                Polled::Status(event) => {
                    assert_eq!(event["state"], "unlocked");
                    assert!(checked(&event) > previous, "{event} after {previous}");
                    previous = checked(&event);
                    events += 1;
                }
                Polled::Eof => {
                    closed = true;
                    break;
                }
                Polled::Pending => break,
            }
        }
        if closed {
            break;
        }
    }
    let elapsed = t0.elapsed();
    assert!(closed, "stream still open after {elapsed:?}");
    assert!(
        elapsed >= ms(MAX_SSE_STREAM_MS),
        "closed early at {elapsed:?}"
    );
    assert!(
        elapsed <= ms(MAX_SSE_STREAM_MS + SSE_KEEPALIVE_MS + 1000),
        "closed late at {elapsed:?}"
    );
    let expected = MAX_SSE_STREAM_MS / SSE_KEEPALIVE_MS;
    assert!(
        events + 2 >= expected && events <= expected + 1,
        "{events} keep-alives over the stream lifetime"
    );
    // The slot is free again.
    let mut again = h.open_stream().await;
    again.expect_state_now("unlocked").await;
}

/// F2 / §2.8: a poller task that ends makes `serve` return an error (`main` exits
/// `EXIT_RUNTIME`). The reading counter is exhausted so the first reservation fails.
#[tokio::test(start_paused = true)]
async fn test_rmc_poller_ending_makes_serve_return_an_error() {
    let mut h = Harness::start_with(Options {
        seq_start: Some(u64::MAX),
        ..Options::default()
    })
    .await;
    // Opening a stream wakes the poller; its first reservation overflows.
    let mut stream = connect(&h.path).await;
    stream
        .write_all(&raw_request("GET", "/api/events", &with_identity(&[])))
        .await
        .unwrap();
    let result = h.await_server().await;
    assert!(
        matches!(result, Err(ServeError::PollerEnded)),
        "serve must report the ended poller: {result:?}"
    );
    let mut buf = Vec::new();
    let _ = timeout(IO_BOUND, stream.read_to_end(&mut buf)).await;
    // Whatever the stream received, it never got a valid reading.
    assert!(
        !String::from_utf8_lossy(&buf).contains("\"state\":\"unlocked\""),
        "{:?}",
        String::from_utf8_lossy(&buf)
    );
}

/// R2-4: a backward wall-clock step never silences a change (the filter is the monotonic
/// `seq`, not `checked_unix_ms`).
#[tokio::test(start_paused = true)]
async fn test_rmc_backward_wall_clock_does_not_silence_changes() {
    let h = Harness::start().await;
    let mut stream = h.open_stream().await;
    let first = stream.expect_state_now("unlocked").await;
    let change_bound = DEFAULT_POLL_INTERVAL_MS + SNAPSHOT_DEADLINE_MS;
    h.clock.shift_ms(-3_600_000);
    h.source.set_locked();
    let locked = stream.expect_state_within(&h, "locked", change_bound).await;
    assert!(
        checked(&locked) < checked(&first),
        "the clock did step back"
    );
    h.source.set_unlocked();
    stream
        .expect_state_within(&h, "unlocked", change_bound)
        .await;
    // The keep-alive still arrives, stamped from the stepped clock.
    stream.expect_silence_for(&h, SSE_KEEPALIVE_MS - 1000).await;
    let (keepalive, _) = stream
        .await_event(&h, 2000)
        .await
        .expect("keep-alive after the clock step");
    assert_eq!(keepalive["state"], "unlocked");
    assert!(checked(&keepalive) < checked(&first));
}

/// R3-1: a reading's `seq` is reserved when its read starts, so a slow read that began
/// before a stream's own first read can never override that newer state.
#[tokio::test(start_paused = true)]
async fn test_rmc_seq_is_reserved_when_a_read_starts() {
    let h = Harness::start().await;
    let mut stream_a = h.open_stream().await;
    stream_a.expect_state_now("unlocked").await;
    // The next poller read snapshots `unlocked` at its start and then hangs.
    let before = h.source.reads();
    h.source.hold_next(1);
    h.step_ms(250, DEFAULT_POLL_INTERVAL_MS).await;
    h.source.wait_reads_at_least(before + 1).await;
    // The desktop locks while that read is in flight; a new stream reads `locked`.
    h.source.set_locked();
    let mut stream_b = h.open_stream().await;
    let b_first = stream_b.expect_event_now().await;
    assert_eq!(b_first["state"], "locked", "{b_first}");
    // The slow read completes now with its stale `unlocked` and an older seq.
    h.source.release();
    settle().await;
    stream_b.expect_silence_now().await;
    // Over the next poller ticks (all `locked`) B still sees nothing new.
    stream_b.expect_silence_for(&h, 3000).await;
    // Stream A ends up locked.
    let mut last = None;
    while let Polled::Status(event) = stream_a.poll_event() {
        last = Some(event);
    }
    assert_eq!(
        last.map(|e| e["state"].clone()),
        Some(Value::from("locked")),
        "stream A ends up locked"
    );
}

// ---------------------------------------------------------------------------------------
// Connection bounds, shutdown, logging
// ---------------------------------------------------------------------------------------

/// §2.8 / RC-4: beyond `MAX_CONNECTIONS` an accepted stream is dropped at once with no
/// response; idle connections are closed without response after `REQUEST_HEAD_TIMEOUT_MS`.
#[tokio::test(start_paused = true)]
async fn test_rmc_connection_limit_drops_excess_without_response() {
    let h = Harness::start().await;
    let mut idle = Vec::new();
    for _ in 0..MAX_CONNECTIONS {
        idle.push(connect(&h.path).await);
    }
    settle().await;
    let mut excess = connect(&h.path).await;
    let mut buf = Vec::new();
    let outcome = timeout(IO_BOUND, excess.read_to_end(&mut buf)).await;
    assert!(
        matches!(outcome, Ok(Ok(0)) | Ok(Err(_))),
        "the 17th connection is dropped at once: {outcome:?}"
    );
    assert!(
        buf.is_empty(),
        "no response bytes: {:?}",
        String::from_utf8_lossy(&buf)
    );
    // The idle connections are still open before the head timeout.
    settle().await;
    let mut probe = [0u8; 1];
    for stream in &idle {
        let r = stream.try_read(&mut probe);
        assert!(
            matches!(r, Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock),
            "idle connection closed early: {r:?}"
        );
    }
    h.advance_ms(REQUEST_HEAD_TIMEOUT_MS - 1).await;
    for stream in &idle {
        let r = stream.try_read(&mut probe);
        assert!(
            matches!(r, Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock),
            "idle connection closed before REQUEST_HEAD_TIMEOUT_MS: {r:?}"
        );
    }
    h.advance_ms(1).await;
    for mut stream in idle {
        let mut buf = Vec::new();
        let outcome = timeout(IO_BOUND, stream.read_to_end(&mut buf)).await;
        assert!(matches!(outcome, Ok(Ok(0)) | Ok(Err(_))), "{outcome:?}");
        assert!(buf.is_empty(), "idle connections get no response");
    }
    // Permits are back.
    let r = h.get("/api/status").await;
    assert_eq!(r.status, 200);
}

/// §2.8: an incomplete head is closed without response after exactly
/// `REQUEST_HEAD_TIMEOUT_MS`.
#[tokio::test(start_paused = true)]
async fn test_rmc_request_head_timeout_closes_without_response() {
    let h = Harness::start().await;
    let partial = spawn_exchange(
        &h.path,
        b"GET /api/status HTTP/1.1\r\nHost: pc.tail1234.ts.net\r\n".to_vec(),
    );
    settle().await;
    h.advance_ms(REQUEST_HEAD_TIMEOUT_MS - 1).await;
    assert!(
        !finished_soon(&partial).await,
        "the connection is kept until the head timeout"
    );
    h.advance_ms(1).await;
    assert!(
        finished_soon(&partial).await,
        "the head timeout closes the connection"
    );
    let outcome = partial.await.unwrap();
    assert!(
        matches!(outcome, Outcome::Closed),
        "closed without a response: {outcome:?}"
    );
    assert_eq!(h.source.reads(), 0);
    let r = h.get("/api/status").await;
    assert_eq!(r.status, 200, "the service is unaffected");
}

/// §2.7 / §2.8: a clean shutdown ends the streams, returns `Ok(())` and removes the socket
/// file.
#[tokio::test(start_paused = true)]
async fn test_rmc_shutdown_removes_the_socket_and_ends_streams() {
    let h = Harness::start().await;
    let mut stream = h.open_stream().await;
    stream.expect_state_now("unlocked").await;
    let path = h.path.clone();
    assert!(path.exists());
    let result = h.shutdown().await;
    assert!(result.is_ok(), "{result:?}");
    assert!(
        !path.exists(),
        "the socket file is removed on clean shutdown"
    );
    stream.expect_eof_now().await;
    assert!(
        UnixStream::connect(&path).await.is_err(),
        "nothing listens after shutdown"
    );
}

/// RC-5 / D12: no identity, header value, Host, session id or request path is ever logged,
/// at any level, on accepted or refused requests.
#[tokio::test(start_paused = true)]
async fn test_rmc_server_never_logs_identity_or_request_data() {
    let capture = LogCapture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(capture.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let h = Harness::start().await;
    let _ = h.get("/api/status").await;
    let _ = h.lock(&[]).await;
    let _ = h
        .raw(&raw_request(
            "GET",
            "/api/status",
            &[("Host", HOST), ("Tailscale-User-Login", "evil@example.org")],
        ))
        .await;
    let _ = h
        .raw(&raw_request(
            "GET",
            "/nope-secret-probe",
            &[
                ("Host", "evil.example.com"),
                ("Tailscale-User-Login", LOGIN),
            ],
        ))
        .await;
    let _ = h.get("/nope-secret-probe").await;
    let _ = h
        .request(
            "POST",
            "/api/lock",
            &[
                ("Origin", "https://attacker.example"),
                ("X-Soos-Action", "lock"),
            ],
        )
        .await;
    let _ = h
        .raw(b"GET / HTTP/1.0\r\nX-Secret: hunter2-value\r\n\r\n")
        .await;
    let mut stream = h.open_stream().await;
    stream.expect_state_now("unlocked").await;
    h.source.set_error(SourceError::Call);
    stream
        .expect_state_within(
            &h,
            "unavailable",
            DEFAULT_POLL_INTERVAL_MS + SNAPSHOT_DEADLINE_MS,
        )
        .await;
    drop(stream);
    settle().await;
    let _ = h.shutdown().await;

    let log = capture.text();
    for forbidden in [
        LOGIN,
        "evil@example.org",
        "owner@",
        HOST,
        "evil.example.com",
        "attacker.example",
        SESSION_ID,
        "/nope-secret-probe",
        "hunter2-value",
        "/api/status",
        "/api/lock",
        "seat0",
    ] {
        assert!(
            !log.contains(forbidden),
            "the log must not contain {forbidden:?}:\n{log}"
        );
    }
}

// ---------------------------------------------------------------------------------------
// D5a′ — the request head as `tailscale serve unix:` delivers it (spec §13, Revision 4)
// ---------------------------------------------------------------------------------------

/// Synthetic stand-in for the forwarding address Serve adds (never a real 100.x address).
const FORWARDED_FOR: &str = "100.64.0.1";

/// The §13.1 capture, verbatim in shape, with the synthetic names of this suite: Serve
/// rewrites `Host` to `localhost` and carries the original name in `X-Forwarded-Host`.
/// `extra` is appended after the captured headers.
fn serve_head<'a>(extra: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
    let mut headers = vec![
        ("Host", "localhost"),
        ("Tailscale-User-Login", LOGIN),
        ("X-Forwarded-For", FORWARDED_FOR),
        ("X-Forwarded-Host", HOST),
        ("X-Forwarded-Proto", "https"),
    ];
    headers.extend_from_slice(extra);
    headers
}

/// The captured head with one header name dropped and `replacement` appended.
fn serve_head_without<'a>(
    name: &str,
    replacement: &[(&'a str, &'a str)],
) -> Vec<(&'a str, &'a str)> {
    let mut headers: Vec<(&str, &str)> = serve_head(&[])
        .into_iter()
        .filter(|(n, _)| !n.eq_ignore_ascii_case(name))
        .collect();
    headers.extend_from_slice(replacement);
    headers
}

/// D5a′ / RMC20: the head captured on the owner's host is accepted end to end (`Host:
/// localhost` is not inspected, the effective host comes from `X-Forwarded-Host`), and the
/// lock `Origin` is compared to the effective host, not to `Host`.
#[tokio::test(start_paused = true)]
async fn test_rmc_serve_head_status_and_lock_end_to_end() {
    let h = Harness::start().await;
    let r = h
        .raw(&raw_request("GET", "/api/status", &serve_head(&[])))
        .await;
    assert_eq!(r.status, 200, "{:?}", String::from_utf8_lossy(&r.body));
    r.assert_json_body();
    r.assert_mandatory_headers();
    assert_status_shape(&r.json());
    assert_eq!(r.json()["state"], "unlocked");
    assert_eq!(h.source.reads(), 1);

    // `Origin: https://localhost` names the rewritten `Host`, not the effective host.
    let r = h
        .raw(&raw_request(
            "POST",
            "/api/lock",
            &serve_head(&[("X-Soos-Action", "lock"), ("Origin", "https://localhost")]),
        ))
        .await;
    assert_eq!(r.status, 403);
    assert_eq!(r.result(), "forbidden");
    assert!(
        h.source.lock_ids().is_empty(),
        "a mismatched Origin never reaches LockSession"
    );
    assert_eq!(
        h.source.reads(),
        1,
        "CSRF is refused before any logind read"
    );

    let r = h
        .raw(&raw_request(
            "POST",
            "/api/lock",
            &serve_head(&[
                ("X-Soos-Action", "lock"),
                ("Origin", "https://pc.tail1234.ts.net"),
            ]),
        ))
        .await;
    assert_eq!(r.status, 202, "{:?}", String::from_utf8_lossy(&r.body));
    r.assert_json_body();
    assert_eq!(r.result(), "lock_requested");
    assert_eq!(h.source.lock_ids(), vec![SESSION_ID.to_string()]);
    assert_eq!(h.source.reads(), 2, "one fresh snapshot before the lock");

    // The browser Origin carries the name the user typed, case and :443 included.
    h.advance_ms(MIN_LOCK_INTERVAL_MS + 1).await;
    let r = h
        .raw(&raw_request(
            "POST",
            "/api/lock",
            &serve_head(&[
                ("X-Soos-Action", "lock"),
                ("Origin", "https://PC.Tail1234.TS.NET:443"),
            ]),
        ))
        .await;
    assert_eq!(r.status, 202);
    assert_eq!(h.source.lock_ids().len(), 2);
}

/// D5a′ "Transport": the captured head over `tailscale serve --http` is `421` end to end,
/// with the mandatory headers and no logind read.
#[tokio::test(start_paused = true)]
async fn test_rmc_serve_head_with_http_proto_is_misdirected() {
    let h = Harness::start().await;
    let r = h
        .raw(&raw_request(
            "GET",
            "/api/status",
            &serve_head_without("X-Forwarded-Proto", &[("X-Forwarded-Proto", "http")]),
        ))
        .await;
    assert_eq!(r.status, 421);
    r.assert_json_body();
    r.assert_mandatory_headers();
    assert_eq!(r.result(), "misdirected_request");
    let r = h
        .raw(&raw_request(
            "POST",
            "/api/lock",
            &serve_head_without(
                "X-Forwarded-Proto",
                &[
                    ("X-Forwarded-Proto", "http"),
                    ("X-Soos-Action", "lock"),
                    ("Origin", "https://pc.tail1234.ts.net"),
                ],
            ),
        ))
        .await;
    assert_eq!(r.status, 421, "the lock route is refused the same way");
    assert_eq!(r.result(), "misdirected_request");
    assert_eq!(h.source.reads(), 0);
    assert!(h.source.lock_ids().is_empty());
    // The scheme is the only difference with the accepted head: the `421` above is the
    // transport rule, not the rewritten `Host: localhost`.
    let r = h
        .raw(&raw_request("GET", "/api/status", &serve_head(&[])))
        .await;
    assert_eq!(r.status, 200, "{:?}", String::from_utf8_lossy(&r.body));
    assert_eq!(h.source.reads(), 1);
}

/// D5a′: a proxied head without `X-Forwarded-Proto`, or with a foreign `X-Forwarded-Host`,
/// is `421` end to end even though `Host: localhost` is the same as in the accepted head;
/// a repeated `X-Forwarded-Host` is refused as well.
#[tokio::test(start_paused = true)]
async fn test_rmc_serve_head_without_proto_or_with_foreign_host_is_misdirected() {
    let h = Harness::start().await;
    let variants: Vec<(&str, Vec<(&str, &str)>)> = vec![
        (
            "no X-Forwarded-Proto",
            serve_head_without("X-Forwarded-Proto", &[]),
        ),
        (
            "X-Forwarded-Proto twice",
            serve_head(&[("X-Forwarded-Proto", "https")]),
        ),
        (
            "foreign X-Forwarded-Host",
            serve_head_without("X-Forwarded-Host", &[("X-Forwarded-Host", "evil.com")]),
        ),
        (
            "X-Forwarded-Host with a port",
            serve_head_without(
                "X-Forwarded-Host",
                &[("X-Forwarded-Host", "pc.tail1234.ts.net:8443")],
            ),
        ),
        (
            "X-Forwarded-Host repeated",
            serve_head(&[("X-Forwarded-Host", HOST)]),
        ),
        (
            "X-Forwarded-Host is an IP literal",
            serve_head_without("X-Forwarded-Host", &[("X-Forwarded-Host", FORWARDED_FOR)]),
        ),
    ];
    for (label, headers) in variants {
        let r = h.raw(&raw_request("GET", "/api/status", &headers)).await;
        assert_eq!(r.status, 421, "{label}");
        r.assert_mandatory_headers();
        assert_eq!(r.result(), "misdirected_request", "{label}");
    }
    assert_eq!(h.source.reads(), 0, "no refused head reached logind");
    // The accepted head still passes on the same server.
    let r = h
        .raw(&raw_request("GET", "/api/status", &serve_head(&[])))
        .await;
    assert_eq!(r.status, 200);
}

/// D5a′ / RC-2: `allowed_hosts` applies to the effective host behind Serve.
#[tokio::test(start_paused = true)]
async fn test_rmc_allowed_hosts_apply_to_the_forwarded_host() {
    let h = Harness::start_with(Options {
        allowed_hosts: vec!["mypc.tail1234.ts.net".to_string()],
        ..Options::default()
    })
    .await;
    let r = h
        .raw(&raw_request("GET", "/api/status", &serve_head(&[])))
        .await;
    assert_eq!(
        r.status, 421,
        "another *.ts.net name in X-Forwarded-Host is refused once a list exists"
    );
    let r = h
        .raw(&raw_request(
            "GET",
            "/api/status",
            &serve_head_without(
                "X-Forwarded-Host",
                &[("X-Forwarded-Host", "MYPC.tail1234.ts.net:443")],
            ),
        ))
        .await;
    assert_eq!(r.status, 200, "{:?}", String::from_utf8_lossy(&r.body));
    assert_eq!(h.source.reads(), 1);
}
