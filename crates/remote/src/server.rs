//! Accept loop, request handling, SSE streams, the lock flow (spec §2.8, D4–D9) and the
//! opt-in unlock flow (ADR 2026-10-06).
//!
//! Every bound is a `tokio::time` primitive (constants in the crate root): the connection
//! count (`MAX_CONNECTIONS`, excess accepted streams are dropped without a byte), the head
//! read (`REQUEST_HEAD_TIMEOUT_MS`), every write (`RESPONSE_WRITE_TIMEOUT_MS`), every
//! logind snapshot (`SNAPSHOT_DEADLINE_MS`), the whole lock flow (`LOCK_FLOW_DEADLINE_MS`),
//! the number and lifetime of streams (`MAX_SSE_STREAMS`, `MAX_SSE_STREAM_MS`).
//!
//! Dispatch order on a parsed head: HTTP error → `Host` (`421`) → identity (`403`) → route
//! → CSRF for the lock and the unlock (`403`) → `allow_unlock` for the unlock (`403`) →
//! handler. No logind call, stream slot or subscriber
//! registration happens before every check has passed.
//!
//! Readings: one poller task reads logind every `poll_interval_ms` while at least one
//! stream is open and publishes every read on a `watch` channel with a strictly increasing
//! `seq` reserved when the read **starts** (R3-1). Each stream starts with its own fresh read
//! and then forwards readings whose `seq` is above the last one it sent when the view
//! changed, or as a keep-alive so that an event reaches the phone at least every
//! `SSE_KEEPALIVE_MS`; a reading is sent at most once and never re-stamped.

use std::future::Future;
use std::io::ErrorKind;
use std::pin::pin;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{watch, Mutex, Notify, OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinSet;
use tokio::time::{sleep, sleep_until, timeout, timeout_at, Instant};
use tracing::{debug, info};

use crate::assets::asset;
use crate::config::RemoteConfig;
use crate::http::{
    encode_response, encode_response_head, encode_sse_event, encode_sse_head, parse_request_head,
    write_bounded, HttpError, Method, RequestHead, Response, WriteError,
};
use crate::identity::{authorize, check_host};
use crate::logind::{SessionSource, SourceError};
use crate::routes::{allow_header, check_lock_csrf, check_unlock_csrf, route, Route};
use crate::session::{select_session, SessionProps};
use crate::status::{status_from, view_changed, Reading, StatusView};
use crate::{
    LOCK_FLOW_DEADLINE_MS, MAX_CONNECTIONS, MAX_REQUEST_HEAD_BYTES, MAX_SSE_STREAMS,
    MAX_SSE_STREAM_MS, MIN_LOCK_INTERVAL_MS, MIN_UNLOCK_INTERVAL_MS, REQUEST_HEAD_TIMEOUT_MS,
    RESPONSE_WRITE_TIMEOUT_MS, SNAPSHOT_DEADLINE_MS, SSE_KEEPALIVE_MS, UNLOCK_FLOW_DEADLINE_MS,
};

// Streams hold their connection permit for their whole lifetime; request permits must
// always remain.
const _: () = assert!(MAX_SSE_STREAMS < MAX_CONNECTIONS);

/// `from + after` as a `tokio::time::Instant`; an unrepresentable deadline (never in
/// practice) falls back to `from`, which fails closed (the bound fires at once).
fn deadline(from: Instant, after: Duration) -> Instant {
    from.checked_add(after).unwrap_or(from)
}

/// Bytes read at a time while assembling a request head.
const HEAD_CHUNK_BYTES: usize = 1024;
/// Fixed buffer of a stream's read half: bytes are discarded, EOF ends the stream.
const STREAM_SINK_BYTES: usize = 64;
/// Most bytes discarded by the lingering close before the connection is dropped.
const LINGER_MAX_BYTES: usize = 64 * 1024;

/// Source of `checked_unix_ms` (Unix time in ms); production uses `SystemTime::now`.
pub type UnixClock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// Shared server state: configuration, own uid, logind source and clocks.
pub struct ServerState<S: SessionSource> {
    config: RemoteConfig,
    uid: u32,
    source: S,
    unix_clock: UnixClock,
    seq_start: u64,
}

impl<S: SessionSource> ServerState<S> {
    /// Production state: `SystemTime` clock, reading counter starting at 0 (first seq 1).
    #[must_use]
    pub fn new(config: RemoteConfig, uid: u32, source: S) -> Self {
        Self {
            config,
            uid,
            source,
            unix_clock: Arc::new(|| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
            }),
            seq_start: 0,
        }
    }

    /// Test hook: injected `checked_unix_ms` source.
    #[must_use]
    pub fn with_unix_clock(mut self, clock: UnixClock) -> Self {
        self.unix_clock = clock;
        self
    }

    /// Test hook: value the shared reading counter holds before the first reservation
    /// (`u64::MAX` makes the first `checked_add` fail and ends the poller).
    #[must_use]
    pub fn with_seq_start(mut self, seq_start: u64) -> Self {
        self.seq_start = seq_start;
        self
    }

    /// The configuration.
    #[must_use]
    pub fn config(&self) -> &RemoteConfig {
        &self.config
    }

    /// The own uid.
    #[must_use]
    pub fn uid(&self) -> u32 {
        self.uid
    }

    /// The logind source.
    #[must_use]
    pub fn source(&self) -> &S {
        &self.source
    }

    /// The injected Unix clock.
    #[must_use]
    pub fn unix_clock(&self) -> &UnixClock {
        &self.unix_clock
    }

    /// The initial value of the reading counter.
    #[must_use]
    pub fn seq_start(&self) -> u64 {
        self.seq_start
    }
}

/// Why `serve` returned without a clean shutdown (`main` exits `EXIT_RUNTIME`).
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    /// The poller task ended.
    #[error("poller task ended")]
    PollerEnded,
    /// The accept loop ended with an I/O error.
    #[error("accept loop ended: {0:?}")]
    Accept(std::io::ErrorKind),
    /// A supervised task panicked.
    #[error("server task panicked")]
    TaskPanicked,
}

/// Runtime state shared by the accept loop, the poller and every connection task.
struct Shared<S: SessionSource> {
    state: Arc<ServerState<S>>,
    /// Reading counter (reserved when a read starts, R3-1).
    seq: AtomicU64,
    /// Set when the counter overflowed: the poller returns and the server shuts down.
    fatal: AtomicBool,
    /// Open streams registered as poller subscribers.
    subscribers: AtomicUsize,
    /// Wakes the poller when the first subscriber registers (permit-storing).
    poller_wake: Notify,
    /// Latest reading while subscribers exist; `None` when idle.
    readings: watch::Sender<Option<Reading>>,
    /// Open streams (bounded by `MAX_SSE_STREAMS`).
    sse_slots: AtomicUsize,
    /// Last accepted lock (the lock gate serializes the whole flow).
    lock_gate: Mutex<Option<Instant>>,
    /// Last accepted unlock (the unlock gate serializes the whole flow).
    unlock_gate: Mutex<Option<Instant>>,
    /// Connection permits.
    connections: Arc<Semaphore>,
    /// Set to `true` at shutdown so every stream ends.
    closing: watch::Sender<bool>,
}

impl<S: SessionSource> Shared<S> {
    fn new(state: Arc<ServerState<S>>) -> Self {
        Self {
            seq: AtomicU64::new(state.seq_start),
            state,
            fatal: AtomicBool::new(false),
            subscribers: AtomicUsize::new(0),
            poller_wake: Notify::new(),
            readings: watch::Sender::new(None),
            sse_slots: AtomicUsize::new(0),
            lock_gate: Mutex::new(None),
            unlock_gate: Mutex::new(None),
            connections: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
            closing: watch::Sender::new(false),
        }
    }

    fn uid(&self) -> u32 {
        self.state.uid
    }

    fn poll_interval(&self) -> Duration {
        Duration::from_millis(self.state.config.poll_interval_ms)
    }

    /// Reserves the next `seq` (`checked_add`); `None` on overflow.
    fn reserve_seq(&self) -> Option<u64> {
        self.seq
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_add(1))
            .ok()
            .and_then(|previous| previous.checked_add(1))
    }

    /// Records the counter overflow and wakes the poller so that it returns.
    fn set_fatal(&self) {
        self.fatal.store(true, Ordering::SeqCst);
        self.poller_wake.notify_one();
    }

    fn is_fatal(&self) -> bool {
        self.fatal.load(Ordering::SeqCst)
    }

    /// One fresh `own_sessions` read under `SNAPSHOT_DEADLINE_MS`; `checked_unix_ms` is
    /// taken from the injected clock when the read starts and never re-stamped.
    async fn fresh_read(&self) -> (u64, Result<Vec<SessionProps>, SourceError>) {
        let checked_unix_ms = (self.state.unix_clock)();
        let result = match timeout(
            Duration::from_millis(SNAPSHOT_DEADLINE_MS),
            self.state.source.own_sessions(self.uid()),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(SourceError::Timeout),
        };
        (checked_unix_ms, result)
    }

    /// One fresh view (D8).
    async fn fresh_view(&self) -> StatusView {
        let (checked_unix_ms, result) = self.fresh_read().await;
        status_from(&result, self.uid(), checked_unix_ms)
    }
}

/// Frees an SSE slot on every exit path.
struct SlotGuard<S: SessionSource>(Arc<Shared<S>>);

impl<S: SessionSource> Drop for SlotGuard<S> {
    fn drop(&mut self) {
        let _ = self
            .0
            .sse_slots
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                Some(n.saturating_sub(1))
            });
    }
}

/// Unregisters a poller subscriber on every exit path.
struct SubscriberGuard<S: SessionSource>(Arc<Shared<S>>);

impl<S: SessionSource> Drop for SubscriberGuard<S> {
    fn drop(&mut self) {
        let _ = self
            .0
            .subscribers
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                Some(n.saturating_sub(1))
            });
    }
}

/// Serves `listener` until `shutdown` resolves (clean: `Ok(())`, the socket file at
/// `config.socket_path` removed) or a supervised task (accept loop, poller) ends (`Err`).
///
/// # Errors
///
/// [`ServeError`].
pub async fn serve<S: SessionSource>(
    listener: UnixListener,
    state: Arc<ServerState<S>>,
    shutdown: impl Future<Output = ()> + Send,
) -> Result<(), ServeError> {
    let socket_path = state.config.socket_path.clone();
    let shared = Arc::new(Shared::new(state));
    let connections: Arc<Mutex<JoinSet<()>>> = Arc::new(Mutex::new(JoinSet::new()));
    let mut supervised: JoinSet<ServeError> = JoinSet::new();
    supervised.spawn(poller(Arc::clone(&shared)));
    supervised.spawn(accept_loop(
        listener,
        Arc::clone(&shared),
        Arc::clone(&connections),
    ));

    let mut shutdown = pin!(shutdown);
    let result = tokio::select! {
        () = &mut shutdown => Ok(()),
        finished = supervised.join_next() => match finished {
            Some(Ok(err)) => Err(err),
            Some(Err(_)) => Err(ServeError::TaskPanicked),
            None => Err(ServeError::PollerEnded),
        },
    };

    shared.closing.send_replace(true);
    supervised.abort_all();
    while supervised.join_next().await.is_some() {}
    {
        let mut set = connections.lock().await;
        set.abort_all();
        while set.join_next().await.is_some() {}
    }
    if std::fs::symlink_metadata(&socket_path)
        .is_ok_and(|m| std::os::unix::fs::FileTypeExt::is_socket(&m.file_type()))
    {
        let _ = std::fs::remove_file(&socket_path);
    }
    match &result {
        Ok(()) => debug!("server stopped on shutdown"),
        Err(err) => debug!(%err, "server stopped"),
    }
    result
}

/// Reads logind every `poll_interval_ms` while at least one stream is open and publishes
/// every read; idle otherwise (zero D-Bus traffic without a stream). Returns only when the
/// reading counter overflowed.
async fn poller<S: SessionSource>(shared: Arc<Shared<S>>) -> ServeError {
    let interval = shared.poll_interval();
    loop {
        if shared.is_fatal() {
            return ServeError::PollerEnded;
        }
        while shared.subscribers.load(Ordering::SeqCst) == 0 {
            shared.poller_wake.notified().await;
            if shared.is_fatal() {
                return ServeError::PollerEnded;
            }
        }
        let Some(seq) = shared.reserve_seq() else {
            shared.set_fatal();
            return ServeError::PollerEnded;
        };
        let view = shared.fresh_view().await;
        shared.readings.send_replace(Some(Reading { seq, view }));
        sleep(interval).await;
        if shared.subscribers.load(Ordering::SeqCst) == 0 {
            shared.readings.send_replace(None);
        }
    }
}

/// Accepts connections; a connection beyond `MAX_CONNECTIONS` is dropped at once.
async fn accept_loop<S: SessionSource>(
    listener: UnixListener,
    shared: Arc<Shared<S>>,
    connections: Arc<Mutex<JoinSet<()>>>,
) -> ServeError {
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::ConnectionAborted | ErrorKind::Interrupted | ErrorKind::WouldBlock
                ) =>
            {
                continue;
            }
            Err(e) => return ServeError::Accept(e.kind()),
        };
        let Ok(permit) = Arc::clone(&shared.connections).try_acquire_owned() else {
            debug!("connection limit reached, dropping an accepted connection");
            drop(stream);
            continue;
        };
        let mut set = connections.lock().await;
        while set.try_join_next().is_some() {}
        set.spawn(handle_connection(stream, permit, Arc::clone(&shared)));
    }
}

/// Why a request head could not be obtained.
enum HeadRead {
    /// A parse error with a status (or `Incomplete` at the deadline).
    Http(HttpError),
    /// EOF or a read error before a complete head.
    Closed,
}

/// Assembles the request head in a buffer capped at `MAX_REQUEST_HEAD_BYTES + 1` bytes,
/// re-parsing after every read; bytes after the head terminator are never read on purpose
/// and ignored when already received.
async fn read_head(stream: &mut UnixStream, buf: &mut Vec<u8>) -> Result<RequestHead, HeadRead> {
    let capacity = MAX_REQUEST_HEAD_BYTES.saturating_add(1);
    loop {
        match parse_request_head(buf) {
            Ok(head) => return Ok(head),
            Err(HttpError::Incomplete) => {}
            Err(err) => return Err(HeadRead::Http(err)),
        }
        let room = capacity.saturating_sub(buf.len());
        if room == 0 {
            return Err(HeadRead::Http(HttpError::HeadTooLarge));
        }
        let mut chunk = [0u8; HEAD_CHUNK_BYTES];
        let take = room.min(HEAD_CHUNK_BYTES);
        let Some(window) = chunk.get_mut(..take) else {
            return Err(HeadRead::Closed);
        };
        match stream.read(window).await {
            Ok(0) | Err(_) => return Err(HeadRead::Closed),
            Ok(n) => buf.extend_from_slice(window.get(..n).unwrap_or_default()),
        }
    }
}

/// Lingering close: shuts the write side down, then discards whatever the client still
/// sends (an ignored body, a pipelined request) until it closes, bounded by
/// `RESPONSE_WRITE_TIMEOUT_MS` and `LINGER_MAX_BYTES`, so that unread input never turns the
/// response into a connection reset. Nothing read here is parsed or kept.
async fn finish(stream: &mut UnixStream) {
    let _ = stream.shutdown().await;
    let mut sink = [0u8; HEAD_CHUNK_BYTES];
    let mut discarded = 0usize;
    let _ = timeout(Duration::from_millis(RESPONSE_WRITE_TIMEOUT_MS), async {
        loop {
            match stream.read(&mut sink).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    discarded = discarded.saturating_add(n);
                    if discarded > LINGER_MAX_BYTES {
                        break;
                    }
                }
            }
        }
    })
    .await;
}

/// Writes a fully computed response (head only for `HEAD`) under the write bound and
/// closes the connection.
async fn respond(stream: &mut UnixStream, method: Method, response: &Response) {
    let bytes = if method == Method::Head {
        encode_response_head(response)
    } else {
        encode_response(response)
    };
    debug!(status = response.status, "response");
    if write_bounded(stream, &bytes).await.is_ok() {
        finish(stream).await;
    }
}

/// One connection: bounded head read, then the §2.8 dispatch order.
async fn handle_connection<S: SessionSource>(
    mut stream: UnixStream,
    _permit: OwnedSemaphorePermit,
    shared: Arc<Shared<S>>,
) {
    let head_deadline = deadline(
        Instant::now(),
        Duration::from_millis(REQUEST_HEAD_TIMEOUT_MS),
    );
    let mut buf = Vec::with_capacity(MAX_REQUEST_HEAD_BYTES.saturating_add(1));
    let head = match timeout_at(head_deadline, read_head(&mut stream, &mut buf)).await {
        Ok(Ok(head)) => head,
        Ok(Err(HeadRead::Http(err))) => {
            if let Some(status) = err.status(&buf) {
                debug!(%err, "request head refused");
                let result = match status {
                    400 => "bad_request",
                    413 => "body_not_allowed",
                    414 => "path_too_long",
                    _ => "head_too_large",
                };
                respond(&mut stream, Method::Get, &Response::json(status, result)).await;
            }
            return;
        }
        Ok(Err(HeadRead::Closed)) => return,
        Err(_) => {
            debug!("request head deadline, connection closed");
            return;
        }
    };
    drop(buf);

    let config = &shared.state.config;
    let headers: Vec<(&str, &[u8])> = head
        .headers
        .iter()
        .map(|(n, v)| (n.as_str(), v.as_slice()))
        .collect();
    let normalized_host = match check_host(&headers, &config.allowed_hosts) {
        Ok(normalized) => normalized,
        Err(err) => {
            debug!(%err, "misdirected request");
            respond(
                &mut stream,
                head.method,
                &Response::json(421, "misdirected_request"),
            )
            .await;
            return;
        }
    };
    if let Err(err) = authorize(&headers, &config.allowed_logins) {
        debug!(%err, "identity refused");
        respond(&mut stream, head.method, &Response::json(403, "forbidden")).await;
        return;
    }
    drop(headers);

    let resolved = route(head.method, &head.path);
    debug!(?resolved, "request");
    match resolved {
        Route::Asset(id) => {
            let embedded = asset(id);
            let response = Response {
                status: 200,
                content_type: embedded.content_type,
                body: embedded.body.to_vec(),
                extra_headers: Vec::new(),
            };
            respond(&mut stream, head.method, &response).await;
        }
        Route::Status => {
            let view = shared.fresh_view().await;
            let response = match serde_json::to_vec(&view) {
                Ok(body) => Response {
                    status: 200,
                    content_type: "application/json",
                    body,
                    extra_headers: Vec::new(),
                },
                Err(_) => Response::json(503, "unavailable"),
            };
            respond(&mut stream, head.method, &response).await;
        }
        Route::Events => {
            if head.method == Method::Head {
                if write_bounded(&mut stream, &encode_sse_head()).await.is_ok() {
                    finish(&mut stream).await;
                }
                return;
            }
            serve_stream(stream, shared).await;
        }
        Route::Lock => {
            if let Err(err) = check_lock_csrf(&head, &normalized_host) {
                debug!(%err, "lock refused");
                respond(&mut stream, head.method, &Response::json(403, "forbidden")).await;
                return;
            }
            let response = lock_flow(&shared).await;
            respond(&mut stream, head.method, &response).await;
        }
        Route::Unlock => {
            if let Err(err) = check_unlock_csrf(&head, &normalized_host) {
                debug!(%err, "unlock refused");
                respond(&mut stream, head.method, &Response::json(403, "forbidden")).await;
                return;
            }
            if !config.allow_unlock {
                debug!("unlock refused: allow_unlock is false");
                respond(
                    &mut stream,
                    head.method,
                    &Response::json(403, "unlock_disabled"),
                )
                .await;
                return;
            }
            let response = unlock_flow(&shared).await;
            respond(&mut stream, head.method, &response).await;
        }
        Route::NotFound => {
            respond(&mut stream, head.method, &Response::json(404, "not_found")).await;
        }
        Route::MethodNotAllowed => {
            let mut response = Response::json(405, "method_not_allowed");
            response
                .extra_headers
                .push(("Allow", allow_header(&head.path).to_string()));
            respond(&mut stream, head.method, &response).await;
        }
    }
}

/// D9: rate limit → fresh snapshot → `select_session` → `LockSession`, serialized under the
/// lock gate and bounded by `LOCK_FLOW_DEADLINE_MS`. The interval is recorded only when
/// `lock_session` is called (a failed call still counts; `409`/`503` from the snapshot do
/// not).
async fn lock_flow<S: SessionSource>(shared: &Shared<S>) -> Response {
    let flow = async {
        let mut gate = shared.lock_gate.lock().await;
        if let Some(last) = *gate {
            if Instant::now().saturating_duration_since(last)
                < Duration::from_millis(MIN_LOCK_INTERVAL_MS)
            {
                return Response::json(429, "rate_limited");
            }
        }
        let (_, result) = shared.fresh_read().await;
        let sessions = match result {
            Ok(sessions) => sessions,
            Err(err) => {
                debug!(%err, "lock snapshot failed");
                return Response::json(503, "unavailable");
            }
        };
        let Some(session) = select_session(&sessions, shared.uid()) else {
            return Response::json(409, "no_session");
        };
        if session.locked {
            return Response::json(409, "already_locked");
        }
        *gate = Some(Instant::now());
        match shared.state.source.lock_session(&session.id).await {
            Ok(()) => Response::json(202, "lock_requested"),
            Err(err) => {
                debug!(%err, "lock call failed");
                Response::json(503, "unavailable")
            }
        }
    };
    match timeout(Duration::from_millis(LOCK_FLOW_DEADLINE_MS), flow).await {
        Ok(response) => response,
        Err(_) => {
            debug!("lock flow deadline");
            Response::json(503, "unavailable")
        }
    }
}

/// ADR 2026-10-06: rate limit → fresh snapshot → `select_session` → `UnlockSession`,
/// serialized under the unlock gate and bounded by `UNLOCK_FLOW_DEADLINE_MS`; the mirror of
/// [`lock_flow`]. The interval is recorded only when `unlock_session` is called. Every accepted
/// unlock (one that reaches `unlock_session`) leaves one `info` audit line without identity or
/// session id.
async fn unlock_flow<S: SessionSource>(shared: &Shared<S>) -> Response {
    let flow = async {
        let mut gate = shared.unlock_gate.lock().await;
        if let Some(last) = *gate {
            if Instant::now().saturating_duration_since(last)
                < Duration::from_millis(MIN_UNLOCK_INTERVAL_MS)
            {
                return Response::json(429, "rate_limited");
            }
        }
        let (_, result) = shared.fresh_read().await;
        let sessions = match result {
            Ok(sessions) => sessions,
            Err(err) => {
                debug!(%err, "unlock snapshot failed");
                return Response::json(503, "unavailable");
            }
        };
        let Some(session) = select_session(&sessions, shared.uid()) else {
            return Response::json(409, "no_session");
        };
        if !session.locked {
            return Response::json(409, "already_unlocked");
        }
        *gate = Some(Instant::now());
        info!("remote unlock requested");
        match shared.state.source.unlock_session(&session.id).await {
            Ok(()) => Response::json(202, "unlock_requested"),
            Err(err) => {
                debug!(%err, "unlock call failed");
                Response::json(503, "unavailable")
            }
        }
    };
    match timeout(Duration::from_millis(UNLOCK_FLOW_DEADLINE_MS), flow).await {
        Ok(response) => response,
        Err(_) => {
            debug!("unlock flow deadline");
            Response::json(503, "unavailable")
        }
    }
}

/// Serializes a view as one SSE event and writes it under the write bound.
async fn send_event<W: AsyncWrite + Unpin>(
    writer: &mut W,
    view: &StatusView,
) -> Result<(), WriteError> {
    let json = serde_json::to_string(view).map_err(|_| WriteError::Io)?;
    write_bounded(writer, &encode_sse_event(&json)).await
}

/// `GET /api/events` (D6): slot → subscriber → head → own fresh first read → forward
/// readings. A reading is sent when its view differs from the last one sent, or as a
/// keep-alive: a reading whose successor cannot be expected before the keep-alive deadline
/// (`last_sent_at + SSE_KEEPALIVE_MS`) is sent even if unchanged, so the phone receives an
/// event at least every `SSE_KEEPALIVE_MS` with an advancing `checked_unix_ms`. When the
/// deadline passes without a reading, the next reading is sent (`keepalive_pending`); a
/// hard fallback re-sends the newest unsent reading after one more poll interval plus the
/// snapshot deadline. The stream ends at EOF on its read half, on a write failure, after
/// `MAX_SSE_STREAM_MS`, at shutdown, or when the reading counter overflowed.
async fn serve_stream<S: SessionSource>(mut stream: UnixStream, shared: Arc<Shared<S>>) {
    let acquired = shared
        .sse_slots
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
            (n < MAX_SSE_STREAMS).then(|| n.saturating_add(1))
        });
    if acquired.is_err() {
        debug!("stream limit reached");
        respond(
            &mut stream,
            Method::Get,
            &Response::json(503, "too_many_streams"),
        )
        .await;
        return;
    }
    let _slot = SlotGuard(Arc::clone(&shared));
    let _ = shared
        .subscribers
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
            Some(n.saturating_add(1))
        });
    shared.poller_wake.notify_one();
    let _subscriber = SubscriberGuard(Arc::clone(&shared));
    let mut readings = shared.readings.subscribe();
    let mut closing = shared.closing.subscribe();

    if write_bounded(&mut stream, &encode_sse_head())
        .await
        .is_err()
    {
        return;
    }
    let Some(seq) = shared.reserve_seq() else {
        shared.set_fatal();
        return;
    };
    let view = shared.fresh_view().await;
    let first = Reading { seq, view };
    let (mut read_half, mut write_half) = stream.into_split();
    if send_event(&mut write_half, &first.view).await.is_err() {
        return;
    }
    debug!("stream opened");

    let opened_at = Instant::now();
    let end_at = deadline(opened_at, Duration::from_millis(MAX_SSE_STREAM_MS));
    let keepalive = Duration::from_millis(SSE_KEEPALIVE_MS);
    let poll_interval = shared.poll_interval();
    let pending_grace = poll_interval.saturating_add(Duration::from_millis(SNAPSHOT_DEADLINE_MS));
    let mut last_seq = first.seq;
    let mut newest = first.clone();
    let mut last_sent = first;
    let mut last_sent_at = opened_at;
    let mut keepalive_pending = false;
    let mut sink = [0u8; STREAM_SINK_BYTES];

    loop {
        let keepalive_at = deadline(last_sent_at, keepalive);
        let fallback_at = deadline(keepalive_at, pending_grace);
        tokio::select! {
            biased;
            closed = closing.changed() => {
                if closed.is_err() || *closing.borrow_and_update() {
                    break;
                }
            }
            changed = readings.changed() => {
                if changed.is_err() {
                    break;
                }
                let Some(reading) = readings.borrow_and_update().clone() else {
                    continue;
                };
                if reading.seq <= last_seq {
                    continue;
                }
                last_seq = reading.seq;
                let changed_view = view_changed(&last_sent.view, &reading.view);
                newest = reading;
                let next_read_too_late = Instant::now()
                    .checked_add(poll_interval)
                    .is_none_or(|next| next >= keepalive_at);
                if changed_view || keepalive_pending || next_read_too_late {
                    if send_event(&mut write_half, &newest.view).await.is_err() {
                        break;
                    }
                    last_sent = newest.clone();
                    last_sent_at = Instant::now();
                    keepalive_pending = false;
                }
            }
            () = sleep_until(keepalive_at), if !keepalive_pending => {
                // No reading landed in the last poll interval: the next one is sent.
                keepalive_pending = true;
            }
            () = sleep_until(fallback_at), if keepalive_pending => {
                if newest.seq > last_sent.seq {
                    if send_event(&mut write_half, &newest.view).await.is_err() {
                        break;
                    }
                    last_sent = newest.clone();
                }
                last_sent_at = Instant::now();
                keepalive_pending = false;
            }
            () = sleep_until(end_at) => {
                debug!("stream lifetime reached");
                break;
            }
            read = read_half.read(&mut sink) => {
                if matches!(read, Ok(0) | Err(_)) {
                    break;
                }
            }
        }
    }
    debug!("stream closed");
}
