//! Shared end-to-end harness of the passkey / Funnel contract tests (ADR 2026-10-06
//! "Tailscale Funnel Access and In-House Passkey Authentication for `soos-remote`", spec
//! §10.6, §10.9). It is the `server_tests.rs` harness (frozen paused clock, scripted logind,
//! raw HTTP/1.1 over the Unix socket; see that file's module documentation) extended with
//! the passkey configuration, the credential store fixture, the Funnel request shape set by
//! `tailscaled` and the WebAuthn ceremonies.
//!
//! Include with `#[path = "common/passkey.rs"] mod passkey;` then
//! `#[path = "common/harness.rs"] mod harness;`.

#![allow(
    dead_code,
    unused_imports,
    reason = "Shared test fixtures library used conditionally across test modules"
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

use soos_remote::auth::RandomSource;
use soos_remote::config::{AuthConfig, RemoteConfig, TailscaleLogin};
use soos_remote::logind::{SessionSource, SourceError};
use soos_remote::server::{serve, ServeError, ServerState};
use soos_remote::session::SessionProps;
use soos_remote::{
    CREDENTIALS_FILE_NAME, DEFAULT_POLL_INTERVAL_MS, LOCK_FLOW_DEADLINE_MS, MAX_CONNECTIONS,
    MAX_HEADERS, MAX_PATH_LEN, MAX_REQUEST_HEAD_BYTES, MAX_SSE_STREAMS, MAX_SSE_STREAM_MS,
    MIN_LOCK_INTERVAL_MS, MIN_UNLOCK_INTERVAL_MS, REQUEST_HEAD_TIMEOUT_MS, SNAPSHOT_DEADLINE_MS,
    SSE_KEEPALIVE_MS, UNLOCK_FLOW_DEADLINE_MS,
};

use super::passkey::*;

pub const HOST: &str = "pc.tail1234.ts.net";
pub const LOGIN: &str = "owner@example.com";
pub const UID: u32 = 1000;
pub const SESSION_ID: &str = "c0ffee42";
pub const BASE_UNIX_MS: u64 = 1_700_000_000_000;
/// Virtual bound of a client I/O step; it can only fire once auto-advance resumes.
pub const IO_BOUND: Duration = Duration::from_secs(120);
/// Real time after which the frozen clock is released (a stalled server, not a slow CI).
pub const WALL_CLOCK_FALLBACK: Duration = Duration::from_secs(20);
/// Scheduler rounds granted to the server between two client observations.
pub const SETTLE_ROUNDS: usize = 8;
/// Scheduler rounds granted before declaring "no event will come without time passing".
pub const POLL_ROUNDS: usize = 64;
/// Virtual step of [`SseClient::await_event`].
pub const STEP_MS: u64 = 250;
pub const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self'; \
                   connect-src 'self'; manifest-src 'self'; base-uri 'none'; \
                   form-action 'none'; frame-ancestors 'none'";

pub fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

/// Lets every runnable task (server included) run and the I/O driver deliver events,
/// without moving the clock.
pub async fn settle() {
    for _ in 0..SETTLE_ROUNDS {
        yield_now().await;
    }
}

// ---------------------------------------------------------------------------------------
// Test doubles
// ---------------------------------------------------------------------------------------

/// Unix clock bound to the paused tokio clock plus a settable offset.
pub struct TestClock {
    pub start: Instant,
    pub offset_ms: AtomicI64,
}

impl TestClock {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            start: Instant::now(),
            offset_ms: AtomicI64::new(0),
        })
    }

    pub fn now_ms(&self) -> u64 {
        let elapsed = self.start.elapsed().as_millis() as i64;
        (BASE_UNIX_MS as i64 + elapsed + self.offset_ms.load(Ordering::SeqCst)) as u64
    }

    pub fn shift_ms(&self, delta: i64) {
        self.offset_ms.fetch_add(delta, Ordering::SeqCst);
    }
}

pub fn session(locked: bool) -> SessionProps {
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

pub struct MockInner {
    pub answer: Mutex<Result<Vec<SessionProps>, SourceError>>,
    pub lock_answer: Mutex<Result<(), SourceError>>,
    pub unlock_answer: Mutex<Result<(), SourceError>>,
    pub hold_remaining: AtomicUsize,
    pub hold_lock_remaining: AtomicUsize,
    pub hold_unlock_remaining: AtomicUsize,
    pub release: watch::Sender<u64>,
    pub started: watch::Sender<u64>,
    pub lock_ids: Mutex<Vec<String>>,
    pub unlock_ids: Mutex<Vec<String>>,
    pub uids: Mutex<Vec<u32>>,
}

/// Scripted logind: a settable `own_sessions` answer (snapshotted when a call starts), a
/// settable `lock_session` answer, "hold the next N calls" gates, and spies.
#[derive(Clone)]
pub struct MockSource(pub Arc<MockInner>);

impl MockSource {
    pub fn unlocked() -> Self {
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

    pub fn set_sessions(&self, sessions: Vec<SessionProps>) {
        *self.0.answer.lock().unwrap() = Ok(sessions);
    }

    pub fn set_unlocked(&self) {
        self.set_sessions(vec![session(false)]);
    }

    pub fn set_locked(&self) {
        self.set_sessions(vec![session(true)]);
    }

    pub fn set_no_session(&self) {
        self.set_sessions(Vec::new());
    }

    pub fn set_error(&self, error: SourceError) {
        *self.0.answer.lock().unwrap() = Err(error);
    }

    pub fn set_lock_result(&self, result: Result<(), SourceError>) {
        *self.0.lock_answer.lock().unwrap() = result;
    }

    pub fn set_unlock_result(&self, result: Result<(), SourceError>) {
        *self.0.unlock_answer.lock().unwrap() = result;
    }

    /// The next `n` `unlock_session` calls block until [`Self::release`].
    pub fn hold_unlock_next(&self, n: usize) {
        self.0.hold_unlock_remaining.store(n, Ordering::SeqCst);
    }

    pub fn unlock_ids(&self) -> Vec<String> {
        self.0.unlock_ids.lock().unwrap().clone()
    }

    /// The next `n` `own_sessions` calls block (after snapshotting the answer) until
    /// [`Self::release`].
    pub fn hold_next(&self, n: usize) {
        self.0.hold_remaining.store(n, Ordering::SeqCst);
    }

    /// The next `n` `lock_session` calls block until [`Self::release`].
    pub fn hold_lock_next(&self, n: usize) {
        self.0.hold_lock_remaining.store(n, Ordering::SeqCst);
    }

    pub fn release(&self) {
        self.0.release.send_modify(|g| *g += 1);
    }

    /// Number of `own_sessions` calls started so far.
    pub fn reads(&self) -> u64 {
        *self.0.started.borrow()
    }

    pub async fn wait_reads_at_least(&self, n: u64) {
        let mut rx = self.0.started.subscribe();
        timeout(IO_BOUND, rx.wait_for(|c| *c >= n))
            .await
            .expect("a read must start within the bound")
            .expect("mock alive");
    }

    pub fn lock_ids(&self) -> Vec<String> {
        self.0.lock_ids.lock().unwrap().clone()
    }

    pub fn uids(&self) -> Vec<u32> {
        self.0.uids.lock().unwrap().clone()
    }

    pub fn take_hold(counter: &AtomicUsize) -> bool {
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
pub struct LogCapture(pub Arc<Mutex<Vec<u8>>>);

impl LogCapture {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

pub struct LogWriter(pub Arc<Mutex<Vec<u8>>>);

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

pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
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
    pub fn header(&self, name: &str) -> Option<&str> {
        let values = self.headers_named(name);
        assert!(values.len() <= 1, "header {name} repeated: {values:?}");
        values.first().copied()
    }

    pub fn headers_named(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| {
            panic!(
                "body is JSON: {e}: {:?}",
                String::from_utf8_lossy(&self.body)
            )
        })
    }

    pub fn result(&self) -> String {
        self.json()["result"]
            .as_str()
            .expect("result field")
            .to_string()
    }

    pub fn assert_mandatory_headers(&self) {
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

    pub fn assert_json_body(&self) {
        assert_eq!(self.header("content-type"), Some("application/json"));
        assert_eq!(
            self.header("content-length"),
            Some(self.body.len().to_string().as_str())
        );
    }
}

/// Splits `bytes` at the first blank line into a parsed head and the body offset.
pub fn parse_head(bytes: &[u8]) -> Option<(HttpResponse, usize)> {
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

pub fn parse_response(bytes: &[u8]) -> HttpResponse {
    let (mut response, body_start) = parse_head(bytes)
        .unwrap_or_else(|| panic!("no head terminator in {:?}", String::from_utf8_lossy(bytes)));
    response.body = bytes[body_start..].to_vec();
    response
}

pub fn raw_request(method: &str, target: &str, headers: &[(&str, &str)]) -> Vec<u8> {
    let mut out = format!("{method} {target} HTTP/1.1\r\n");
    for (name, value) in headers {
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    out.push_str("\r\n");
    out.into_bytes()
}

pub fn with_identity<'a>(extra: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
    let mut headers = vec![("Host", HOST), ("Tailscale-User-Login", LOGIN)];
    headers.extend_from_slice(extra);
    headers
}

/// What the client observed on a connection that may or may not answer.
#[derive(Debug)]
pub enum Outcome {
    Response(HttpResponse),
    /// Closed (EOF or reset) with no bytes.
    Closed,
    /// Nothing within the bound.
    Silent,
}

pub async fn connect(path: &Path) -> UnixStream {
    timeout(IO_BOUND, UnixStream::connect(path))
        .await
        .expect("connect within the bound")
        .expect("connect")
}

/// One request: connect, write `bytes`, read to EOF. Real I/O; no virtual time passes.
pub async fn exchange(path: &Path, bytes: &[u8]) -> Outcome {
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
pub fn spawn_exchange(path: &Path, bytes: Vec<u8>) -> JoinHandle<Outcome> {
    let path = path.to_path_buf();
    tokio::spawn(async move { exchange(&path, &bytes).await })
}

/// Waits (scheduler rounds only, no clock movement) until the task is finished.
pub async fn finished_soon<T>(task: &JoinHandle<T>) -> bool {
    for _ in 0..POLL_ROUNDS {
        if task.is_finished() {
            return true;
        }
        yield_now().await;
    }
    task.is_finished()
}

pub enum Polled {
    Status(Value),
    Eof,
    Pending,
}

pub struct SseClient {
    pub stream: UnixStream,
    pub buf: Vec<u8>,
    pub head: HttpResponse,
}

impl SseClient {
    /// Non-blocking: a buffered or immediately readable event, EOF, or nothing yet.
    pub fn poll_event(&mut self) -> Polled {
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
    pub async fn expect_event_now(&mut self) -> Value {
        for _ in 0..POLL_ROUNDS {
            match self.poll_event() {
                Polled::Status(v) => return v,
                Polled::Eof => panic!("stream ended while an event was expected"),
                Polled::Pending => yield_now().await,
            }
        }
        panic!("no event arrived without time passing");
    }

    pub async fn expect_state_now(&mut self, state: &str) -> Value {
        let event = self.expect_event_now().await;
        assert_eq!(event["state"], state, "{event}");
        event
    }

    /// No event arrives without time passing.
    pub async fn expect_silence_now(&mut self) {
        settle().await;
        match self.poll_event() {
            Polled::Pending => {}
            Polled::Status(v) => panic!("unexpected event {v}"),
            Polled::Eof => panic!("unexpected end of stream"),
        }
    }

    pub async fn expect_eof_now(&mut self) {
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
    pub async fn await_event(&mut self, h: &Harness, max_ms: u64) -> Option<(Value, u64)> {
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

    pub async fn expect_state_within(&mut self, h: &Harness, state: &str, max_ms: u64) -> Value {
        let (event, elapsed) = self
            .await_event(h, max_ms)
            .await
            .unwrap_or_else(|| panic!("no event within {max_ms} ms (expected {state})"));
        assert_eq!(event["state"], state, "{event} after {elapsed} ms");
        event
    }

    /// Moves the clock by `total_ms` in [`STEP_MS`] steps and asserts no event arrives.
    pub async fn expect_silence_for(&mut self, h: &Harness, total_ms: u64) {
        if let Some((event, elapsed)) = self.await_event(h, total_ms.saturating_sub(STEP_MS)).await
        {
            panic!(
                "unexpected event {event} after {elapsed} ms (silence expected for {total_ms} ms)"
            );
        }
    }
}

pub fn parse_event(frame: &[u8]) -> Value {
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

pub fn checked(event: &Value) -> u64 {
    event["checked_unix_ms"].as_u64().expect("checked_unix_ms")
}

pub fn assert_status_shape(json: &Value) {
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
pub struct FrozenClock {
    pub release: Option<mpsc::Sender<()>>,
}

impl FrozenClock {
    pub fn hold() -> Self {
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

/// What the credential store holds when the harness starts.
#[derive(Clone, Default)]
pub enum StoreSetup {
    /// No store file (zero passkeys).
    #[default]
    Absent,
    /// The owner's synced passkey under [`OWNER_HANDLE`].
    Owner,
    /// These passkeys under [`OWNER_HANDLE`].
    Passkeys(Vec<StoredPasskey>),
}

#[derive(Default)]
pub struct Options {
    pub poll_interval_ms: Option<u64>,
    pub allowed_hosts: Vec<String>,
    pub seq_start: Option<u64>,
    pub allow_unlock: bool,
    /// `rp_id` of the configuration (`None`: passkeys off).
    pub rp_id: Option<String>,
    pub allow_funnel: bool,
    pub store: StoreSetup,
    /// Injected CSPRNG (`None`: the production source).
    pub random: Option<RandomSource>,
}

impl Options {
    /// Passkeys on (`rp_id` = [`HOST`]), unlock enabled, the owner's passkey stored, Funnel
    /// off.
    pub fn passkeys() -> Self {
        Self {
            allow_unlock: true,
            rp_id: Some(HOST.to_string()),
            store: StoreSetup::Owner,
            ..Self::default()
        }
    }

    /// [`Options::passkeys`] plus `allow_funnel = true`.
    pub fn funnel() -> Self {
        Self {
            allow_funnel: true,
            ..Self::passkeys()
        }
    }
}

pub struct Harness {
    pub _dir: TempDir,
    pub dir: PathBuf,
    pub path: PathBuf,
    pub store_path: PathBuf,
    pub owner: Authenticator,
    pub source: MockSource,
    pub clock: Arc<TestClock>,
    pub shutdown: Option<oneshot::Sender<()>>,
    pub server: Option<JoinHandle<Result<(), ServeError>>>,
    pub _frozen: FrozenClock,
    /// Every challenge and cookie value the harness saw (log hygiene needles).
    pub secrets: Mutex<Vec<String>>,
}

impl Harness {
    pub async fn start() -> Self {
        Self::start_with(Options::default()).await
    }

    pub async fn start_with(options: Options) -> Self {
        let frozen = FrozenClock::hold();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("remote.sock");
        let store_path = dir.path().join(CREDENTIALS_FILE_NAME);
        let owner = Authenticator::owner();
        match &options.store {
            StoreSetup::Absent => {}
            StoreSetup::Owner => write_store(&store_path, &[StoredPasskey::of(&owner)]),
            StoreSetup::Passkeys(passkeys) => write_store(&store_path, passkeys),
        }
        let config = RemoteConfig {
            allowed_logins: vec![TailscaleLogin::parse(LOGIN).unwrap()],
            socket_path: path.clone(),
            poll_interval_ms: options.poll_interval_ms.unwrap_or(DEFAULT_POLL_INTERVAL_MS),
            allowed_hosts: options.allowed_hosts,
            allow_unlock: options.allow_unlock,
            auth: AuthConfig {
                rp_id: options.rp_id.clone(),
                allow_funnel: options.allow_funnel,
                credentials_path: Some(store_path.clone()),
            },
            alerts: soos_remote::config::AlertsConfig::default(),
            push: soos_remote::config::PushConfig::default(),
            camera: soos_remote::config::CameraConfig::default(),
        };
        let source = MockSource::unlocked();
        let clock = TestClock::new();
        let clock_fn = Arc::clone(&clock);
        let mut state = ServerState::new(config, UID, source.clone())
            .with_unix_clock(Arc::new(move || clock_fn.now_ms()))
            .with_credentials_path(store_path.clone())
            .with_file_owner_uid(own_uid());
        if let Some(random) = options.random.clone() {
            state = state.with_random(random);
        }
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
            dir: dir.path().to_path_buf(),
            _dir: dir,
            path,
            store_path,
            owner,
            source,
            clock,
            shutdown: Some(tx),
            server: Some(server),
            _frozen: frozen,
            secrets: Mutex::new(Vec::new()),
        }
    }

    /// Moves the virtual clock by `n` ms in one jump, then lets every task run.
    pub async fn advance_ms(&self, n: u64) {
        tokio::time::advance(ms(n)).await;
        settle().await;
    }

    /// Moves the virtual clock by `total` ms in `step` ms steps (periodic timers fire at
    /// every step, as they would in real time).
    pub async fn step_ms(&self, step: u64, total: u64) {
        let mut done = 0;
        while done < total {
            let n = step.min(total - done);
            self.advance_ms(n).await;
            done += n;
        }
    }

    pub async fn raw(&self, bytes: &[u8]) -> HttpResponse {
        match exchange(&self.path, bytes).await {
            Outcome::Response(r) => r,
            Outcome::Closed => panic!("server closed the connection without a response"),
            Outcome::Silent => panic!("no response within {IO_BOUND:?}"),
        }
    }

    pub async fn request(
        &self,
        method: &str,
        target: &str,
        extra: &[(&str, &str)],
    ) -> HttpResponse {
        self.raw(&raw_request(method, target, &with_identity(extra)))
            .await
    }

    pub async fn get(&self, target: &str) -> HttpResponse {
        self.request("GET", target, &[]).await
    }

    pub async fn lock(&self, extra: &[(&str, &str)]) -> HttpResponse {
        let mut headers = vec![("X-Soos-Action", "lock")];
        headers.extend_from_slice(extra);
        self.request("POST", "/api/lock", &headers).await
    }

    pub async fn unlock(&self, extra: &[(&str, &str)]) -> HttpResponse {
        let mut headers = vec![("X-Soos-Action", "unlock")];
        headers.extend_from_slice(extra);
        self.request("POST", "/api/unlock", &headers).await
    }

    pub async fn start_unlock_enabled() -> Self {
        Self::start_with(Options {
            allow_unlock: true,
            ..Options::default()
        })
        .await
    }

    pub async fn try_open_stream(&self, headers: &[(&str, &str)]) -> Result<SseClient, Outcome> {
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

    pub async fn open_stream(&self) -> SseClient {
        match self.try_open_stream(&with_identity(&[])).await {
            Ok(client) => client,
            Err(Outcome::Response(r)) => panic!("stream refused with status {}", r.status),
            Err(Outcome::Closed) => panic!("stream closed without a head"),
            Err(Outcome::Silent) => panic!("no stream head within the bound"),
        }
    }

    pub async fn shutdown(mut self) -> Result<(), ServeError> {
        let tx = self.shutdown.take().unwrap();
        let _ = tx.send(());
        let server = self.server.take().unwrap();
        timeout(IO_BOUND, server)
            .await
            .expect("serve returns after shutdown")
            .expect("serve task not panicked")
    }

    pub async fn await_server(&mut self) -> Result<(), ServeError> {
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

pub fn assert_sse_head(head: &HttpResponse) {
    assert_eq!(head.status, 200);
    assert_eq!(head.header("content-type"), Some("text/event-stream"));
    assert!(
        head.header("content-length").is_none(),
        "no Content-Length on a stream"
    );
    head.assert_mandatory_headers();
}

// ---------------------------------------------------------------------------------------
// Passkey and Funnel extensions
// ---------------------------------------------------------------------------------------

/// The owner's user handle in every pre-provisioned store.
pub const OWNER_HANDLE: [u8; 16] = [0x0d; 16];
/// `Origin` of the web app.
pub const ORIGIN: &str = "https://pc.tail1234.ts.net";

/// Writes a store file (spec §4.4 format, `0600`) holding `passkeys` under [`OWNER_HANDLE`].
pub fn write_store(path: &Path, passkeys: &[StoredPasskey]) {
    write_store_with_handle(path, &OWNER_HANDLE, passkeys);
}

pub fn write_store_with_handle(path: &Path, handle: &[u8], passkeys: &[StoredPasskey]) {
    write_file_mode(path, store_json(handle, passkeys).as_bytes(), 0o600);
}

/// How a request reaches the socket.
#[derive(Clone, Debug)]
pub enum Via {
    /// `tailscale serve` with the allowed identity (no Funnel marker).
    Tailnet,
    /// `tailscale funnel`: no identity, the marker `?1`, the client address set by
    /// `tailscaled`, and optionally the session cookie.
    Funnel { ip: String, cookie: Option<String> },
}

impl Via {
    pub fn funnel(ip: &str) -> Self {
        Self::Funnel {
            ip: ip.to_string(),
            cookie: None,
        }
    }

    pub fn with_cookie(&self, cookie: &str) -> Self {
        match self {
            Self::Tailnet => Self::Tailnet,
            Self::Funnel { ip, .. } => Self::Funnel {
                ip: ip.clone(),
                cookie: Some(cookie.to_string()),
            },
        }
    }

    pub fn headers(&self) -> Vec<(String, String)> {
        match self {
            Self::Tailnet => vec![
                ("Host".to_string(), HOST.to_string()),
                ("Tailscale-User-Login".to_string(), LOGIN.to_string()),
            ],
            Self::Funnel { ip, cookie } => {
                let mut h = vec![
                    ("Host".to_string(), "localhost".to_string()),
                    ("X-Forwarded-Host".to_string(), HOST.to_string()),
                    ("X-Forwarded-Proto".to_string(), "https".to_string()),
                    ("Tailscale-Funnel-Request".to_string(), "?1".to_string()),
                ];
                if !ip.is_empty() {
                    h.push(("X-Forwarded-For".to_string(), ip.clone()));
                }
                if let Some(cookie) = cookie {
                    h.push(("Cookie".to_string(), cookie.clone()));
                }
                h
            }
        }
    }
}

/// Request bytes: `via` headers, then `extra`, then (when `body` is given)
/// `Content-Type: application/json` and its exact `Content-Length`, then the body.
pub fn request_bytes(
    via: &Via,
    method: &str,
    target: &str,
    extra: &[(&str, &str)],
    body: Option<&str>,
) -> Vec<u8> {
    let mut out = format!("{method} {target} HTTP/1.1\r\n");
    for (n, v) in via.headers() {
        out.push_str(&format!("{n}: {v}\r\n"));
    }
    for (n, v) in extra {
        out.push_str(&format!("{n}: {v}\r\n"));
    }
    if let Some(body) = body {
        out.push_str("Content-Type: application/json\r\n");
        out.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    out.push_str("\r\n");
    if let Some(body) = body {
        out.push_str(body);
    }
    out.into_bytes()
}

/// The `Set-Cookie` value turned into the `Cookie` header a browser sends back.
pub fn cookie_of(response: &HttpResponse) -> String {
    let set = response
        .header("set-cookie")
        .unwrap_or_else(|| panic!("no Set-Cookie on {response:?}"));
    set.split(';').next().unwrap().trim().to_string()
}

/// A held connection: the request head (and any `partial` body bytes) is written, the
/// connection stays open.
pub struct Held {
    pub stream: UnixStream,
    pub buf: Vec<u8>,
}

impl Held {
    /// A complete response that needs no clock movement, if one arrives (scheduler rounds
    /// only). `None` = nothing yet.
    pub async fn response_now(&mut self) -> Option<HttpResponse> {
        for _ in 0..POLL_ROUNDS {
            let mut chunk = [0u8; 4096];
            match self.stream.try_read(&mut chunk) {
                Ok(0) => {
                    if self.buf.is_empty() {
                        return None;
                    }
                    return Some(parse_response(&self.buf));
                }
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if let Some((head, start)) = parse_head(&self.buf) {
                        let len: usize = head
                            .header("content-length")
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(0);
                        if self.buf.len() >= start + len {
                            return Some(parse_response(&self.buf));
                        }
                    }
                    yield_now().await;
                }
                Err(_) => return None,
            }
        }
        None
    }

    /// Closed by the server without any response byte (EOF or reset), without clock
    /// movement.
    pub async fn closed_silently_now(&mut self) -> bool {
        for _ in 0..POLL_ROUNDS {
            let mut chunk = [0u8; 4096];
            match self.stream.try_read(&mut chunk) {
                Ok(0) => return self.buf.is_empty(),
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => yield_now().await,
                Err(_) => return self.buf.is_empty(),
            }
        }
        false
    }

    pub async fn send(&mut self, bytes: &[u8]) {
        timeout(IO_BOUND, self.stream.write_all(bytes))
            .await
            .expect("write within the bound")
            .expect("write");
    }
}

impl Harness {
    /// One request through `via`; the connection is closed by the client after the answer.
    pub async fn send(
        &self,
        via: &Via,
        method: &str,
        target: &str,
        extra: &[(&str, &str)],
        body: Option<&str>,
    ) -> HttpResponse {
        self.raw(&request_bytes(via, method, target, extra, body))
            .await
    }

    pub async fn get_via(&self, via: &Via, target: &str) -> HttpResponse {
        self.send(via, "GET", target, &[], None).await
    }

    /// Writes the head of a request declaring `declared` body bytes (none sent) and keeps
    /// the connection open.
    pub async fn hold(
        &self,
        via: &Via,
        method: &str,
        target: &str,
        extra: &[(&str, &str)],
        declared: Option<usize>,
    ) -> Held {
        let mut bytes = request_bytes(via, method, target, extra, None);
        if let Some(len) = declared {
            let cut = bytes.len() - 2;
            let mut head = bytes[..cut].to_vec();
            head.extend_from_slice(
                format!("Content-Type: application/json\r\nContent-Length: {len}\r\n\r\n")
                    .as_bytes(),
            );
            bytes = head;
        }
        let mut stream = connect(&self.path).await;
        timeout(IO_BOUND, stream.write_all(&bytes))
            .await
            .expect("write within the bound")
            .expect("write");
        settle().await;
        Held {
            stream,
            buf: Vec::new(),
        }
    }

    /// A passkey ceremony POST: `X-Soos-Action: action`, `Origin: https://<rp_id>`.
    pub async fn ceremony(
        &self,
        via: &Via,
        path: &str,
        action: &str,
        body: Option<&str>,
    ) -> HttpResponse {
        self.send(
            via,
            "POST",
            path,
            &[("X-Soos-Action", action), ("Origin", ORIGIN)],
            body,
        )
        .await
    }

    pub fn remember(&self, secret: &str) {
        self.secrets.lock().unwrap().push(secret.to_string());
    }

    /// The challenge of an options response (remembered as a secret).
    pub fn challenge_of(&self, response: &HttpResponse) -> String {
        assert_eq!(
            response.status,
            200,
            "options refused: {}",
            response.result_or_body()
        );
        let challenge = response.json()["challenge"]
            .as_str()
            .expect("challenge")
            .to_string();
        self.remember(&challenge);
        challenge
    }

    pub async fn unlock_options(&self, via: &Via) -> HttpResponse {
        self.ceremony(via, "/api/auth/unlock/options", "unlock-options", None)
            .await
    }

    pub async fn login_options(&self, via: &Via) -> HttpResponse {
        self.ceremony(via, "/api/auth/login/options", "login-options", None)
            .await
    }

    pub async fn auth_state(&self, via: &Via) -> HttpResponse {
        self.get_via(via, "/api/auth/state").await
    }

    pub async fn logout(&self, via: &Via) -> HttpResponse {
        self.ceremony(via, "/api/auth/logout", "logout", None).await
    }

    /// `POST /api/unlock` with `X-Soos-Action: unlock` and an optional JSON body.
    pub async fn post_unlock(&self, via: &Via, body: Option<&str>) -> HttpResponse {
        self.send(
            via,
            "POST",
            "/api/unlock",
            &[("X-Soos-Action", "unlock")],
            body,
        )
        .await
    }

    pub async fn post_login(&self, via: &Via, body: &str) -> HttpResponse {
        self.ceremony(via, "/api/auth/login/verify", "login", Some(body))
            .await
    }

    /// An assertion body over `challenge` (a `webauthn.get` client data with the app
    /// origin), signed by `auth` with `flags` and `count`, carrying `handle`.
    pub fn assertion(
        &self,
        auth: &Authenticator,
        challenge: &str,
        flags: u8,
        count: u32,
        handle: Option<&[u8]>,
    ) -> String {
        let client = client_data("webauthn.get", challenge, ORIGIN, "");
        let ad = assertion_auth_data(HOST, flags, count);
        let sig = auth.sign(&ad, &client);
        assertion_body(&auth.credential_id, &client, &ad, &sig, handle)
    }

    /// The default valid assertion of `auth` (its UV flags, counter 0, owner handle).
    pub fn valid_assertion(&self, auth: &Authenticator, challenge: &str) -> String {
        self.assertion(auth, challenge, auth.flags(), 0, Some(&OWNER_HANDLE))
    }

    /// Unlock options then `POST /api/unlock` with a valid assertion of `auth`.
    pub async fn unlock_with(&self, via: &Via, auth: &Authenticator) -> HttpResponse {
        let options = self.unlock_options(via).await;
        if options.status != 200 {
            return options;
        }
        let challenge = self.challenge_of(&options);
        let body = self.valid_assertion(auth, &challenge);
        self.post_unlock(via, Some(&body)).await
    }

    /// Login options then verify with a valid assertion of `auth` from `ip`.
    pub async fn login(&self, via: &Via, auth: &Authenticator) -> HttpResponse {
        let options = self.login_options(via).await;
        if options.status != 200 {
            return options;
        }
        let challenge = self.challenge_of(&options);
        let body = self.valid_assertion(auth, &challenge);
        let response = self.post_login(via, &body).await;
        if let Some(set) = response.header("set-cookie") {
            self.remember(set);
        }
        response
    }

    /// A logged-in Funnel caller at `ip` (owner passkey).
    pub async fn session(&self, ip: &str) -> Via {
        let via = Via::funnel(ip);
        let response = self.login(&via, &self.owner.clone()).await;
        assert_eq!(response.status, 200, "login: {}", response.result_or_body());
        assert_eq!(response.result(), "logged_in");
        via.with_cookie(&cookie_of(&response))
    }

    /// The Unix time (s) of the injected clock.
    pub fn now_unix_s(&self) -> u64 {
        self.clock.now_ms() / 1000
    }

    /// Writes the enrollment code file for the normalized `code` in the socket directory.
    pub fn write_code(&self, code: &str, expires_unix_s: u64) {
        self.remember(code);
        write_file_mode(
            &self.dir.join(soos_remote::ENROLL_CODE_FILE_NAME),
            code_file_line(&code_hash(code), expires_unix_s).as_bytes(),
            0o600,
        );
    }

    pub fn code_file_exists(&self) -> bool {
        self.dir.join(soos_remote::ENROLL_CODE_FILE_NAME).exists()
    }

    pub async fn register_options(&self, code: &str) -> HttpResponse {
        let body = format!("{{\"code\":\"{code}\"}}");
        self.ceremony(
            &Via::Tailnet,
            "/api/auth/register/options",
            "register-options",
            Some(&body),
        )
        .await
    }

    /// A registration body of `auth` for `challenge` (`webauthn.create`, UV, `none`).
    pub fn registration_for(&self, auth: &Authenticator, challenge: &str) -> String {
        let client = client_data("webauthn.create", challenge, ORIGIN, "");
        let flags = auth.flags() | AT;
        registration_body(
            &auth.credential_id,
            &client,
            &registration(auth, HOST, flags),
        )
    }

    pub async fn register_verify(&self, body: &str) -> HttpResponse {
        self.ceremony(
            &Via::Tailnet,
            "/api/auth/register/verify",
            "register",
            Some(body),
        )
        .await
    }

    /// Full tailnet registration of `auth` with `code`; returns (options, verify).
    pub async fn register(&self, auth: &Authenticator, code: &str) -> (HttpResponse, HttpResponse) {
        let options = self.register_options(code).await;
        if options.status != 200 {
            let copy = HttpResponse {
                status: options.status,
                headers: options.headers.clone(),
                body: options.body.clone(),
            };
            return (options, copy);
        }
        let challenge = self.challenge_of(&options);
        let body = self.registration_for(auth, &challenge);
        let verify = self.register_verify(&body).await;
        (options, verify)
    }

    pub async fn open_stream_via(&self, via: &Via) -> Result<SseClient, Outcome> {
        let headers = via.headers();
        let borrowed: Vec<(&str, &str)> = headers
            .iter()
            .map(|(n, v)| (n.as_str(), v.as_str()))
            .collect();
        self.try_open_stream(&borrowed).await
    }

    /// The store file parsed as JSON.
    pub fn store_json_value(&self) -> Value {
        serde_json::from_slice(&std::fs::read(&self.store_path).expect("store file"))
            .expect("store JSON")
    }
}

impl HttpResponse {
    /// `result` when the body is the JSON result shape, the raw body otherwise (diagnostics).
    pub fn result_or_body(&self) -> String {
        serde_json::from_slice::<Value>(&self.body)
            .ok()
            .and_then(|v| v["result"].as_str().map(str::to_string))
            .unwrap_or_else(|| format!("{} {:?}", self.status, String::from_utf8_lossy(&self.body)))
    }
}

impl SseClient {
    /// Moves the clock in [`STEP_MS`] steps, discarding events, until the stream ends;
    /// returns the virtual time consumed, or `None` if it is still open after `max_ms`.
    pub async fn ends_within(&mut self, h: &Harness, max_ms: u64) -> Option<u64> {
        let mut elapsed = 0;
        loop {
            for _ in 0..SETTLE_ROUNDS {
                match self.poll_event() {
                    Polled::Eof => return Some(elapsed),
                    Polled::Status(_) => {}
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
}
