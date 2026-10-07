//! Contract tests of GitHub #345 for the `soos-remote` daemon preview client (spec §5,
//! tests 17–22 and 62, matrix row RLC12).
//!
//! A fake `soos-daemon` listens on a socket inside a `tempfile::TempDir`, decodes every
//! request with `soos-protocol` and answers from a per-test script. The fake runs as tasks of
//! the test runtime. Tokio's auto-advancing paused clock is not used: it can fire timers while
//! a Unix-socket readiness event is still pending. Exchanges run on the real clock, and the
//! long connection lifetime is skipped with `jump_clock` while no exchange is in flight.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::enum_variant_names,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use soos_protocol::codec::{encode, encode_preview};
use soos_protocol::message::{decode_client_message, ClientMessage, FrameFormat};
use soos_protocol::types::{
    PreviewResponse, ReasonClass, Request, RequestKind, Response, Verdict, CURRENT_VERSION,
    MAX_MESSAGE_SIZE, MAX_PREVIEW_MESSAGE_SIZE,
};
use soos_remote::camera_ipc::{DaemonPreviewClient, PreviewError, PreviewFrame, PreviewSource};
use soos_remote::{
    CAMERA_DAEMON_BACKOFF_MIN_MS, CAMERA_DAEMON_CLOSE_WAIT_MS, CAMERA_DAEMON_CONNECT_TIMEOUT_MS,
    CAMERA_DAEMON_IO_TIMEOUT_MS, CAMERA_DAEMON_MAX_REQUESTS_PER_CONNECTION,
    CAMERA_DAEMON_RECONNECT_AFTER_MS, CAMERA_DAEMON_SERVICE,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Instant;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn own_uid() -> u32 {
    nix::unistd::getuid().as_raw()
}

fn ms<T>(value: T) -> Duration
where
    u64: TryFrom<T>,
    <u64 as TryFrom<T>>::Error: std::fmt::Debug,
{
    Duration::from_millis(u64::try_from(value).unwrap())
}

/// `CLOCK_MONOTONIC` in nanoseconds, the clock the real daemon stamps responses with.
fn mono_now_ns() -> u64 {
    let ts = nix::time::clock_gettime(nix::time::ClockId::CLOCK_MONOTONIC).unwrap();
    u64::try_from(ts.tv_sec()).unwrap() * 1_000_000_000 + u64::try_from(ts.tv_nsec()).unwrap()
}

/// Moves the tokio clock forward by `by` while nothing is in flight, then resumes real time
/// (current-thread runtime, `test-util`).
async fn jump_clock(by: Duration) {
    tokio::time::pause();
    tokio::time::advance(by).await;
    tokio::time::resume();
}

fn expect_err(result: Result<PreviewFrame, PreviewError>) -> PreviewError {
    match result {
        Ok(_) => panic!("expected a preview error, got a frame"),
        Err(err) => err,
    }
}

fn length_prefix(declared: usize) -> Vec<u8> {
    u32::try_from(declared).unwrap().to_be_bytes().to_vec()
}

fn rgb_fixture(width: u32, height: u32) -> Vec<u8> {
    (0..width * height * 3).map(|i| (i % 251) as u8).collect()
}

// ---------------------------------------------------------------------------
// Fake daemon
// ---------------------------------------------------------------------------

/// What the fake does with one decoded request.
#[derive(Clone)]
enum Reply {
    /// A valid `PreviewResponse`.
    Frame {
        sequence: u64,
        width: u32,
        height: u32,
        format: u8,
        data: Vec<u8>,
    },
    /// A fresh standard `Response` bound to the request nonce.
    Refuse(Verdict, ReasonClass),
    /// A standard `Response` with explicit stamps (relative to now) and nonce binding.
    Stamped {
        verdict: Verdict,
        reason: ReasonClass,
        issued_offset_ns: i64,
        expires_offset_ns: i64,
        foreign_nonce: bool,
    },
    /// Bytes written verbatim; the connection stays open.
    Raw(Vec<u8>),
    /// Bytes written verbatim, then the fake closes the connection.
    RawThenClose(Vec<u8>),
    /// No answer at all; the connection stays open.
    Silent,
    /// The fake closes the connection without any reply byte.
    CloseNoReply,
}

#[derive(Clone)]
struct Action {
    reply: Reply,
    /// Close the connection right after the reply (daemon idle close).
    close_after: bool,
}

fn keep(reply: Reply) -> Action {
    Action {
        reply,
        close_after: false,
    }
}

fn frame(sequence: u64) -> Reply {
    Reply::Frame {
        sequence,
        width: 16,
        height: 16,
        format: 0,
        data: rgb_fixture(16, 16),
    }
}

/// What the fake does when it observes the client's EOF (write shutdown or close).
#[derive(Clone, Copy)]
enum EofMode {
    /// Close its side at once (the real daemon).
    CloseNow,
    /// Close its side after this delay.
    CloseAfter(Duration),
    /// Never close its side (a daemon slower than `CAMERA_DAEMON_CLOSE_WAIT_MS`).
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ev {
    Accepted(usize),
    Request(usize),
    ClientEof(usize),
    Closed(usize),
}

/// (connection index, request index on that connection, global request index) → action.
type Script = Arc<dyn Fn(usize, usize, usize) -> Action + Send + Sync>;

#[derive(Default)]
struct LogInner {
    events: Vec<(Ev, Instant)>,
    requests: Vec<(usize, Request, FrameFormat)>,
    accepted: usize,
    open: usize,
    max_open: usize,
}

struct Log {
    inner: Mutex<LogInner>,
    request_count: watch::Sender<usize>,
}

impl Log {
    fn push(&self, ev: Ev) {
        let mut inner = self.inner.lock().unwrap();
        match ev {
            Ev::Accepted(_) => {
                inner.accepted += 1;
                inner.open += 1;
                inner.max_open = inner.max_open.max(inner.open);
            }
            Ev::Closed(_) => inner.open -= 1,
            Ev::Request(_) | Ev::ClientEof(_) => {}
        }
        inner.events.push((ev, Instant::now()));
    }

    fn record_request(&self, conn: usize, req: Request, format: FrameFormat) -> usize {
        let global = {
            let mut inner = self.inner.lock().unwrap();
            inner.requests.push((conn, req, format));
            inner.events.push((Ev::Request(conn), Instant::now()));
            inner.requests.len() - 1
        };
        self.request_count.send_modify(|n| *n += 1);
        global
    }
}

struct FakeDaemon {
    path: PathBuf,
    log: Arc<Log>,
    accept_task: JoinHandle<()>,
    _dir: tempfile::TempDir,
}

impl Drop for FakeDaemon {
    fn drop(&mut self) {
        self.accept_task.abort();
    }
}

impl FakeDaemon {
    fn start(script: Script, eof_mode: EofMode) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let (request_count, _) = watch::channel(0usize);
        let log = Arc::new(Log {
            inner: Mutex::new(LogInner::default()),
            request_count,
        });
        let accept_log = Arc::clone(&log);
        let accept_task = tokio::spawn(async move {
            let mut conn = 0usize;
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                accept_log.push(Ev::Accepted(conn));
                tokio::spawn(serve_connection(
                    stream,
                    conn,
                    Arc::clone(&accept_log),
                    Arc::clone(&script),
                    eof_mode,
                ));
                conn += 1;
            }
        });
        Self {
            path,
            log,
            accept_task,
            _dir: dir,
        }
    }

    fn scripted(script: impl Fn(usize, usize, usize) -> Action + Send + Sync + 'static) -> Self {
        Self::start(Arc::new(script), EofMode::CloseNow)
    }

    fn always(reply: Reply) -> Self {
        Self::scripted(move |_, _, _| keep(reply.clone()))
    }

    fn client(&self) -> DaemonPreviewClient {
        DaemonPreviewClient::new(self.path.clone(), own_uid()).with_expected_daemon_uid(own_uid())
    }

    fn accepted(&self) -> usize {
        self.log.inner.lock().unwrap().accepted
    }

    fn max_open(&self) -> usize {
        self.log.inner.lock().unwrap().max_open
    }

    fn request_conns(&self) -> Vec<usize> {
        self.log
            .inner
            .lock()
            .unwrap()
            .requests
            .iter()
            .map(|(c, _, _)| *c)
            .collect()
    }

    fn request(&self, index: usize) -> Request {
        self.log.inner.lock().unwrap().requests[index].1.clone()
    }

    fn request_format(&self, index: usize) -> FrameFormat {
        self.log.inner.lock().unwrap().requests[index].2
    }

    fn request_total(&self) -> usize {
        self.log.inner.lock().unwrap().requests.len()
    }

    fn event_at(&self, ev: Ev) -> Option<(usize, Instant)> {
        self.log
            .inner
            .lock()
            .unwrap()
            .events
            .iter()
            .enumerate()
            .find(|(_, (e, _))| *e == ev)
            .map(|(i, (_, t))| (i, *t))
    }

    async fn wait_requests(&self, n: usize) {
        let mut rx = self.log.request_count.subscribe();
        rx.wait_for(|count| *count >= n).await.unwrap();
    }
}

fn encode_reply(reply: &Reply, req: &Request) -> Vec<u8> {
    let now = mono_now_ns();
    let offset = |delta: i64| -> u64 {
        if delta >= 0 {
            now + delta as u64
        } else {
            now - delta.unsigned_abs()
        }
    };
    match reply {
        Reply::Frame {
            sequence,
            width,
            height,
            format,
            data,
        } => encode_preview(&PreviewResponse {
            version: CURRENT_VERSION,
            sequence: *sequence,
            width: *width,
            height: *height,
            format: *format,
            timestamp_monotonic_ns: now,
            data: data.clone(),
        })
        .unwrap(),
        Reply::Refuse(verdict, reason) => encode(&Response {
            version: CURRENT_VERSION,
            request_id: req.request_id,
            verdict: *verdict,
            reason_class: *reason,
            issued_monotonic_ns: now,
            expires_monotonic_ns: now + 2_000_000_000,
        })
        .unwrap(),
        Reply::Stamped {
            verdict,
            reason,
            issued_offset_ns,
            expires_offset_ns,
            foreign_nonce,
        } => {
            let mut request_id = req.request_id;
            if *foreign_nonce {
                request_id[0] ^= 0xFF;
            }
            encode(&Response {
                version: CURRENT_VERSION,
                request_id,
                verdict: *verdict,
                reason_class: *reason,
                issued_monotonic_ns: offset(*issued_offset_ns),
                expires_monotonic_ns: offset(*expires_offset_ns),
            })
            .unwrap()
        }
        Reply::Raw(bytes) | Reply::RawThenClose(bytes) => bytes.clone(),
        Reply::Silent | Reply::CloseNoReply => Vec::new(),
    }
}

async fn serve_connection(
    mut stream: UnixStream,
    conn: usize,
    log: Arc<Log>,
    script: Script,
    eof_mode: EofMode,
) {
    let mut on_conn = 0usize;
    loop {
        let mut len = [0u8; 4];
        if stream.read_exact(&mut len).await.is_err() {
            log.push(Ev::ClientEof(conn));
            match eof_mode {
                EofMode::CloseNow => {}
                EofMode::CloseAfter(delay) => tokio::time::sleep(delay).await,
                EofMode::Never => {
                    let _held = stream;
                    std::future::pending::<()>().await;
                    return;
                }
            }
            log.push(Ev::Closed(conn));
            return;
        }
        let declared = u32::from_be_bytes(len) as usize;
        assert!(
            declared > 0 && declared <= MAX_MESSAGE_SIZE,
            "a preview request must fit MAX_MESSAGE_SIZE"
        );
        let mut payload = vec![0u8; declared];
        stream.read_exact(&mut payload).await.unwrap();
        let (message, format) = decode_client_message(&payload).expect("decodable request");
        let ClientMessage::Request(req) = message else {
            panic!("the camera client must send a Request, not an Event");
        };
        let global = log.record_request(conn, req.clone(), format);
        let action = script(conn, on_conn, global);
        on_conn += 1;

        let bytes = encode_reply(&action.reply, &req);
        let closes = action.close_after
            || matches!(action.reply, Reply::RawThenClose(_) | Reply::CloseNoReply);
        if closes {
            // Recorded before the bytes leave, so a reconnect can never be logged first.
            log.push(Ev::Closed(conn));
        }
        if !bytes.is_empty() && stream.write_all(&bytes).await.is_err() {
            if !closes {
                log.push(Ev::Closed(conn));
            }
            return;
        }
        if closes {
            drop(stream);
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// 17
// ---------------------------------------------------------------------------

/// RLC12: one tagged `PreviewFrame` request per exchange with a fresh 32-byte nonce.
#[tokio::test]
async fn test_rlc_client_sends_preview_request() {
    let fake = FakeDaemon::always(frame(1));
    let mut client = fake.client();
    client.next_frame().await.unwrap();
    client.next_frame().await.unwrap();

    assert_eq!(fake.request_total(), 2);
    for index in 0..2 {
        let req = fake.request(index);
        assert_eq!(req.version, CURRENT_VERSION);
        assert_eq!(req.kind, RequestKind::PreviewFrame);
        assert_eq!(req.uid_hint, own_uid());
        assert_eq!(req.service, CAMERA_DAEMON_SERVICE);
        assert_eq!(req.service, "soos-remote");
        assert_eq!(req.deadline_monotonic_ns, u64::MAX);
        assert_eq!(req.request_id.len(), 32);
        assert_ne!(req.request_id, [0u8; 32], "nonce must come from the CSPRNG");
        assert_eq!(fake.request_format(index), FrameFormat::Tagged);
    }
    assert_ne!(
        fake.request(0).request_id,
        fake.request(1).request_id,
        "every request carries a fresh nonce"
    );
}

// ---------------------------------------------------------------------------
// 18
// ---------------------------------------------------------------------------

async fn one_shot(reply: Reply) -> Result<PreviewFrame, PreviewError> {
    let fake = FakeDaemon::always(reply);
    let mut client = fake.client();
    client.next_frame().await
}

/// RLC12: daemon verdicts map to `PreviewError`; nonce and freshness are enforced.
#[tokio::test]
async fn test_rlc_client_maps_daemon_replies() {
    let cases = [
        (
            Reply::Refuse(Verdict::ProtocolError, ReasonClass::UidMismatch),
            PreviewError::Refused,
        ),
        (
            Reply::Refuse(Verdict::Deny, ReasonClass::UidMismatch),
            PreviewError::Refused,
        ),
        (
            Reply::Refuse(Verdict::ProtocolError, ReasonClass::RateLimited),
            PreviewError::RateLimited,
        ),
        (
            Reply::Refuse(Verdict::Unavailable, ReasonClass::InternalError),
            PreviewError::Unavailable,
        ),
        (
            Reply::Refuse(Verdict::Allow, ReasonClass::FaceMatch),
            PreviewError::Protocol,
        ),
    ];
    for (reply, expected) in cases {
        assert_eq!(expect_err(one_shot(reply).await), expected);
    }

    // Nonce mismatch: a refusal for another request is never a verdict.
    let foreign = Reply::Stamped {
        verdict: Verdict::ProtocolError,
        reason: ReasonClass::UidMismatch,
        issued_offset_ns: 0,
        expires_offset_ns: 2_000_000_000,
        foreign_nonce: true,
    };
    assert_eq!(expect_err(one_shot(foreign).await), PreviewError::Protocol);

    // Stale stamp (expired one second ago).
    let stale = Reply::Stamped {
        verdict: Verdict::ProtocolError,
        reason: ReasonClass::UidMismatch,
        issued_offset_ns: -3_000_000_000,
        expires_offset_ns: -1_000_000_000,
        foreign_nonce: false,
    };
    assert_eq!(expect_err(one_shot(stale).await), PreviewError::Protocol);

    // Future-dated stamp (one second ahead, far beyond the 10 ms skew bound).
    let future = Reply::Stamped {
        verdict: Verdict::ProtocolError,
        reason: ReasonClass::UidMismatch,
        issued_offset_ns: 1_000_000_000,
        expires_offset_ns: 3_000_000_000,
        foreign_nonce: false,
    };
    assert_eq!(expect_err(one_shot(future).await), PreviewError::Protocol);

    // A valid preview frame is copied out field by field.
    let data = rgb_fixture(32, 24);
    let reply = Reply::Frame {
        sequence: 4242,
        width: 32,
        height: 24,
        format: 0,
        data: data.clone(),
    };
    let Ok(got) = one_shot(reply).await else {
        panic!("a valid PreviewResponse must yield a frame");
    };
    assert_eq!(got.sequence, 4242);
    assert_eq!(got.width, 32);
    assert_eq!(got.height, 24);
    assert_eq!(got.format, 0);
    assert_eq!(got.data.as_slice(), data.as_slice());

    // The explicit empty preview is a frame (the view loop treats it as "camera waking").
    let empty = Reply::Frame {
        sequence: 0,
        width: 0,
        height: 0,
        format: 255,
        data: Vec::new(),
    };
    let Ok(got) = one_shot(empty).await else {
        panic!("an empty preview is not an error of the client");
    };
    assert_eq!(got.format, 255);
    assert!(got.data.is_empty());
}

// ---------------------------------------------------------------------------
// 19
// ---------------------------------------------------------------------------

/// RLC12/RLC-S6: the declared length is checked before any allocation or payload read.
#[tokio::test]
async fn test_rlc_client_bounds_replies() {
    // Declared 0.
    assert_eq!(
        expect_err(one_shot(Reply::Raw(length_prefix(0))).await),
        PreviewError::Protocol
    );
    // Declared MAX_PREVIEW_MESSAGE_SIZE + 1: refused at once (no payload is ever sent, so a
    // client that tried to read it would time out with `Io` instead).
    let started = std::time::Instant::now();
    assert_eq!(
        expect_err(one_shot(Reply::Raw(length_prefix(MAX_PREVIEW_MESSAGE_SIZE + 1))).await),
        PreviewError::Protocol
    );
    assert!(started.elapsed() < ms(CAMERA_DAEMON_IO_TIMEOUT_MS));
    // u32::MAX declared.
    assert_eq!(
        expect_err(one_shot(Reply::Raw(u32::MAX.to_be_bytes().to_vec())).await),
        PreviewError::Protocol
    );
    // Truncated payload: 100 bytes declared, 10 sent, then EOF.
    let mut truncated = length_prefix(100);
    truncated.extend_from_slice(&[0u8; 10]);
    assert_eq!(
        expect_err(one_shot(Reply::RawThenClose(truncated)).await),
        PreviewError::Io
    );
    // Garbage that is neither a Response nor a PreviewResponse.
    let mut garbage = length_prefix(8);
    garbage.extend_from_slice(&[0xFF; 8]);
    assert_eq!(
        expect_err(one_shot(Reply::Raw(garbage)).await),
        PreviewError::Protocol
    );
}

// ---------------------------------------------------------------------------
// 20
// ---------------------------------------------------------------------------

/// RLC12/RLC-S5: many frames share one connection.
#[tokio::test]
async fn test_rlc_client_holds_at_most_one_connection() {
    let fake = FakeDaemon::scripted(|_, _, g| keep(frame(g as u64 + 1)));
    let mut client = fake.client();
    for _ in 0..20 {
        client.next_frame().await.unwrap();
    }
    assert_eq!(
        fake.accepted(),
        1,
        "frames must reuse one daemon connection"
    );
    assert_eq!(fake.max_open(), 1);

    // Daemon-side close after a reply: the next call reconnects and succeeds.
    let fake = FakeDaemon::scripted(|conn, _, g| Action {
        reply: frame(g as u64 + 1),
        close_after: conn == 0,
    });
    let mut client = fake.client();
    client.next_frame().await.unwrap();
    client.next_frame().await.unwrap();
    assert_eq!(fake.accepted(), 2);
    assert_eq!(fake.max_open(), 1);
    assert_eq!(*fake.request_conns().last().unwrap(), 1);
}

/// F5: a proactive reconnect after `CAMERA_DAEMON_RECONNECT_AFTER_MS` half-closes the old
/// connection and waits for the daemon's close before connecting the new one.
#[tokio::test]
async fn test_rlc_client_holds_at_most_one_connection_reconnects_after_lifetime() {
    // The fake closes its side 100 ms after the client's EOF: a client that connects before
    // the old permit is released is caught by the event order.
    let fake = FakeDaemon::start(
        Arc::new(|_, _, g| keep(frame(g as u64 + 1))),
        EofMode::CloseAfter(Duration::from_millis(100)),
    );
    let mut client = fake.client();
    client.next_frame().await.unwrap();
    client.next_frame().await.unwrap();
    assert_eq!(fake.accepted(), 1);

    jump_clock(ms(CAMERA_DAEMON_RECONNECT_AFTER_MS) + Duration::from_millis(1)).await;
    client.next_frame().await.unwrap();

    assert_eq!(fake.accepted(), 2, "an old connection must be replaced");
    assert_eq!(fake.request_conns(), vec![0, 0, 1]);
    let (eof_index, _) = fake
        .event_at(Ev::ClientEof(0))
        .expect("the client must shut down its write side of the old connection");
    let (closed_index, _) = fake.event_at(Ev::Closed(0)).unwrap();
    let (accept_index, _) = fake.event_at(Ev::Accepted(1)).unwrap();
    assert!(
        eof_index < accept_index,
        "write shutdown must be observed before the new connection"
    );
    assert!(
        closed_index < accept_index,
        "the new connection must wait for the daemon to close the old one"
    );
    assert_eq!(fake.max_open(), 1);
}

/// Tolerance of the fake-side close-wait measurement (B6).
const CLOSE_WAIT_TOLERANCE_MS: u64 = 10;

/// F5: when the daemon never closes the old connection, the client waits
/// `CAMERA_DAEMON_CLOSE_WAIT_MS` (bounded) before it connects again.
#[tokio::test]
async fn test_rlc_client_holds_at_most_one_connection_bounded_close_wait() {
    let fake = FakeDaemon::start(
        Arc::new(|_, _, g| keep(frame(g as u64 + 1))),
        EofMode::Never,
    );
    let mut client = fake.client();
    client.next_frame().await.unwrap();

    jump_clock(ms(CAMERA_DAEMON_RECONNECT_AFTER_MS) + Duration::from_millis(1)).await;
    let before = Instant::now();
    client.next_frame().await.unwrap();
    let took = before.elapsed();

    let (_, eof_at) = fake.event_at(Ev::ClientEof(0)).expect("write shutdown");
    let (_, accept_at) = fake.event_at(Ev::Accepted(1)).unwrap();
    // B6 (auditor): the client starts its close-wait timer when it shuts its write half
    // down, i.e. before the fake records the EOF; the fake-side measurement therefore gets
    // a 10 ms scheduling tolerance, while the client-side measurement (from the start of the
    // call, which precedes the shutdown) is strict.
    assert!(
        accept_at.duration_since(eof_at) + ms(CLOSE_WAIT_TOLERANCE_MS)
            >= ms(CAMERA_DAEMON_CLOSE_WAIT_MS),
        "the client must wait for the daemon's EOF (bounded) before connecting"
    );
    assert!(
        accept_at.duration_since(before) >= ms(CAMERA_DAEMON_CLOSE_WAIT_MS),
        "the next connection is accepted only after the full close wait"
    );
    assert!(
        took >= ms(CAMERA_DAEMON_CLOSE_WAIT_MS),
        "the call includes the full close wait"
    );
    assert!(
        took < ms(CAMERA_DAEMON_CLOSE_WAIT_MS) + ms(CAMERA_DAEMON_CONNECT_TIMEOUT_MS),
        "the close wait is bounded by CAMERA_DAEMON_CLOSE_WAIT_MS"
    );
}

/// F5 + request cap: after `CAMERA_DAEMON_MAX_REQUESTS_PER_CONNECTION` requests the client
/// replaces its connection (graceful close first).
#[tokio::test]
async fn test_rlc_client_holds_at_most_one_connection_request_cap() {
    let fake = FakeDaemon::start(
        Arc::new(|_, _, g| keep(frame(g as u64 + 1))),
        EofMode::CloseAfter(Duration::from_millis(50)),
    );
    let mut client = fake.client();
    let cap = usize::try_from(CAMERA_DAEMON_MAX_REQUESTS_PER_CONNECTION).unwrap();
    for _ in 0..=cap {
        client.next_frame().await.unwrap();
    }
    let conns = fake.request_conns();
    assert_eq!(conns.len(), cap + 1);
    assert_eq!(conns.iter().filter(|c| **c == 0).count(), cap);
    assert_eq!(conns[cap], 1);
    let (eof_index, _) = fake.event_at(Ev::ClientEof(0)).expect("write shutdown");
    let (closed_index, _) = fake.event_at(Ev::Closed(0)).unwrap();
    let (accept_index, _) = fake.event_at(Ev::Accepted(1)).unwrap();
    assert!(eof_index < accept_index);
    assert!(closed_index < accept_index);
    assert_eq!(fake.max_open(), 1);
}

/// §5.3: a cancelled `next_frame` never leaves a half-read connection for reuse.
#[tokio::test]
async fn test_rlc_client_holds_at_most_one_connection_after_cancel() {
    let fake = FakeDaemon::scripted(|_, _, g| {
        if g == 0 {
            keep(Reply::Silent)
        } else {
            keep(frame(g as u64))
        }
    });
    let mut client = fake.client();
    {
        let pending = client.next_frame();
        tokio::select! {
            _ = pending => panic!("the silent fake never answers"),
            () = fake.wait_requests(1) => {}
        }
    }
    client.next_frame().await.unwrap();
    assert_eq!(
        fake.request_conns(),
        vec![0, 1],
        "a fresh connection after a cancel"
    );
}

// ---------------------------------------------------------------------------
// 21
// ---------------------------------------------------------------------------

/// RLC12/RLC-S5: the daemon peer must be root (uid 0) in production.
#[tokio::test]
async fn test_rlc_client_refuses_unexpected_daemon_uid() {
    let fake = FakeDaemon::always(frame(1));
    if own_uid() != 0 {
        let mut default_client = DaemonPreviewClient::new(fake.path.clone(), own_uid());
        assert_eq!(
            expect_err(default_client.next_frame().await),
            PreviewError::Io
        );
        assert_eq!(
            fake.request_total(),
            0,
            "no request may be sent to a non-root listener"
        );
    }
    let mut client = fake.client();
    assert!(client.next_frame().await.is_ok());
}

// ---------------------------------------------------------------------------
// 22
// ---------------------------------------------------------------------------

/// RLC12/RLC-S6: one exchange is bounded by `CAMERA_DAEMON_IO_TIMEOUT_MS`.
#[tokio::test]
async fn test_rlc_client_exchange_is_time_bounded() {
    let upper = ms(CAMERA_DAEMON_IO_TIMEOUT_MS)
        + ms(CAMERA_DAEMON_CONNECT_TIMEOUT_MS)
        + ms(CAMERA_DAEMON_CLOSE_WAIT_MS);

    // Never answers.
    let fake = FakeDaemon::always(Reply::Silent);
    let mut client = fake.client();
    let start = Instant::now();
    assert_eq!(expect_err(client.next_frame().await), PreviewError::Io);
    let took = start.elapsed();
    assert!(took >= ms(CAMERA_DAEMON_IO_TIMEOUT_MS), "took {took:?}");
    assert!(took < upper, "took {took:?}");

    // Answers a partial reply, then stalls.
    let mut partial = length_prefix(100);
    partial.extend_from_slice(&[0u8; 10]);
    let fake = FakeDaemon::always(Reply::Raw(partial));
    let mut client = fake.client();
    let start = Instant::now();
    assert_eq!(expect_err(client.next_frame().await), PreviewError::Io);
    let took = start.elapsed();
    assert!(took >= ms(CAMERA_DAEMON_IO_TIMEOUT_MS), "took {took:?}");
    assert!(took < upper, "took {took:?}");
}

// ---------------------------------------------------------------------------
// 62
// ---------------------------------------------------------------------------

/// F12: an idle close of a reused connection is retried once, at once, with a new nonce.
#[tokio::test]
async fn test_rlc_client_retries_once_after_idle_close() {
    // (a) The daemon closes a served connection right after its reply.
    let fake = FakeDaemon::scripted(|conn, _, g| Action {
        reply: frame(g as u64 + 1),
        close_after: conn == 0,
    });
    let mut client = fake.client();
    client.next_frame().await.unwrap();
    let start = Instant::now();
    client.next_frame().await.unwrap();
    assert!(
        start.elapsed() < ms(CAMERA_DAEMON_BACKOFF_MIN_MS),
        "the retry is immediate (no back-off)"
    );
    assert_eq!(fake.accepted(), 2);
    assert_eq!(fake.max_open(), 1);

    // (b) The daemon reads the request on the reused connection, then closes without a reply
    // byte: the retry on a fresh connection carries a new nonce.
    let fake = FakeDaemon::scripted(|conn, on_conn, g| match (conn, on_conn) {
        (0, 0) => keep(frame(1)),
        (0, _) => keep(Reply::CloseNoReply),
        _ => keep(frame(g as u64 + 1)),
    });
    let mut client = fake.client();
    client.next_frame().await.unwrap();
    let start = Instant::now();
    client.next_frame().await.unwrap();
    assert!(start.elapsed() < ms(CAMERA_DAEMON_BACKOFF_MIN_MS));
    assert_eq!(fake.request_conns(), vec![0, 0, 1]);
    assert_ne!(
        fake.request(1).request_id,
        fake.request(2).request_id,
        "the retried request must carry a new nonce"
    );
    assert_eq!(fake.max_open(), 1);

    // (c) Exactly one retry: when the fresh connection also fails, the call is `Io`.
    let fake = FakeDaemon::scripted(|conn, on_conn, _| match (conn, on_conn) {
        (0, 0) => keep(frame(1)),
        _ => keep(Reply::CloseNoReply),
    });
    let mut client = fake.client();
    client.next_frame().await.unwrap();
    assert_eq!(expect_err(client.next_frame().await), PreviewError::Io);
    assert_eq!(fake.accepted(), 2, "exactly one retry");
    assert_eq!(fake.request_conns(), vec![0, 0, 1]);

    // (d) An `Io` on a fresh connection is never retried.
    let fake = FakeDaemon::always(Reply::CloseNoReply);
    let mut client = fake.client();
    assert_eq!(expect_err(client.next_frame().await), PreviewError::Io);
    assert_eq!(fake.accepted(), 1, "a fresh connection is not retried");
    assert_eq!(fake.request_total(), 1);
}
