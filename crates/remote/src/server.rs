//! Accept loop, request handling, SSE streams, the lock flow (spec §2.8, D4–D9) and the
//! opt-in unlock flow (ADR 2026-10-06).
//!
//! Every bound is a `tokio::time` primitive (constants in the crate root): the connection
//! count (`MAX_CONNECTIONS`, excess accepted streams are dropped without a byte), the head
//! read (`REQUEST_HEAD_TIMEOUT_MS`), every write (`RESPONSE_WRITE_TIMEOUT_MS`), every
//! logind snapshot (`SNAPSHOT_DEADLINE_MS`), the whole lock flow (`LOCK_FLOW_DEADLINE_MS`),
//! the number and lifetime of streams (`MAX_SSE_STREAMS`, `MAX_SSE_STREAM_MS`).
//!
//! Dispatch order on a parsed head (spec §6 of the Funnel/passkey ADR): HTTP error → `Host`
//! (`421`) → classification (tailnet identity or Funnel marker, `403`) → Funnel class permit
//! (`503 busy`) and Funnel host = `rp_id` (`421`) → route → Funnel session (revocation, then
//! validation; `403 login_required`, register routes `403 forbidden`) and anonymous permit →
//! route CSRF (`403`) → route gates (`allow_unlock`, `rp_id`, body presence, lockout) →
//! anonymous body-read reservation → bounded body read (no mutex held) → handler. No logind
//! call, stream slot, subscriber registration or body read happens before every check has
//! passed, and every unlock needs a fresh passkey assertion with user verification.
//!
//! Readings: one poller task reads logind every `poll_interval_ms` while at least one
//! stream is open and publishes every read on a `watch` channel with a strictly increasing
//! `seq` reserved when the read **starts** (R3-1). Each stream starts with its own fresh read
//! and then forwards readings whose `seq` is above the last one it sent when the view
//! changed, or as a keep-alive so that an event reaches the phone at least every
//! `SSE_KEEPALIVE_MS`; a reading is sent at most once and never re-stamped.

use std::future::Future;
use std::io::ErrorKind;
use std::path::PathBuf;
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

use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use crate::assets::asset;
use crate::audit;
use crate::auth::{
    decode_assertion, decode_register_options, decode_registration, system_random,
    AssertionFailure, AuthState, BodyReadGuard, Capacity, FunnelStreamGuard, LimitKey,
    RandomSource, Verified,
};
use crate::challenge::{
    ChallengeBinding, ChallengeError, ChallengePurpose, PendingRegistration, Taken,
};
use crate::config::RemoteConfig;
use crate::credentials::{
    CredentialStore, PasskeyFile, PasskeyRecord, StoreError, STORE_LOCK_RETRY_MS,
};
use crate::enroll::{normalized_code_hash, read_code_file, remove_code_file, EnrollError};
use crate::http::{
    encode_response, encode_response_head, encode_sse_event, encode_sse_head, parse_request,
    read_body, write_bounded, BodyError, HttpError, Method, ParsedRequest, RequestHead, Response,
    WriteError,
};
use crate::identity::{check_host, classify_request, client_hint, ClientHint, PathClass};
use crate::logind::{SessionSource, SourceError};
use crate::routes::{
    accepts_body, allow_header, check_auth_csrf, check_lock_csrf, check_unlock_csrf,
    is_funnel_public, route, Route,
};
use crate::session::{select_session, SessionProps};
use crate::status::{status_from, view_changed, Reading, StatusView};
use crate::webauthn::{
    b64url_encode, parse_client_data, verify_registration, CeremonyType, RelyingParty,
};
use crate::websession::{clear_cookie_header, session_token_hash, set_cookie_header, Touch};
use crate::{
    ACTION_LOGIN, ACTION_LOGIN_OPTIONS, ACTION_LOGOUT, ACTION_REGISTER, ACTION_REGISTER_OPTIONS,
    ACTION_UNLOCK_OPTIONS, CHALLENGE_BYTES, ENROLL_CODE_TTL_S, FUNNEL_REFUSAL_LINGER_MS,
    LOCK_FLOW_DEADLINE_MS, MAX_CONNECTIONS, MAX_PASSKEYS, MAX_REQUEST_HEAD_BYTES, MAX_SSE_STREAMS,
    MAX_SSE_STREAM_MS, MIN_LOCK_INTERVAL_MS, MIN_UNLOCK_INTERVAL_MS, REQUEST_HEAD_TIMEOUT_MS,
    RESPONSE_WRITE_TIMEOUT_MS, SESSION_TOKEN_BYTES, SNAPSHOT_DEADLINE_MS, SSE_KEEPALIVE_MS,
    STORE_LOCK_TIMEOUT_MS, UNLOCK_FLOW_DEADLINE_MS, USER_HANDLE_BYTES, WEBAUTHN_TIMEOUT_MS,
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
    random: RandomSource,
    credentials_path: Option<PathBuf>,
    file_owner_uid: u32,
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
            random: system_random(),
            credentials_path: None,
            file_owner_uid: uid,
        }
    }

    /// Test hook: injected CSPRNG (scripted or failing).
    #[must_use]
    pub fn with_random(mut self, random: RandomSource) -> Self {
        self.random = random;
        self
    }

    /// The resolved credential store path (`resolve_credentials_path`); `None` keeps every
    /// passkey route fail-closed (`503 store_unavailable`).
    #[must_use]
    pub fn with_credentials_path(mut self, path: PathBuf) -> Self {
        self.credentials_path = Some(path);
        self
    }

    /// Test hook: uid that must own the credential store and the enrollment code file
    /// (default: the state's uid, i.e. the service uid).
    #[must_use]
    pub fn with_file_owner_uid(mut self, uid: u32) -> Self {
        self.file_owner_uid = uid;
        self
    }

    /// The injected CSPRNG.
    #[must_use]
    pub fn random(&self) -> &RandomSource {
        &self.random
    }

    /// The credential store path.
    #[must_use]
    pub fn credentials_path(&self) -> Option<&std::path::Path> {
        self.credentials_path.as_deref()
    }

    /// The uid that must own the store and code files.
    #[must_use]
    pub fn file_owner_uid(&self) -> u32 {
        self.file_owner_uid
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
    /// Every mutable passkey structure; never held across a logind call nor a body read.
    auth: Mutex<AuthState>,
    /// Per-class connection capacity (outside the auth mutex).
    capacity: Capacity,
    /// The relying party (`None` when `rp_id` is not configured).
    rp: Option<RelyingParty>,
}

impl<S: SessionSource> Shared<S> {
    fn new(state: Arc<ServerState<S>>) -> Self {
        Self {
            seq: AtomicU64::new(state.seq_start),
            fatal: AtomicBool::new(false),
            subscribers: AtomicUsize::new(0),
            poller_wake: Notify::new(),
            readings: watch::Sender::new(None),
            sse_slots: AtomicUsize::new(0),
            lock_gate: Mutex::new(None),
            unlock_gate: Mutex::new(None),
            connections: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
            closing: watch::Sender::new(false),
            auth: Mutex::new(AuthState::new(
                state
                    .credentials_path
                    .clone()
                    .map(|path| CredentialStore::new(path, state.file_owner_uid)),
            )),
            capacity: Capacity::new(),
            rp: state.config.auth.rp_id.as_deref().map(RelyingParty::new),
            state,
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

/// Passkey and session helpers of the shared state (spec §4.6, §5, §6).
impl<S: SessionSource> Shared<S> {
    /// Unix time in seconds from the injected clock (enrollment-code expiry).
    fn now_unix_s(&self) -> u64 {
        (self.state.unix_clock)() / 1000
    }

    /// The `0700` socket directory (home of the enrollment code file).
    fn socket_dir(&self) -> Option<&std::path::Path> {
        self.state.config.socket_path.parent()
    }

    /// Revocation, then validation of a presented session (dispatch step 7, spec §4.3).
    async fn validate_session(
        &self,
        presented: Option<[u8; 32]>,
        touch: Touch,
    ) -> Option<[u8; 32]> {
        let mut auth = self.auth.lock().await;
        auth.revoke();
        let hash = presented?;
        auth.sessions.validate(Instant::now(), &hash, touch)
    }

    /// SSE keep-alive re-validation: revocation first, then [`Touch::Keep`].
    async fn session_still_valid(&self, hash: &[u8; 32]) -> bool {
        let mut auth = self.auth.lock().await;
        auth.revoke();
        auth.sessions
            .validate(Instant::now(), hash, Touch::Keep)
            .is_some()
    }

    /// Records one counted authentication failure of `key`.
    async fn fail(&self, key: LimitKey) {
        let in_flight = self.capacity.hints_in_flight();
        self.auth
            .lock()
            .await
            .record_failure(key, Instant::now(), &in_flight);
    }

    /// `GET /api/auth/state` (spec §4.6); never reads logind.
    async fn auth_state_view(&self, ctx: &RequestContext) -> Response {
        let config = &self.state.config;
        let authenticated = match ctx.class {
            PathClass::Tailnet => true,
            PathClass::Funnel => ctx.session.is_some(),
        };
        let mode = match ctx.class {
            PathClass::Tailnet => "tailnet",
            PathClass::Funnel => "funnel",
        };
        if !authenticated {
            return json_value(
                200,
                &serde_json::json!({"mode": mode, "authenticated": false}),
            );
        }
        let passkeys = matches!(
            self.auth.lock().await.store_snapshot(),
            Ok(Some(file)) if !file.passkeys.is_empty()
        );
        json_value(
            200,
            &serde_json::json!({
                "mode": mode,
                "authenticated": true,
                "passkeys": passkeys,
                "unlock_enabled": config.allow_unlock && config.auth.rp_id.is_some(),
                "enrollment": ctx.class == PathClass::Tailnet && config.auth.rp_id.is_some(),
            }),
        )
    }

    /// The common tail of the login and unlock options routes: lockout and options limiter
    /// of `key`, a non-empty store, a fresh challenge issued into its pool.
    async fn issue_options(
        &self,
        key: LimitKey,
        class: PathClass,
        purpose: ChallengePurpose,
        binding: ChallengeBinding,
    ) -> Response {
        let Some(rp) = self.rp.as_ref() else {
            return Response::json(403, "passkeys_not_configured");
        };
        let in_flight = self.capacity.hints_in_flight();
        let mut auth = self.auth.lock().await;
        let now = Instant::now();
        if auth.locked_out(key, now) || auth.options_exhausted(key, now) {
            return Response::json(429, "rate_limited");
        }
        match auth.store_snapshot() {
            Ok(Some(file)) if !file.passkeys.is_empty() => {}
            Ok(_) => return Response::json(409, "no_passkey"),
            Err(err) => return store_error_response(err),
        }
        let mut challenge = Zeroizing::new([0u8; CHALLENGE_BYTES]);
        if (self.state.random)(challenge.as_mut_slice()).is_err() {
            return Response::json(503, "unavailable");
        }
        match auth
            .challenges
            .issue(now, class, purpose, binding, None, *challenge)
        {
            Ok(_) => {}
            Err(ChallengeError::PoolFull) => return Response::json(429, "too_many_challenges"),
            Err(_) => return Response::json(503, "unavailable"),
        }
        auth.record_issuance(key, now, &in_flight);
        drop(auth);
        options_response(rp.rp_id(), &challenge)
    }

    /// Persists the counter and backup state of a verified assertion when they changed
    /// (synced `0/0` passkeys never write).
    async fn persist_counter(&self, verified: &Verified) -> Result<(), StoreError> {
        if !verified.changed {
            return Ok(());
        }
        let id = verified.credential_id.clone();
        let outcome = verified.outcome;
        self.store_update(move |current| {
            let mut file = current.ok_or(StoreError::NoSuchPasskey)?;
            let record = file
                .passkeys
                .iter_mut()
                .find(|r| r.credential_id == id)
                .ok_or(StoreError::NoSuchPasskey)?;
            // Re-checked under the store lock: a concurrent assertion of the same
            // credential may already have stored a higher counter, which must never be
            // overwritten by a lower one (clone detection, last writer would otherwise win).
            if outcome.sign_count < record.sign_count {
                return Ok((file, ()));
            }
            record.sign_count = outcome.sign_count;
            record.backup_state = outcome.backup_state;
            Ok((file, ()))
        })
        .await
    }

    /// Read-modify-write of the credential store: non-blocking lock attempts every
    /// `STORE_LOCK_RETRY_MS` with `tokio::time::sleep` in between (never a blocking wait on
    /// the runtime, the auth mutex released while waiting), at most `STORE_LOCK_TIMEOUT_MS`
    /// (then `Busy`). A store failure other than `Busy` drops every web session.
    ///
    /// Accepted bound (candid review 2026-10-06): the read, `fsync` and `rename` of
    /// [`crate::credentials::CredentialStore::update_locked`] run synchronously on the
    /// current-thread runtime. They touch one local `0600` file of at most
    /// `MAX_CREDENTIAL_STORE_BYTES` (16 KiB) and happen only on a passkey registration,
    /// a removal or a counter change of a device-bound credential, so the stall is a
    /// single small local write; `spawn_blocking` is not used because the store lives
    /// behind the auth mutex and every caller already waits for its result.
    async fn store_update<T>(
        &self,
        f: impl FnOnce(Option<PasskeyFile>) -> Result<(PasskeyFile, T), StoreError>,
    ) -> Result<T, StoreError> {
        let started = Instant::now();
        let budget = Duration::from_millis(STORE_LOCK_TIMEOUT_MS);
        let lock = loop {
            let attempt = {
                let auth = self.auth.lock().await;
                match auth.store.as_ref() {
                    Some(store) => store.try_lock(),
                    None => Err(StoreError::Io),
                }
            };
            match attempt {
                Ok(Some(lock)) => break lock,
                Ok(None) => {}
                Err(err) => {
                    self.auth.lock().await.store_failed(err);
                    return Err(err);
                }
            }
            if started.elapsed() >= budget {
                return Err(StoreError::Busy);
            }
            sleep(Duration::from_millis(STORE_LOCK_RETRY_MS)).await;
        };
        let mut auth = self.auth.lock().await;
        let result = match auth.store.as_mut() {
            Some(store) => store.update_locked(&lock, &self.state.random, f),
            None => Err(StoreError::Io),
        };
        if let Err(err) = result {
            auth.store_failed(err);
        }
        drop(auth);
        drop(lock);
        result
    }

    /// Checks the enrollment code file (and, for register options, the typed `code`):
    /// absent, insecure, malformed, expired or future-dated files are refused (the last
    /// three removed); a wrong code counts one attempt and the `MAX_ENROLL_CODE_ATTEMPTS`th
    /// removes the file.
    async fn check_code(&self, typed: Option<&str>) -> CodeCheck {
        let Some(dir) = self.socket_dir() else {
            return CodeCheck::Rejected;
        };
        let file = match read_code_file(dir, self.state.file_owner_uid) {
            Ok(Some(file)) => file,
            Ok(None) => return CodeCheck::Rejected,
            Err(EnrollError::Malformed) => {
                remove_code_file(dir);
                return CodeCheck::Rejected;
            }
            Err(err) => {
                debug!(%err, "enrollment code file refused");
                return CodeCheck::Rejected;
            }
        };
        let now = self.now_unix_s();
        if file.expires_unix_s <= now || file.expires_unix_s > now.saturating_add(ENROLL_CODE_TTL_S)
        {
            remove_code_file(dir);
            self.auth.lock().await.reset_code_attempts();
            return CodeCheck::Rejected;
        }
        let Some(typed) = typed else {
            return CodeCheck::Valid(file.code_hash);
        };
        let matches =
            normalized_code_hash(typed).is_some_and(|hash| bool::from(hash.ct_eq(&file.code_hash)));
        if matches {
            return CodeCheck::Valid(file.code_hash);
        }
        if self.auth.lock().await.wrong_code(&file.code_hash) {
            remove_code_file(dir);
        }
        CodeCheck::Rejected
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
/// re-parsing after every read with [`parse_request`]; bytes after the head terminator are
/// never read on purpose, and those already received are the body prefix of a body route
/// (ignored on every other route).
async fn read_head(stream: &mut UnixStream, buf: &mut Vec<u8>) -> Result<ParsedRequest, HeadRead> {
    let capacity = MAX_REQUEST_HEAD_BYTES.saturating_add(1);
    loop {
        match parse_request(buf, accepts_body) {
            Ok(parsed) => return Ok(parsed),
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
/// sends (an ignored body, a pipelined request) until it closes, bounded by `linger` and
/// `LINGER_MAX_BYTES`, so that unread input never turns the response into a connection
/// reset. Nothing read here is parsed or kept.
async fn finish(stream: &mut UnixStream, linger: Duration) {
    let _ = stream.shutdown().await;
    let mut sink = [0u8; HEAD_CHUNK_BYTES];
    let mut discarded = 0usize;
    let _ = timeout(linger, async {
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

/// The lingering bound of a served response (and of every classified tailnet response).
fn served_linger() -> Duration {
    Duration::from_millis(RESPONSE_WRITE_TIMEOUT_MS)
}

/// The lingering bound of a refusal before classification or of a refused Funnel request.
fn refusal_linger() -> Duration {
    Duration::from_millis(FUNNEL_REFUSAL_LINGER_MS)
}

/// Writes a fully computed response (head only for `HEAD`) under the write bound and
/// closes the connection after a lingering close of at most `linger`.
async fn respond_with(
    stream: &mut UnixStream,
    method: Method,
    response: &Response,
    linger: Duration,
) {
    let bytes = if method == Method::Head {
        encode_response_head(response)
    } else {
        encode_response(response)
    };
    debug!(status = response.status, "response");
    if write_bounded(stream, &bytes).await.is_ok() {
        finish(stream, linger).await;
    }
}

/// [`respond_with`] with the served-response linger.
async fn respond(stream: &mut UnixStream, method: Method, response: &Response) {
    respond_with(stream, method, response, served_linger()).await;
}

/// Class permits of one connection; every refusal releases them before lingering.
#[derive(Default)]
struct ClassPermits {
    funnel: Option<OwnedSemaphorePermit>,
    anonymous: Option<OwnedSemaphorePermit>,
    body_read: Option<BodyReadGuard>,
    stream: Option<FunnelStreamGuard>,
}

impl ClassPermits {
    fn release(&mut self) {
        *self = Self::default();
    }
}

/// What the dispatcher knows about a classified request.
struct RequestContext {
    parsed: ParsedRequest,
    /// Body bytes received together with the head (zeroized on drop).
    prefix: Zeroizing<Vec<u8>>,
    normalized_host: String,
    class: PathClass,
    /// Anonymous rate-limit bucket (Funnel only; `UNKNOWN` on the tailnet, never used there).
    hint: ClientHint,
    /// SHA-256 of a valid web-session token (Funnel only).
    session: Option<[u8; 32]>,
}

impl RequestContext {
    fn head(&self) -> &RequestHead {
        &self.parsed.head
    }

    /// The limiter key of the unlock routes.
    fn unlock_key(&self) -> LimitKey {
        match self.class {
            PathClass::Tailnet => LimitKey::Tailnet,
            PathClass::Funnel => LimitKey::FunnelSession,
        }
    }

    /// The binding of an unlock challenge of this caller.
    fn unlock_binding(&self) -> ChallengeBinding {
        match (self.class, self.session) {
            (PathClass::Funnel, Some(hash)) => ChallengeBinding::WebSession(hash),
            _ => ChallengeBinding::None,
        }
    }
}

/// Sends `response` for a request of `class`: a Funnel refusal (status ≥ 400) releases every
/// class permit first and lingers at most `FUNNEL_REFUSAL_LINGER_MS`; anything else keeps the
/// served-response linger.
async fn answer(
    stream: &mut UnixStream,
    method: Method,
    response: &Response,
    class: PathClass,
    permits: &mut ClassPermits,
) {
    if class == PathClass::Funnel && response.status >= 400 {
        permits.release();
        respond_with(stream, method, response, refusal_linger()).await;
    } else {
        respond(stream, method, response).await;
    }
}

/// A JSON response with an arbitrary object body.
fn json_value(status: u16, value: &serde_json::Value) -> Response {
    match serde_json::to_vec(value) {
        Ok(body) => Response {
            status,
            content_type: "application/json",
            body,
            extra_headers: Vec::new(),
        },
        Err(_) => Response::json(503, "unavailable"),
    }
}

/// The result string of a head refusal.
fn head_refusal_result(err: HttpError, status: u16) -> &'static str {
    match (err, status) {
        (HttpError::BodyTooLarge, _) => "body_too_large",
        (_, 400) => "bad_request",
        (_, 413) => "body_not_allowed",
        (_, 414) => "path_too_long",
        _ => "head_too_large",
    }
}

/// One connection: bounded head read, then the dispatch order of spec §6.
async fn handle_connection<S: SessionSource>(
    mut stream: UnixStream,
    _permit: OwnedSemaphorePermit,
    shared: Arc<Shared<S>>,
) {
    let head_deadline = deadline(
        Instant::now(),
        Duration::from_millis(REQUEST_HEAD_TIMEOUT_MS),
    );
    let mut buf = Zeroizing::new(Vec::with_capacity(MAX_REQUEST_HEAD_BYTES.saturating_add(1)));
    let parsed = match timeout_at(head_deadline, read_head(&mut stream, &mut buf)).await {
        Ok(Ok(parsed)) => parsed,
        Ok(Err(HeadRead::Http(err))) => {
            if let Some(status) = err.status(&buf) {
                debug!(%err, "request head refused");
                let result = head_refusal_result(err, status);
                respond_with(
                    &mut stream,
                    Method::Get,
                    &Response::json(status, result),
                    refusal_linger(),
                )
                .await;
            }
            return;
        }
        Ok(Err(HeadRead::Closed)) => return,
        Err(_) => {
            debug!("request head deadline, connection closed");
            return;
        }
    };
    let prefix = Zeroizing::new(buf.get(parsed.head_len..).unwrap_or_default().to_vec());
    drop(buf);

    let config = &shared.state.config;
    let method = parsed.head.method;
    let (normalized_host, caller, hint, presented) = {
        let headers: Vec<(&str, &[u8])> = parsed
            .head
            .headers
            .iter()
            .map(|(n, v)| (n.as_str(), v.as_slice()))
            .collect();
        let normalized_host = match check_host(&headers, &config.allowed_hosts) {
            Ok(normalized) => normalized,
            Err(err) => {
                debug!(%err, "misdirected request");
                let response = Response::json(421, "misdirected_request");
                respond_with(&mut stream, method, &response, refusal_linger()).await;
                return;
            }
        };
        let caller =
            match classify_request(&headers, &config.allowed_logins, config.auth.allow_funnel) {
                Ok(caller) => caller,
                Err(err) => {
                    debug!(%err, "identity refused");
                    let response = Response::json(403, "forbidden");
                    respond_with(&mut stream, method, &response, refusal_linger()).await;
                    return;
                }
            };
        let (hint, presented) = if caller.class() == PathClass::Funnel {
            (client_hint(&headers), session_token_hash(&headers))
        } else {
            (ClientHint::UNKNOWN, None)
        };
        (normalized_host, caller, hint, presented)
    };
    let class = caller.class();
    let mut permits = ClassPermits::default();

    if class == PathClass::Funnel {
        match shared.capacity.enter_funnel() {
            Ok(permit) => permits.funnel = Some(permit),
            Err(_) => {
                debug!("funnel capacity reached");
                answer(
                    &mut stream,
                    method,
                    &Response::json(503, "busy"),
                    class,
                    &mut permits,
                )
                .await;
                return;
            }
        }
        if config.auth.rp_id.as_deref() != Some(normalized_host.as_str()) {
            debug!("funnel host is not the relying party");
            let response = Response::json(421, "misdirected_request");
            answer(&mut stream, method, &response, class, &mut permits).await;
            return;
        }
    }

    let resolved = route(method, &parsed.head.path);
    debug!(?resolved, "request");
    let mut session = None;
    if class == PathClass::Funnel {
        let touch = match resolved {
            Route::Asset(_) | Route::NotFound | Route::MethodNotAllowed => Touch::Keep,
            _ => Touch::Refresh,
        };
        session = shared.validate_session(presented, touch).await;
        let refusal = if matches!(resolved, Route::RegisterOptions | Route::RegisterVerify) {
            Some(Response::json(403, "forbidden"))
        } else if !is_funnel_public(resolved) && session.is_none() {
            Some(Response::json(403, "login_required"))
        } else {
            None
        };
        if let Some(response) = refusal {
            debug!("funnel request refused before routing");
            answer(&mut stream, method, &response, class, &mut permits).await;
            return;
        }
        if session.is_none() {
            match shared.capacity.enter_anonymous() {
                Ok(permit) => permits.anonymous = Some(permit),
                Err(_) => {
                    debug!("anonymous funnel capacity reached");
                    let response = Response::json(503, "busy");
                    answer(&mut stream, method, &response, class, &mut permits).await;
                    return;
                }
            }
        }
    }

    let ctx = RequestContext {
        parsed,
        prefix,
        normalized_host,
        class,
        hint,
        session,
    };
    let response = match resolved {
        Route::Asset(id) => {
            let embedded = asset(id);
            Some(Response {
                status: 200,
                content_type: embedded.content_type,
                body: embedded.body.to_vec(),
                extra_headers: Vec::new(),
            })
        }
        Route::Status => {
            let view = shared.fresh_view().await;
            Some(match serde_json::to_vec(&view) {
                Ok(body) => Response {
                    status: 200,
                    content_type: "application/json",
                    body,
                    extra_headers: Vec::new(),
                },
                Err(_) => Response::json(503, "unavailable"),
            })
        }
        Route::Events => {
            if method == Method::Head {
                if write_bounded(&mut stream, &encode_sse_head()).await.is_ok() {
                    finish(&mut stream, served_linger()).await;
                }
                return;
            }
            if class == PathClass::Funnel {
                match shared.capacity.enter_funnel_stream() {
                    Ok(slot) => permits.stream = Some(slot),
                    Err(_) => {
                        debug!("funnel stream limit reached");
                        let response = Response::json(503, "too_many_streams");
                        answer(&mut stream, method, &response, class, &mut permits).await;
                        return;
                    }
                }
            }
            serve_stream(stream, Arc::clone(&shared), ctx.session).await;
            drop(permits);
            return;
        }
        Route::Lock => Some(match check_lock_csrf(ctx.head(), &ctx.normalized_host) {
            Ok(()) => lock_flow(&shared).await,
            Err(err) => {
                debug!(%err, "lock refused");
                Response::json(403, "forbidden")
            }
        }),
        Route::Unlock => unlock_route(&shared, &ctx, &mut stream, &mut permits).await,
        Route::AuthState => Some(shared.auth_state_view(&ctx).await),
        Route::LoginOptions => Some(login_options(&shared, &ctx).await),
        Route::LoginVerify => login_verify(&shared, &ctx, &mut stream, &mut permits).await,
        Route::Logout => Some(logout(&shared, &ctx).await),
        Route::UnlockOptions => Some(unlock_options(&shared, &ctx).await),
        Route::RegisterOptions => register_options(&shared, &ctx, &mut stream).await,
        Route::RegisterVerify => register_verify(&shared, &ctx, &mut stream).await,
        Route::NotFound => Some(Response::json(404, "not_found")),
        Route::MethodNotAllowed => {
            let mut response = Response::json(405, "method_not_allowed");
            response
                .extra_headers
                .push(("Allow", allow_header(&ctx.head().path).to_string()));
            Some(response)
        }
    };
    match response {
        Some(response) => answer(&mut stream, method, &response, class, &mut permits).await,
        None => debug!("connection closed without a response"),
    }
}

// ---------------------------------------------------------------------------------------
// Passkey routes (spec §5, §5.1)
// ---------------------------------------------------------------------------------------

/// How a body read ended.
enum BodyOutcome {
    /// The bounded body.
    Body(Zeroizing<Vec<u8>>),
    /// A refusal; `counted` when it is an authentication failure of the caller's key.
    Refuse(Response, bool),
    /// Deadline or peer closed: the connection is closed without a response.
    Close,
}

/// Reads the body of a body route under `BODY_READ_TIMEOUT_MS`, with no mutex held.
async fn read_request_body(stream: &mut UnixStream, ctx: &RequestContext) -> BodyOutcome {
    match read_body(stream, &ctx.prefix, ctx.parsed.framing).await {
        Ok(body) => BodyOutcome::Body(body),
        Err(BodyError::Malformed) => BodyOutcome::Refuse(Response::json(400, "bad_request"), true),
        Err(BodyError::TooLarge) => {
            BodyOutcome::Refuse(Response::json(413, "body_too_large"), false)
        }
        Err(BodyError::Timeout | BodyError::Closed) => BodyOutcome::Close,
    }
}

/// The CSRF, `rp_id` and host gates of every `/api/auth/*` POST route (spec §5): `X-Soos-Action`
/// equal to `action` and the required `Origin` (`403 forbidden`), `rp_id` configured (`403
/// passkeys_not_configured`), effective host equal to `rp_id` (`421`).
fn auth_route_gate<S: SessionSource>(
    shared: &Shared<S>,
    ctx: &RequestContext,
    action: &str,
) -> Result<(), Response> {
    let rp_id = shared.state.config.auth.rp_id.as_deref();
    let csrf_host = rp_id.unwrap_or(ctx.normalized_host.as_str());
    if let Err(err) = check_auth_csrf(ctx.head(), csrf_host, action) {
        debug!(%err, "passkey route refused");
        return Err(Response::json(403, "forbidden"));
    }
    rp_host_gate(shared, ctx)
}

/// `rp_id` configured (`403 passkeys_not_configured`) and equal to the effective host (`421`).
fn rp_host_gate<S: SessionSource>(
    shared: &Shared<S>,
    ctx: &RequestContext,
) -> Result<(), Response> {
    match shared.state.config.auth.rp_id.as_deref() {
        None => Err(Response::json(403, "passkeys_not_configured")),
        Some(rp_id) if rp_id != ctx.normalized_host => {
            Err(Response::json(421, "misdirected_request"))
        }
        Some(_) => Ok(()),
    }
}

/// The options response of a login or unlock ceremony.
fn options_response(rp_id: &str, challenge: &[u8; CHALLENGE_BYTES]) -> Response {
    json_value(
        200,
        &serde_json::json!({
            "challenge": b64url_encode(challenge),
            "rp_id": rp_id,
            "timeout_ms": WEBAUTHN_TIMEOUT_MS,
        }),
    )
}

/// `503 store_unavailable` (sessions already handled by the caller).
fn store_unavailable() -> Response {
    Response::json(503, "store_unavailable")
}

/// The outcome of a store failure on the auth routes.
fn store_error_response(err: StoreError) -> Response {
    debug!(%err, "credential store refused");
    store_unavailable()
}

/// `POST /api/auth/login/options` (Funnel only).
async fn login_options<S: SessionSource>(shared: &Shared<S>, ctx: &RequestContext) -> Response {
    if ctx.class != PathClass::Funnel {
        return Response::json(403, "forbidden");
    }
    if let Err(response) = auth_route_gate(shared, ctx, ACTION_LOGIN_OPTIONS) {
        return response;
    }
    let key = LimitKey::FunnelAnonymous(ctx.hint);
    let binding = ChallengeBinding::Login(ctx.hint);
    shared
        .issue_options(key, PathClass::Funnel, ChallengePurpose::Login, binding)
        .await
}

/// `POST /api/auth/unlock/options` (tailnet, or Funnel with a session).
async fn unlock_options<S: SessionSource>(shared: &Shared<S>, ctx: &RequestContext) -> Response {
    let rp_id = shared.state.config.auth.rp_id.as_deref();
    let csrf_host = rp_id.unwrap_or(ctx.normalized_host.as_str());
    if let Err(err) = check_auth_csrf(ctx.head(), csrf_host, ACTION_UNLOCK_OPTIONS) {
        debug!(%err, "unlock options refused");
        return Response::json(403, "forbidden");
    }
    if !shared.state.config.allow_unlock {
        return Response::json(403, "unlock_disabled");
    }
    if let Err(response) = rp_host_gate(shared, ctx) {
        return response;
    }
    shared
        .issue_options(
            ctx.unlock_key(),
            ctx.class,
            ChallengePurpose::Unlock,
            ctx.unlock_binding(),
        )
        .await
}

/// `POST /api/auth/logout` (Funnel; with or without a session).
async fn logout<S: SessionSource>(shared: &Shared<S>, ctx: &RequestContext) -> Response {
    if ctx.class != PathClass::Funnel {
        return Response::json(403, "forbidden");
    }
    if let Err(response) = auth_route_gate(shared, ctx, ACTION_LOGOUT) {
        return response;
    }
    if let Some(hash) = ctx.session {
        shared.auth.lock().await.sessions.remove(&hash);
    }
    let mut response = Response::json(200, "logged_out");
    response
        .extra_headers
        .push(("Set-Cookie", clear_cookie_header()));
    response
}

/// `POST /api/auth/login/verify` (Funnel only).
async fn login_verify<S: SessionSource>(
    shared: &Shared<S>,
    ctx: &RequestContext,
    stream: &mut UnixStream,
    permits: &mut ClassPermits,
) -> Option<Response> {
    if ctx.class != PathClass::Funnel {
        return Some(Response::json(403, "forbidden"));
    }
    if let Err(response) = auth_route_gate(shared, ctx, ACTION_LOGIN) {
        return Some(response);
    }
    let key = LimitKey::FunnelAnonymous(ctx.hint);
    if shared.auth.lock().await.locked_out(key, Instant::now()) {
        return Some(Response::json(429, "rate_limited"));
    }
    if ctx.session.is_none() {
        match shared.capacity.reserve_anonymous_body_read(ctx.hint) {
            Ok(guard) => permits.body_read = Some(guard),
            Err(_) => return Some(Response::json(503, "busy")),
        }
    }
    let body = match read_request_body(stream, ctx).await {
        BodyOutcome::Body(body) => body,
        BodyOutcome::Refuse(response, counted) => {
            permits.body_read = None;
            if counted {
                shared.fail(key).await;
            }
            return Some(response);
        }
        BodyOutcome::Close => return None,
    };
    let Ok(assertion) = decode_assertion(&body) else {
        drop(body);
        shared.fail(key).await;
        permits.body_read = None;
        return Some(Response::json(400, "bad_request"));
    };
    drop(body);
    let Some(rp) = shared.rp.as_ref() else {
        return Some(Response::json(403, "passkeys_not_configured"));
    };
    let checked = {
        let mut auth = shared.auth.lock().await;
        let checked = auth.check_assertion(
            rp,
            PathClass::Funnel,
            ChallengePurpose::Login,
            &ChallengeBinding::Login(ctx.hint),
            &assertion,
            Instant::now(),
        );
        if matches!(checked, Err(AssertionFailure::Rejected)) {
            auth.record_failure(key, Instant::now(), &shared.capacity.hints_in_flight());
        }
        checked
    };
    // The verified body is done with; the anonymous body-read slot is released before the
    // store and session work.
    drop(assertion);
    permits.body_read = None;
    let verified = match checked {
        Ok(verified) => verified,
        Err(AssertionFailure::Rejected) => return Some(Response::json(403, "passkey_rejected")),
        Err(AssertionFailure::Store(err)) => return Some(store_error_response(err)),
    };
    if let Err(err) = shared.persist_counter(&verified).await {
        return Some(store_error_response(err));
    }
    let mut token = Zeroizing::new([0u8; SESSION_TOKEN_BYTES]);
    if (shared.state.random)(token.as_mut_slice()).is_err() {
        return Some(Response::json(503, "unavailable"));
    }
    let created = {
        let mut auth = shared.auth.lock().await;
        if let Some(previous) = ctx.session {
            auth.sessions.remove(&previous);
        }
        auth.sessions
            .create(Instant::now(), *token, verified.credential_hash)
    };
    let Ok(issued) = created else {
        return Some(Response::json(429, "too_many_sessions"));
    };
    audit::login_accepted();
    let mut response = Response::json(200, "logged_in");
    response
        .extra_headers
        .push(("Set-Cookie", set_cookie_header(&issued)));
    Some(response)
}

/// `POST /api/unlock` (spec §5.1): CSRF → `allow_unlock` → `rp_id` → body present → host →
/// lockout → body → assertion (`unlock` purpose, caller class and binding) → counter
/// persistence → the unchanged unlock flow.
async fn unlock_route<S: SessionSource>(
    shared: &Shared<S>,
    ctx: &RequestContext,
    stream: &mut UnixStream,
    permits: &mut ClassPermits,
) -> Option<Response> {
    if let Err(err) = check_unlock_csrf(ctx.head(), &ctx.normalized_host) {
        debug!(%err, "unlock refused");
        return Some(Response::json(403, "forbidden"));
    }
    if !shared.state.config.allow_unlock {
        debug!("unlock refused: allow_unlock is false");
        return Some(Response::json(403, "unlock_disabled"));
    }
    if shared.state.config.auth.rp_id.is_none() {
        return Some(Response::json(403, "passkeys_not_configured"));
    }
    if !ctx.parsed.has_body() {
        debug!("unlock refused: no passkey assertion");
        return Some(Response::json(403, "passkey_required"));
    }
    if let Err(response) = rp_host_gate(shared, ctx) {
        return Some(response);
    }
    let key = ctx.unlock_key();
    if shared.auth.lock().await.locked_out(key, Instant::now()) {
        return Some(Response::json(429, "rate_limited"));
    }
    let body = match read_request_body(stream, ctx).await {
        BodyOutcome::Body(body) => body,
        BodyOutcome::Refuse(response, counted) => {
            if counted {
                shared.fail(key).await;
            }
            return Some(response);
        }
        BodyOutcome::Close => return None,
    };
    let Ok(assertion) = decode_assertion(&body) else {
        drop(body);
        shared.fail(key).await;
        return Some(Response::json(400, "bad_request"));
    };
    drop(body);
    let Some(rp) = shared.rp.as_ref() else {
        return Some(Response::json(403, "passkeys_not_configured"));
    };
    let checked = {
        let mut auth = shared.auth.lock().await;
        let checked = auth.check_assertion(
            rp,
            ctx.class,
            ChallengePurpose::Unlock,
            &ctx.unlock_binding(),
            &assertion,
            Instant::now(),
        );
        if matches!(checked, Err(AssertionFailure::Rejected)) {
            auth.record_failure(key, Instant::now(), &shared.capacity.hints_in_flight());
        }
        checked
    };
    drop(assertion);
    let verified = match checked {
        Ok(verified) => verified,
        Err(AssertionFailure::Rejected) => {
            debug!("unlock refused: passkey assertion rejected");
            return Some(Response::json(403, "passkey_rejected"));
        }
        Err(AssertionFailure::Store(err)) => return Some(store_error_response(err)),
    };
    if let Err(err) = shared.persist_counter(&verified).await {
        return Some(store_error_response(err));
    }
    permits.body_read = None;
    Some(unlock_flow(shared).await)
}

/// The verdict on a typed enrollment code against the code file.
enum CodeCheck {
    /// The code matches an unexpired code file with this hash.
    Valid([u8; 32]),
    /// No valid code (a counted failure).
    Rejected,
}

/// `POST /api/auth/register/options` (tailnet only).
async fn register_options<S: SessionSource>(
    shared: &Shared<S>,
    ctx: &RequestContext,
    stream: &mut UnixStream,
) -> Option<Response> {
    if let Err(response) = auth_route_gate(shared, ctx, ACTION_REGISTER_OPTIONS) {
        return Some(response);
    }
    let key = LimitKey::Tailnet;
    {
        let mut auth = shared.auth.lock().await;
        let now = Instant::now();
        if auth.locked_out(key, now) || auth.options_exhausted(key, now) {
            return Some(Response::json(429, "rate_limited"));
        }
    }
    let body = match read_request_body(stream, ctx).await {
        BodyOutcome::Body(body) => body,
        BodyOutcome::Refuse(response, counted) => {
            if counted {
                shared.fail(key).await;
            }
            return Some(response);
        }
        BodyOutcome::Close => return None,
    };
    let Ok(code) = decode_register_options(&body) else {
        drop(body);
        shared.fail(key).await;
        return Some(Response::json(400, "bad_request"));
    };
    drop(body);
    let code_hash = match shared.check_code(Some(&code)).await {
        CodeCheck::Valid(hash) => hash,
        CodeCheck::Rejected => {
            shared.fail(key).await;
            return Some(Response::json(403, "enroll_code_rejected"));
        }
    };
    drop(code);
    let Some(rp) = shared.rp.as_ref() else {
        return Some(Response::json(403, "passkeys_not_configured"));
    };
    let rp_id = rp.rp_id().to_string();
    let mut auth = shared.auth.lock().await;
    let (stored_handle, exclude) = match auth.store_snapshot() {
        Ok(Some(file)) => {
            if file.passkeys.len() >= MAX_PASSKEYS {
                return Some(Response::json(409, "passkey_limit"));
            }
            let exclude: Vec<String> = file
                .passkeys
                .iter()
                .map(|r| b64url_encode(&r.credential_id))
                .collect();
            (Some(file.user_handle), exclude)
        }
        Ok(None) => (None, Vec::new()),
        Err(err) => return Some(store_error_response(err)),
    };
    let user_handle = match stored_handle {
        Some(handle) => handle,
        None => {
            let mut fresh = [0u8; USER_HANDLE_BYTES];
            if (shared.state.random)(&mut fresh).is_err() {
                return Some(Response::json(503, "unavailable"));
            }
            fresh
        }
    };
    let mut challenge = Zeroizing::new([0u8; CHALLENGE_BYTES]);
    if (shared.state.random)(challenge.as_mut_slice()).is_err() {
        return Some(Response::json(503, "unavailable"));
    }
    let now = Instant::now();
    if auth
        .challenges
        .issue(
            now,
            PathClass::Tailnet,
            ChallengePurpose::Register,
            ChallengeBinding::EnrollCode(code_hash),
            Some(PendingRegistration { user_handle }),
            *challenge,
        )
        .is_err()
    {
        return Some(Response::json(429, "too_many_challenges"));
    }
    auth.record_issuance(key, now, &[]);
    drop(auth);
    Some(json_value(
        200,
        &serde_json::json!({
            "challenge": b64url_encode(challenge.as_slice()),
            "rp_id": rp_id,
            "timeout_ms": WEBAUTHN_TIMEOUT_MS,
            "user_id": b64url_encode(&user_handle),
            "exclude_credentials": exclude,
        }),
    ))
}

/// `POST /api/auth/register/verify` (tailnet only).
async fn register_verify<S: SessionSource>(
    shared: &Shared<S>,
    ctx: &RequestContext,
    stream: &mut UnixStream,
) -> Option<Response> {
    if let Err(response) = auth_route_gate(shared, ctx, ACTION_REGISTER) {
        return Some(response);
    }
    let key = LimitKey::Tailnet;
    if shared.auth.lock().await.locked_out(key, Instant::now()) {
        return Some(Response::json(429, "rate_limited"));
    }
    let body = match read_request_body(stream, ctx).await {
        BodyOutcome::Body(body) => body,
        BodyOutcome::Refuse(response, counted) => {
            if counted {
                shared.fail(key).await;
            }
            return Some(response);
        }
        BodyOutcome::Close => return None,
    };
    let Ok(registration) = decode_registration(&body) else {
        drop(body);
        shared.fail(key).await;
        return Some(Response::json(400, "bad_request"));
    };
    drop(body);
    let Some(rp) = shared.rp.as_ref() else {
        return Some(Response::json(403, "passkeys_not_configured"));
    };
    let Ok(client) = parse_client_data(rp, &registration.client_data_json, CeremonyType::Create)
    else {
        shared.fail(key).await;
        return Some(Response::json(403, "passkey_rejected"));
    };
    let code_hash = match shared.check_code(None).await {
        CodeCheck::Valid(hash) => hash,
        CodeCheck::Rejected => {
            shared.fail(key).await;
            return Some(Response::json(403, "enroll_code_rejected"));
        }
    };
    let verified = {
        let mut auth = shared.auth.lock().await;
        let taken = auth.challenges.take(
            Instant::now(),
            PathClass::Tailnet,
            ChallengePurpose::Register,
            &ChallengeBinding::EnrollCode(code_hash),
            &client.challenge,
        );
        let checked = match taken {
            Ok(Taken::Register(pending)) => {
                verify_registration(rp, &registration.id, &registration.attestation_object)
                    .map(|credential| (pending, credential))
                    .ok()
            }
            Ok(Taken::Plain) | Err(_) => None,
        };
        if checked.is_none() {
            auth.record_failure(key, Instant::now(), &[]);
        }
        checked
    };
    drop(registration);
    let Some((pending, credential)) = verified else {
        return Some(Response::json(403, "passkey_rejected"));
    };
    let created_unix_s = shared.now_unix_s();
    let written = shared
        .store_update(move |current| {
            let record = PasskeyRecord {
                credential_id: credential.credential_id,
                public_key: credential.public_key,
                sign_count: credential.sign_count,
                backup_eligible: credential.backup_eligible,
                backup_state: credential.backup_state,
                created_unix_s,
            };
            let mut file = match current {
                None => PasskeyFile {
                    user_handle: pending.user_handle,
                    passkeys: Vec::new(),
                },
                Some(file) => file,
            };
            if !bool::from(file.user_handle.ct_eq(&pending.user_handle)) {
                return Err(StoreError::UserHandleConflict);
            }
            if file.passkeys.iter().any(|r| {
                r.credential_id.len() == record.credential_id.len()
                    && bool::from(r.credential_id.ct_eq(&record.credential_id))
            }) {
                return Err(StoreError::Duplicate);
            }
            if file.passkeys.len() >= MAX_PASSKEYS {
                return Err(StoreError::Full);
            }
            file.passkeys.push(record);
            Ok((file, ()))
        })
        .await;
    match written {
        Ok(()) => {
            if let Some(dir) = shared.socket_dir() {
                remove_code_file(dir);
            }
            shared.auth.lock().await.reset_code_attempts();
            audit::passkey_registered();
            Some(Response::json(200, "registered"))
        }
        Err(StoreError::UserHandleConflict) => Some(Response::json(409, "registration_conflict")),
        Err(StoreError::Duplicate) => Some(Response::json(409, "already_registered")),
        Err(StoreError::Full) => Some(Response::json(409, "passkey_limit")),
        Err(err) => Some(store_error_response(err)),
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
async fn serve_stream<S: SessionSource>(
    mut stream: UnixStream,
    shared: Arc<Shared<S>>,
    session: Option<[u8; 32]>,
) {
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
    // A Funnel stream re-validates its web session (revocation first, `Touch::Keep`) at every
    // keep-alive tick and ends as soon as the session is gone (spec §5, F-4).
    let mut session_check_at = deadline(opened_at, keepalive);

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
            () = sleep_until(session_check_at), if session.is_some() => {
                if let Some(hash) = session.as_ref() {
                    if !shared.session_still_valid(hash).await {
                        debug!("stream session ended");
                        break;
                    }
                }
                session_check_at = deadline(Instant::now(), keepalive);
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
