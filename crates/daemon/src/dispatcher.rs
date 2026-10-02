//! Connection dispatcher with bounded concurrency and request routing.

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::time::timeout;
use tracing::{debug, info, warn};
use zeroize::Zeroizing;

use crate::config::DispatcherConfig;
use crate::error::DaemonError;
use crate::health::HealthState;
use crate::inference::{InferenceGate, RequestDeadline};
use crate::limits::{PeerConnectionLimiter, PeerLimitsConfig};
use crate::logging::short_request_id;
use crate::peercred::{get_peer_credentials, verify_peer_credentials, PeerCredentials};
use crate::pipeline::{
    classify_template, current_monotonic_nanos, PipelineComponents, TemplateModelBinding,
    FRAME_POLL_INTERVAL_MS, MAX_FRAME_AGE_NS,
};
use crate::preview::{
    authorize_preview, preview_image_for_frame, PreviewConfig, PREVIEW_FORMAT_EMPTY,
};
use crate::session::SessionValidator;
use crate::session_policy::LocalSessionPolicy;
use crate::shutdown::BlockingTasks;
use soos_camera_v4l::{Frame, PixelFormat};
use soos_evidence_store::{EvidenceFrame, EvidencePixelFormat, EvidenceStore, FrameMetadata};
use soos_policy::{ConsensusDecision, FrameClass, FrameEvaluation, PadAggregator, RateLimiter};
use soos_protocol::codec::{encode, encode_preview, CodecError};
use soos_protocol::message::{decode_client_message, ClientMessage, FrameFormat};
use soos_protocol::types::{
    Event, EventKind, PreviewResponse, ReasonClass, Request, RequestId, RequestKind, Response,
    StatusResponse, Verdict, CURRENT_VERSION, MAX_MESSAGE_SIZE,
};

/// Validity window of a rendered `Response`: `expires = issued + RESPONSE_VALIDITY_NS`
/// (GitHub #258). The PAM client rejects a response whose window is unset, inverted or
/// closed when it reads it (ADR 2026-10-01 "PAM Client Enforces Response Expiry").
pub const RESPONSE_VALIDITY_NS: u64 = 2_000_000_000;

/// Builds the `Response` for `verdict` from one monotonic clock reading (GitHub #258).
///
/// A working clock (`Ok(now)` with `now > 0`) gives `issued_monotonic_ns = now` and
/// `expires_monotonic_ns = now + RESPONSE_VALIDITY_NS` (saturating). When the clock fails
/// or reads zero, the response carries `issued == expires == 0`, i.e. already expired, and
/// an `Allow` verdict is downgraded to `Unavailable` / `InternalError`: an authorization
/// is never rendered without a clock (fail-closed). Other verdicts are kept unchanged.
/// This is the only stamping path of `ConnectionDispatcher::build_response` (GitHub #287).
#[must_use]
pub fn stamp_response(
    request_id: RequestId,
    verdict: Verdict,
    reason_class: ReasonClass,
    clock_reading: Result<u64, DaemonError>,
) -> Response {
    let (verdict, reason_class, issued_monotonic_ns, expires_monotonic_ns) = match clock_reading {
        Ok(now_ns) if now_ns > 0 => (
            verdict,
            reason_class,
            now_ns,
            now_ns.saturating_add(RESPONSE_VALIDITY_NS),
        ),
        Ok(_) | Err(_) => {
            if verdict == Verdict::Allow {
                warn!(
                    "Monotonic clock unavailable while rendering an Allow verdict; \
                     downgrading to Unavailable (fail-closed)"
                );
                (Verdict::Unavailable, ReasonClass::InternalError, 0, 0)
            } else {
                (verdict, reason_class, 0, 0)
            }
        }
    };
    Response {
        version: CURRENT_VERSION,
        request_id,
        verdict,
        reason_class,
        issued_monotonic_ns,
        expires_monotonic_ns,
    }
}

/// Evidence reason recorded for a `PasswordFailed` event snapshot.
pub const PASSWORD_FAILED_EVIDENCE_REASON: &str = "PasswordFailed";

/// Evidence reason recorded for the capture that vetoed a request as a presentation
/// attack (GitHub #261 / PAD-14).
pub const PAD_FAILED_EVIDENCE_REASON: &str = "PadFailed";

/// Internal representation of processed connection output before socket transmission.
#[derive(Debug)]
struct ProcessedOutput {
    /// Serialized wire response (including 4-byte BE length prefix), if a response is expected.
    /// Wrapped in `Zeroizing` because preview responses carry camera pixel data.
    encoded_response: Option<Zeroizing<Vec<u8>>>,
    /// Deferred error to return after response transmission (e.g. wire validation error).
    completion_error: Option<DaemonError>,
    /// Close the connection after this output (an `Auth` request is one-shot, GitHub #157).
    one_shot: bool,
}

/// Internal representation of a request response before transmission.
#[derive(Debug)]
struct ResponseOutput {
    encoded_response: Zeroizing<Vec<u8>>,
    completion_error: Option<DaemonError>,
}

/// Maps the camera pixel format to the self-describing evidence format (GitHub #181),
/// so an intrusion snapshot records how its payload bytes must be decoded.
fn evidence_pixel_format(format: PixelFormat) -> EvidencePixelFormat {
    match format {
        PixelFormat::Yuyv => EvidencePixelFormat::Yuyv,
        PixelFormat::Rgb24 => EvidencePixelFormat::Rgb24,
        PixelFormat::Grey => EvidencePixelFormat::Gray8,
        PixelFormat::Mjpeg => EvidencePixelFormat::Mjpeg,
        PixelFormat::Nv12 => EvidencePixelFormat::Nv12,
    }
}

/// Seals one camera capture into the evidence store under `reason` (GitHub #181, #261).
///
/// The caller checks the opt-in `evidence.enabled` gate; the store itself enforces it again
/// together with `daily_cap_per_uid`, the global daily cap, AES-256-GCM encryption and the
/// retention rotation. Failures are logged without any pixel data and never propagate.
fn store_evidence_capture(store: &EvidenceStore, uid: u32, reason: &str, frame: &Frame) {
    // GitHub #181: persist the frame with its dimensions and pixel format so the evidence
    // can be decoded later.
    let evidence = EvidenceFrame {
        metadata: FrameMetadata {
            width: frame.width,
            height: frame.height,
            pixel_format: evidence_pixel_format(frame.format),
            captured_at_mono_ns: frame.timestamp_mono_ns,
            sequence: frame.sequence,
        },
        data: &frame.data,
    };
    match store.store_frame_snapshot(uid, reason, &evidence, None, None) {
        Ok(snap_res) => {
            info!(
                uid = uid,
                reason = reason,
                "Intrusion evidence snapshot stored"
            );
            let _ = store.rotate_retention(&snap_res.date);
        }
        Err(err) => {
            warn!(
                uid = uid,
                reason = reason,
                error = %err,
                "Failed to store intrusion evidence snapshot"
            );
        }
    }
}

/// Whether a capture taken at `timestamp_ns` is still fresh at `now_ns` (`MAX_FRAME_AGE_NS`).
///
/// Captures without a timestamp, or stamped in the future, are accepted as before.
fn is_frame_fresh(timestamp_ns: u64, now_ns: u64) -> bool {
    if timestamp_ns > 0 && now_ns > timestamp_ns {
        now_ns.saturating_sub(timestamp_ns) <= MAX_FRAME_AGE_NS
    } else {
        true
    }
}

/// Connection dispatcher managing concurrent incoming client requests.
pub struct ConnectionDispatcher {
    config: DispatcherConfig,
    health: Arc<HealthState>,
    pipeline: Option<PipelineComponents>,
    connection_limiter: PeerConnectionLimiter,
    start_time: Instant,
    session_validator: SessionValidator,
    session_policy: LocalSessionPolicy,
    clock_fn: fn() -> Result<u64, DaemonError>,
    preview: PreviewConfig,
    preview_limiter: tokio::sync::Mutex<RateLimiter>,
    inference: InferenceGate,
    peer_limits: PeerLimitsConfig,
    event_limiter: tokio::sync::Mutex<RateLimiter>,
    expected_embedding_model: Option<String>,
    /// Tracked spoof and `PasswordFailed` evidence writes, drained at shutdown (GitHub #287,
    /// #310).
    evidence_writes: BlockingTasks,
}

impl ConnectionDispatcher {
    /// Creates a new connection dispatcher wrapping configuration and health state.
    pub fn new(config: DispatcherConfig, health: Arc<HealthState>) -> Self {
        let peer_limits = PeerLimitsConfig::default();
        let connection_limiter =
            PeerConnectionLimiter::new(config.max_concurrent_connections, &peer_limits);
        let event_limiter =
            tokio::sync::Mutex::new(RateLimiter::new(peer_limits.event_rate_limit_config()));
        let session_validator = if config.enforce_active_session {
            SessionValidator::with_sessions_dir(config.logind_sessions_dir.clone())
        } else {
            SessionValidator::disabled()
        };
        let session_policy = LocalSessionPolicy::from_validator(&session_validator);
        let preview = PreviewConfig::default();
        let preview_limiter =
            tokio::sync::Mutex::new(RateLimiter::new(preview.rate_limit_config()));
        Self {
            config,
            health,
            pipeline: None,
            connection_limiter,
            start_time: Instant::now(),
            session_validator,
            session_policy,
            clock_fn: current_monotonic_nanos,
            preview,
            preview_limiter,
            inference: InferenceGate::default(),
            peer_limits,
            event_limiter,
            expected_embedding_model: None,
            evidence_writes: BlockingTasks::new(),
        }
    }

    /// Creates a new connection dispatcher with an active pipeline.
    pub fn with_pipeline(
        config: DispatcherConfig,
        health: Arc<HealthState>,
        pipeline: PipelineComponents,
    ) -> Self {
        let peer_limits = PeerLimitsConfig::default();
        let connection_limiter =
            PeerConnectionLimiter::new(config.max_concurrent_connections, &peer_limits);
        let event_limiter =
            tokio::sync::Mutex::new(RateLimiter::new(peer_limits.event_rate_limit_config()));
        let session_validator = if config.enforce_active_session {
            SessionValidator::with_sessions_dir(config.logind_sessions_dir.clone())
        } else {
            SessionValidator::disabled()
        };
        let session_policy = LocalSessionPolicy::from_validator(&session_validator);
        let preview = PreviewConfig::default();
        let preview_limiter =
            tokio::sync::Mutex::new(RateLimiter::new(preview.rate_limit_config()));
        Self {
            config,
            health,
            pipeline: Some(pipeline),
            connection_limiter,
            start_time: Instant::now(),
            session_validator,
            session_policy,
            clock_fn: current_monotonic_nanos,
            preview,
            preview_limiter,
            inference: InferenceGate::default(),
            peer_limits,
            event_limiter,
            expected_embedding_model: None,
            evidence_writes: BlockingTasks::new(),
        }
    }

    /// Binds enrolled templates to the loaded embedding model (GitHub #182 / STO-09).
    ///
    /// When set, an `Auth` request whose enrolled template records a different
    /// `model_id` is answered `Unavailable` / `ModelUnavailable` without being matched,
    /// so PAM falls back to the next module. `soos-daemon` always sets it to
    /// [`crate::pipeline::EMBEDDING_MODEL_ID`]; mock harnesses may leave it unset.
    #[must_use]
    pub fn with_expected_embedding_model(mut self, model_id: impl Into<String>) -> Self {
        self.expected_embedding_model = Some(model_id.into());
        self
    }

    /// Returns the embedding model identifier enrolled templates must carry, if bound.
    #[must_use]
    pub fn expected_embedding_model(&self) -> Option<&str> {
        self.expected_embedding_model.as_deref()
    }

    /// Returns the tracked spoof and `PasswordFailed` evidence writes (GitHub #310);
    /// `soos-daemon` drains them at shutdown
    /// within what is left of the connection drain budget (GitHub #287).
    #[must_use]
    pub const fn evidence_writes(&self) -> &BlockingTasks {
        &self.evidence_writes
    }

    /// Overrides the monotonic clock function (used for simulation and test harnesses).
    #[must_use]
    pub fn with_clock_fn(mut self, clock_fn: fn() -> Result<u64, DaemonError>) -> Self {
        self.clock_fn = clock_fn;
        self
    }

    /// Overrides the session validator (used for custom or mock session directories).
    #[must_use]
    pub fn with_session_validator(mut self, validator: SessionValidator) -> Self {
        self.session_policy = LocalSessionPolicy::from_validator(&validator);
        self.session_validator = validator;
        self
    }

    /// Overrides the local-session policy applied to `Auth` requests (GitHub #160).
    #[must_use]
    pub fn with_session_policy(mut self, policy: LocalSessionPolicy) -> Self {
        self.session_policy = policy;
        self
    }

    /// Configures the `[preview]` authorization policy (GitHub #143).
    ///
    /// Without this call the dispatcher keeps [`PreviewConfig::default`], which serves
    /// preview frames to a root peer only (fail-closed).
    #[must_use]
    pub fn with_preview_config(mut self, preview: PreviewConfig) -> Self {
        self.preview_limiter =
            tokio::sync::Mutex::new(RateLimiter::new(preview.rate_limit_config()));
        self.preview = preview;
        self
    }

    /// Overrides the vision inference gate (slot count and initial latency estimate).
    ///
    /// Without this call the dispatcher uses [`InferenceGate::default`]
    /// (`MAX_CONCURRENT_INFERENCES` slots, `DEFAULT_INFERENCE_ESTIMATE_MS` estimate).
    #[must_use]
    pub fn with_inference_gate(mut self, gate: InferenceGate) -> Self {
        self.inference = gate;
        self
    }

    /// Returns the vision inference gate (GitHub #158).
    #[must_use]
    pub const fn inference_gate(&self) -> &InferenceGate {
        &self.inference
    }

    /// Configures the per-peer connection and event limits (`[peer_limits]`, GitHub #157).
    ///
    /// Without this call the dispatcher keeps [`PeerLimitsConfig::default`].
    #[must_use]
    pub fn with_peer_limits(mut self, limits: PeerLimitsConfig) -> Self {
        self.connection_limiter =
            PeerConnectionLimiter::new(self.config.max_concurrent_connections, &limits);
        self.event_limiter =
            tokio::sync::Mutex::new(RateLimiter::new(limits.event_rate_limit_config()));
        self.peer_limits = limits;
        self
    }

    /// Returns the active per-peer limits.
    #[must_use]
    pub const fn peer_limits(&self) -> &PeerLimitsConfig {
        &self.peer_limits
    }

    /// Returns the active preview authorization policy.
    #[must_use]
    pub const fn preview_config(&self) -> &PreviewConfig {
        &self.preview
    }

    fn now_nanos(&self) -> Result<u64, DaemonError> {
        (self.clock_fn)()
    }

    /// Returns the number of currently available concurrency permits (root view: permits
    /// reserved for root peers included).
    pub fn available_permits(&self) -> usize {
        self.connection_limiter.available()
    }

    /// Processes an incoming Unix domain stream through authentication,
    /// bounded reading, peer verification, and response generation.
    ///
    /// Admission is keyed on the kernel `SO_PEERCRED` UID (GitHub #157): a refused peer's
    /// stream is closed immediately, without reading or answering anything. An admitted
    /// connection serves at most `max_requests_per_connection` requests, is not read again
    /// once `max_connection_lifetime` has elapsed, and is closed right after an `Auth`
    /// response (one-shot).
    pub async fn handle_connection(&self, mut stream: UnixStream) -> Result<(), DaemonError> {
        let peer = match get_peer_credentials(&stream) {
            Ok(peer) => peer,
            Err(err) => {
                warn!(error = %err, "SO_PEERCRED lookup failed; closing connection");
                return Err(err);
            }
        };
        let _permit = match self.connection_limiter.try_acquire(peer.uid) {
            Ok(permit) => permit,
            Err(rejection) => {
                warn!(
                    peer_uid = peer.uid,
                    reason = %rejection,
                    "Connection limit reached; closing connection"
                );
                return Err(DaemonError::ConcurrencyLimitReached);
            }
        };

        let opened_at = Instant::now();
        let mut requests_processed: usize = 0;
        loop {
            if requests_processed >= self.peer_limits.max_requests_per_connection {
                info!(
                    peer_uid = peer.uid,
                    requests = requests_processed,
                    "Per-connection request cap reached; closing connection"
                );
                break;
            }
            if opened_at.elapsed() >= self.peer_limits.max_connection_lifetime {
                info!(
                    peer_uid = peer.uid,
                    requests = requests_processed,
                    "Connection lifetime cap reached; closing connection"
                );
                break;
            }

            // Phase 1: Request Reading & Processing Phase.
            // Bounded by connection_timeout. Strictly performs socket reading and pipeline computation,
            // producing a fully encoded in-memory response buffer (Option<Vec<u8>>).
            // Zero response bytes are written during this phase, guaranteeing that async cancellation
            // upon timeout will never leave partial response bytes on the wire.
            let res = timeout(
                self.config.connection_timeout,
                self.read_and_process(&mut stream, peer),
            )
            .await;

            let output = match res {
                Ok(Ok(output)) => {
                    requests_processed = requests_processed.saturating_add(1);
                    output
                }
                Ok(Err(DaemonError::Io(e))) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    // Client disconnected cleanly.
                    break;
                }
                Ok(Err(e)) => {
                    return Err(e);
                }
                Err(_) => {
                    if requests_processed == 0 {
                        warn!("Connection timed out");
                        return Err(DaemonError::Timeout);
                    } else {
                        // After servicing requests, client went idle past timeout; disconnect gracefully.
                        debug!("Persistent connection timed out while idle");
                        break;
                    }
                }
            };

            // Phase 2: Response Transmission Phase.
            // Runs outside the request processing timeout. Writes the pre-encoded response frame
            // atomically using a dedicated write timeout to prevent slow client stalls.
            if let Some(ref encoded_resp) = output.encoded_response {
                self.write_response(&mut stream, encoded_resp).await?;
            }

            if let Some(err) = output.completion_error {
                return Err(err);
            }

            if output.one_shot {
                debug!("Auth request served; closing one-shot connection");
                break;
            }
        }

        Ok(())
    }

    async fn read_and_process(
        &self,
        stream: &mut UnixStream,
        peer: PeerCredentials,
    ) -> Result<ProcessedOutput, DaemonError> {
        // Start of the outer `connection_timeout` window for this request: every later budget
        // (camera wake, consensus loop, inference admission) is measured from here (#159).
        let request_started = Instant::now();

        // Step 1: Peer credentials were extracted once via SO_PEERCRED at admission.

        // Step 2: Read framed length prefix (4 bytes big-endian)
        let mut len_bytes = [0u8; 4];
        stream.read_exact(&mut len_bytes).await?;
        let declared_size = usize::try_from(u32::from_be_bytes(len_bytes)).map_err(|_| {
            DaemonError::OversizedPayload {
                size: usize::MAX,
                max: MAX_MESSAGE_SIZE,
            }
        })?;

        // Step 3: Bounded memory safety assertion
        if declared_size > MAX_MESSAGE_SIZE || declared_size == 0 {
            warn!(
                declared_size = declared_size,
                max = MAX_MESSAGE_SIZE,
                "Rejected oversized message"
            );
            return Err(DaemonError::OversizedPayload {
                size: declared_size,
                max: MAX_MESSAGE_SIZE,
            });
        }

        let mut body_buffer = vec![0u8; declared_size];
        stream.read_exact(&mut body_buffer).await?;

        // Step 4: Classify the frame by protocol rule (GitHub #204 / DMN-15): the message tag
        // trailer selects the type; a legacy untagged frame is accepted only when it decodes
        // exactly as one type. A frame decoding as both is rejected, never guessed.
        let classified = decode_client_message(&body_buffer);
        drop(body_buffer);
        let message = match classified {
            Ok((message, format)) => {
                if format == FrameFormat::Legacy {
                    debug!("Accepted legacy untagged client message");
                }
                message
            }
            Err(err) => {
                warn!(error = %err, "Rejected client message; closing connection");
                return Err(DaemonError::Protocol(format!(
                    "Rejected client frame: {err}"
                )));
            }
        };

        match message {
            ClientMessage::Request(req) => {
                let one_shot = req.kind == RequestKind::Auth;
                let res = self
                    .handle_request(peer.uid, peer.pid, req, request_started)
                    .await?;
                Ok(ProcessedOutput {
                    encoded_response: Some(res.encoded_response),
                    completion_error: res.completion_error,
                    one_shot,
                })
            }
            ClientMessage::Event(event) => {
                self.handle_event(peer.uid, event).await?;
                Ok(ProcessedOutput {
                    encoded_response: None,
                    completion_error: None,
                    one_shot: false,
                })
            }
        }
    }

    async fn handle_event(&self, peer_uid: u32, event: Event) -> Result<(), DaemonError> {
        if event.version != CURRENT_VERSION {
            warn!(version = event.version, "Unsupported event version");
            return Err(DaemonError::Protocol("Unsupported event version".into()));
        }

        if event.kind == EventKind::PasswordFailed {
            // Per-peer-UID event quota (GitHub #175 hardening): bounds how often any peer,
            // root included, can make the daemon capture an evidence snapshot.
            // Trust model (AI/DECISIONS.md, GitHub #175): a root peer (sudo, su, login,
            // gdm-session-worker) may report for any UID; any other peer only for itself.
            if peer_uid != 0 && event.uid.is_some_and(|claimed| claimed != peer_uid) {
                warn!(
                    peer_uid = peer_uid,
                    claimed_uid = ?event.uid,
                    "Unprivileged peer reported an event for another UID; dropping event"
                );
                return Ok(());
            }
            if !self.event_within_quota(peer_uid).await {
                return Ok(());
            }
            let target_uid = event.uid.unwrap_or(peer_uid);
            info!(
                peer_uid = peer_uid,
                target_uid = target_uid,
                "Processing telemetry auth failure event"
            );
            if let Some(ref pipe) = self.pipeline {
                if pipe.evidence_store.config().enabled {
                    if let Some(frame) = pipe.camera.latest_frame() {
                        // GitHub #310: encryption, fsync and the retention `flock` are
                        // blocking; run them on the blocking pool, tracked in the same set as
                        // the spoof evidence writes so that shutdown drains them, and never on
                        // a Tokio worker.
                        let store = Arc::clone(&pipe.evidence_store);
                        self.evidence_writes.spawn_blocking(move || {
                            store_evidence_capture(
                                &store,
                                target_uid,
                                PASSWORD_FAILED_EVIDENCE_REASON,
                                &frame,
                            );
                        });
                    } else {
                        warn!("No camera capture available for evidence snapshot");
                    }
                }
            }
        }

        Ok(())
    }

    /// Records one `PasswordFailed` event of `peer_uid` against the per-peer quota.
    ///
    /// Returns `false` (event dropped, fail closed) when the quota is exhausted, the
    /// limiter table is full, or the monotonic clock is unavailable.
    async fn event_within_quota(&self, peer_uid: u32) -> bool {
        let now_ns = match self.now_nanos() {
            Ok(ns) => ns,
            Err(err) => {
                warn!(error = %err, "Monotonic clock unavailable; dropping event");
                return false;
            }
        };
        let mut limiter = self.event_limiter.lock().await;
        match limiter.check_and_record(peer_uid, now_ns) {
            Ok(()) => true,
            Err(err) => {
                warn!(
                    peer_uid = peer_uid,
                    error = %err,
                    "Event quota exceeded for peer; dropping PasswordFailed event"
                );
                false
            }
        }
    }

    async fn handle_request(
        &self,
        peer_uid: u32,
        peer_pid: Option<i32>,
        req: Request,
        request_started: Instant,
    ) -> Result<ResponseOutput, DaemonError> {
        // Step 5a: Wire protocol validation (version and bounded fields)
        if let Err(val_err) = req.validate() {
            warn!(error = %val_err, "Request wire validation failed; rejecting with ProtocolError");
            let encoded = self.build_response(
                req.request_id,
                Verdict::ProtocolError,
                ReasonClass::MalformedRequest,
            )?;
            return Ok(ResponseOutput {
                encoded_response: encoded,
                completion_error: Some(DaemonError::Validation(val_err)),
            });
        }

        // Step 5b: Diagnostic status query (non-biometric)
        if req.kind == RequestKind::Status {
            let status = self.health.snapshot();
            let status_resp = StatusResponse {
                version: CURRENT_VERSION,
                socket_ready: status.socket_ready,
                camera_ready: status.camera_ready,
                models_verified: status.models_verified,
                is_healthy: status.is_healthy,
                pid: std::process::id(),
                uptime_secs: self.start_time.elapsed().as_secs(),
                memory_locked: self.health.memory_locked(),
            };
            let encoded = Zeroizing::new(encode(&status_resp)?);
            debug!("Generated diagnostic status response");
            return Ok(ResponseOutput {
                encoded_response: encoded,
                completion_error: None,
            });
        }

        // Step 6: Verify peer credentials against request
        let peer_cred = nix::unistd::Uid::from_raw(peer_uid);
        let cred_struct = crate::peercred::PeerCredentials {
            uid: peer_cred.as_raw(),
            gid: 0,
            pid: peer_pid,
        };
        if let Err(err) = verify_peer_credentials(&cred_struct, req.uid_hint) {
            let (verdict, reason_class) = match err {
                DaemonError::UidMismatch {
                    peer_uid,
                    requested_uid,
                } => {
                    warn!(
                        peer_uid = peer_uid,
                        target_uid = requested_uid,
                        "Peer UID mismatch detected; rejecting"
                    );
                    (Verdict::ProtocolError, ReasonClass::UidMismatch)
                }
                _ => (Verdict::ProtocolError, ReasonClass::MalformedRequest),
            };
            let encoded = self.build_response(req.request_id, verdict, reason_class)?;
            return Ok(ResponseOutput {
                encoded_response: encoded,
                completion_error: None,
            });
        }

        // Step 6b: Local-session policy (GitHub #160): a root peer must be tied to its own
        // active, local, seat-attached session of the target UID; fails closed.
        let session_check = if req.kind == RequestKind::Auth {
            self.session_policy
                .authorize_auth(&cred_struct, req.uid_hint)
        } else {
            Ok(())
        };
        if let Err(denial) = session_check {
            warn!(
                peer_uid = peer_uid,
                target_uid = req.uid_hint,
                reason = denial.as_str(),
                "Local-session policy refused auth request; password fallback"
            );
            let encoded = self.build_response(
                req.request_id,
                Verdict::ProtocolError,
                ReasonClass::UidMismatch,
            )?;
            return Ok(ResponseOutput {
                encoded_response: encoded,
                completion_error: None,
            });
        }

        // Step 6c: Camera preview stream (GitHub #143): served only after kernel peer
        // verification, explicit authorization, session validation and rate limiting.
        if req.kind == RequestKind::PreviewFrame {
            return self.handle_preview_request(peer_uid, &req).await;
        }

        // Step 7: Monotonic deadline propagation check
        let now_ns = match self.now_nanos() {
            Ok(ns) => ns,
            Err(err) => {
                warn!(
                    error = %err,
                    "Failed to query monotonic clock; returning Unavailable"
                );
                let encoded = self.build_response(
                    req.request_id,
                    Verdict::Unavailable,
                    ReasonClass::InternalError,
                )?;
                return Ok(ResponseOutput {
                    encoded_response: encoded,
                    completion_error: Some(err),
                });
            }
        };

        if req.deadline_monotonic_ns > 0 && now_ns >= req.deadline_monotonic_ns {
            warn!(
                now_ns = now_ns,
                deadline = req.deadline_monotonic_ns,
                "Request exceeded monotonic deadline before processing"
            );
            let encoded =
                self.build_response(req.request_id, Verdict::Unavailable, ReasonClass::Timeout)?;
            return Ok(ResponseOutput {
                encoded_response: encoded,
                completion_error: None,
            });
        }

        // Step 8: Full pipeline processing
        if let Some(ref pipe) = self.pipeline {
            // Single request deadline (GitHub #159), computed once and threaded through the
            // camera wake wait and the consensus loop: min(client deadline, request start +
            // connection_timeout), each minus the response write margin.
            let deadline = RequestDeadline::compute(
                now_ns,
                req.deadline_monotonic_ns,
                request_started,
                self.config.connection_timeout,
            );

            // 8a: Fetch the enrolled template once, BEFORE the attempt reservation and the
            // camera wake (GitHub #298). The lookup result is kept as-is and answered below,
            // after the reservation, so a missing template or a store error keeps counting
            // as an attempt exactly as before (GitHub #200 / DMN-10).
            let template_lookup = pipe.biometric_store.get(req.uid_hint);

            // 8b: Refuse a template enrolled with another embedding model or of another
            // vector length (GitHub #182, #278) before the reservation, the camera wake and
            // any capture (GitHub #298). Such a template can never authenticate (no
            // inference runs), so it consumes no attempt and never turns the camera on.
            if let (Some(expected), Ok(Some(enrolled_template))) =
                (self.expected_embedding_model.as_deref(), &template_lookup)
            {
                let binding = classify_template(
                    expected,
                    pipe.vision.embedding_dimension(),
                    &enrolled_template.model_id,
                    enrolled_template.embedding.len(),
                );
                if binding == TemplateModelBinding::Foreign {
                    warn!(
                        uid = req.uid_hint,
                        template_model = ?enrolled_template.model_id,
                        template_dimension = enrolled_template.embedding.len(),
                        loaded_model = %expected,
                        "Enrolled template was recorded with a different embedding model; \
                         re-enrollment required, returning Unavailable"
                    );
                    let encoded = self.build_response(
                        req.request_id,
                        Verdict::Unavailable,
                        ReasonClass::ModelUnavailable,
                    )?;
                    return Ok(ResponseOutput {
                        encoded_response: encoded,
                        completion_error: None,
                    });
                }
            }

            // 8c: Atomic attempt reservation (GitHub #200 / DMN-10). The per-UID limit is
            // checked AND recorded under one write-lock acquisition, before any camera or
            // vision work: concurrent requests cannot all pass the same remaining attempt,
            // and a request that ends early (not enrolled, store error, camera down,
            // cancelled) still counts. Only a foreign template (8b) is answered before it.
            // A rejected reservation fails fast without waking the camera.
            {
                let mut engine = pipe.policy.write().await;
                if let Err(err) = engine.record_attempt(req.uid_hint, now_ns) {
                    warn!(
                        uid = req.uid_hint,
                        error = %err,
                        "UID has exceeded rate limit quota; rejecting request immediately"
                    );
                    let encoded = self.build_response(
                        req.request_id,
                        Verdict::ProtocolError,
                        ReasonClass::RateLimited,
                    )?;
                    return Ok(ResponseOutput {
                        encoded_response: encoded,
                        completion_error: None,
                    });
                }
            }

            // 8d: Answer a missing template or a store error (attempt already recorded),
            // still before the camera wake.
            let enrolled_template = match template_lookup {
                Ok(Some(tmpl)) => tmpl,
                Ok(None) => {
                    info!(
                        uid = req.uid_hint,
                        "Target UID is not enrolled; returning Unavailable"
                    );
                    let encoded = self.build_response(
                        req.request_id,
                        Verdict::Unavailable,
                        ReasonClass::InternalError,
                    )?;
                    return Ok(ResponseOutput {
                        encoded_response: encoded,
                        completion_error: None,
                    });
                }
                Err(err) => {
                    warn!(
                        error = %err,
                        uid = req.uid_hint,
                        "Biometric store error retrieving template"
                    );
                    let encoded = self.build_response(
                        req.request_id,
                        Verdict::Unavailable,
                        ReasonClass::InternalError,
                    )?;
                    return Ok(ResponseOutput {
                        encoded_response: encoded,
                        completion_error: None,
                    });
                }
            };

            // 8d-1: Notify activity to wake camera from auto-standby (current template only)
            pipe.camera.notify_activity();

            // 8d-2: If camera is resuming from auto-standby, wait up to 1000-1200ms for it to become ready
            let max_wake = if req.deadline_monotonic_ns > 0
                && req.deadline_monotonic_ns < u64::MAX
                && req.deadline_monotonic_ns > now_ns
            {
                let remaining =
                    Duration::from_nanos(req.deadline_monotonic_ns.saturating_sub(now_ns));
                remaining.min(Duration::from_millis(1200))
            } else {
                Duration::from_millis(1000)
            };
            let max_wake = max_wake
                .min(
                    self.config
                        .connection_timeout
                        .saturating_sub(Duration::from_millis(100)),
                )
                .min(deadline.remaining(now_ns));

            if !pipe.camera.is_ready() {
                let wake_start = Instant::now();
                while !pipe.camera.is_ready() && wake_start.elapsed() < max_wake {
                    tokio::time::sleep(Duration::from_millis(15)).await;
                }
            }

            if !pipe.camera.is_ready() {
                warn!("Camera is not ready; rejecting auth request");
                let encoded = self.build_response(
                    req.request_id,
                    Verdict::Unavailable,
                    ReasonClass::CameraUnavailable,
                )?;
                return Ok(ResponseOutput {
                    encoded_response: encoded,
                    completion_error: None,
                });
            }

            // 8e: Multi-frame PAD consensus loop (GitHub #147 / PAD-02).
            // Allow requires k consecutive passing captures (live at or above the PAD
            // threshold and matching at or above the cosine threshold) inside a bounded
            // window; any spoof-classified capture vetoes the whole request (fail closed).
            let consensus_thresholds = *pipe.policy.read().await.thresholds();
            let mut aggregator = PadAggregator::with_defaults(consensus_thresholds);
            let mut last_sequence: Option<u64> = None;
            let mut last_capture_stale = false;
            // First capture classified as a presentation attack (GitHub #261 / PAD-14),
            // kept only to seal it as opt-in evidence once the verdict is rendered.
            let mut spoof_capture: Option<Arc<Frame>> = None;

            loop {
                let cur_ns = match self.now_nanos() {
                    Ok(ns) => ns,
                    Err(err) => {
                        warn!(error = %err, "Monotonic clock query failed checking loop deadline");
                        let encoded = self.build_response(
                            req.request_id,
                            Verdict::Unavailable,
                            ReasonClass::InternalError,
                        )?;
                        return Ok(ResponseOutput {
                            encoded_response: encoded,
                            completion_error: Some(err),
                        });
                    }
                };

                if deadline.remaining(cur_ns).is_zero() {
                    break;
                }

                if let Some(frame) = pipe.camera.latest_frame() {
                    let is_new = match last_sequence {
                        Some(seq) => frame.sequence != seq,
                        None => true,
                    };

                    if is_new {
                        last_sequence = Some(frame.sequence);

                        if is_frame_fresh(frame.timestamp_mono_ns, cur_ns) {
                            last_capture_stale = false;

                            // 8e-1: Inference admission (GitHub #158 / #159). Never start an
                            // inference whose estimated duration exceeds the remaining budget,
                            // and never queue behind a busy inference slot past the last
                            // feasible start time: finalize with the current consensus instead.
                            let estimate = self.inference.estimate();
                            if !deadline.can_start(cur_ns, estimate) {
                                debug!(
                                    estimate_ms = estimate.as_millis(),
                                    "Remaining budget below the inference estimate; finalizing"
                                );
                                // GitHub #315 (DMN-NEW-2): with no capture evaluated, no
                                // measurement would ever lower an estimate above every
                                // client budget; decay it so face auth recovers. This
                                // request still fails closed.
                                if aggregator.frames_evaluated() == 0 {
                                    self.inference.decay_estimate();
                                }
                                break;
                            }
                            let max_wait = deadline.remaining(cur_ns).saturating_sub(estimate);
                            let Some(permit) = self.inference.acquire_within(max_wait).await else {
                                debug!("Inference slot still busy at the last feasible start; finalizing");
                                break;
                            };
                            let start_ns = self.now_nanos().unwrap_or(u64::MAX);
                            if !deadline.can_start(start_ns, self.inference.estimate()) {
                                drop(permit);
                                break;
                            }
                            if !is_frame_fresh(frame.timestamp_mono_ns, start_ns) {
                                // Aged while waiting for the slot: never evaluate it.
                                drop(permit);
                                continue;
                            }

                            // 8e-2: CPU inference on the bounded blocking pool; the async
                            // worker stays free for the accept loop and Status requests.
                            let vision = Arc::clone(&pipe.vision);
                            let job_frame = Arc::clone(&frame);
                            let result = match self
                                .inference
                                .run(permit, move || vision.process_frame(&job_frame))
                                .await
                            {
                                Ok(result) => result,
                                Err(err) => {
                                    warn!(error = %err, "Vision inference job failed; failing closed");
                                    let encoded = self.build_response(
                                        req.request_id,
                                        Verdict::Unavailable,
                                        ReasonClass::InternalError,
                                    )?;
                                    return Ok(ResponseOutput {
                                        encoded_response: encoded,
                                        completion_error: None,
                                    });
                                }
                            };

                            let evaluation = match result {
                                Ok(output) => {
                                    let sim = match soos_vision::cosine_similarity(
                                        enrolled_template.embedding.as_slice(),
                                        output.embedding.as_slice(),
                                    ) {
                                        Ok(s) => s,
                                        Err(e) => {
                                            warn!(error = %e, "Cosine similarity calculation error");
                                            0.0
                                        }
                                    };
                                    FrameEvaluation::new(
                                        1,
                                        output.pad_result.is_live,
                                        output.pad_result.score,
                                        sim,
                                    )
                                }
                                Err(soos_vision::VisionError::PadFailed { score, threshold }) => {
                                    debug!(
                                        score = score,
                                        threshold = threshold,
                                        "Presentation attack detected (PAD failed)"
                                    );
                                    FrameEvaluation::spoof(score)
                                }
                                Err(soos_vision::VisionError::IrLivenessGateFailed { reason }) => {
                                    debug!(
                                        reason = %reason,
                                        "Presentation attack detected (IR liveness gate)"
                                    );
                                    FrameEvaluation::spoof(0.0)
                                }
                                Err(soos_vision::VisionError::NoFaceDetected) => {
                                    debug!("Zero faces detected in capture");
                                    FrameEvaluation::no_face()
                                }
                                Err(soos_vision::VisionError::MultipleFacesDetected { count }) => {
                                    let count_u8 = u8::try_from(count).unwrap_or(u8::MAX);
                                    debug!(count = count, "Multiple faces detected in capture");
                                    FrameEvaluation::multiple_faces(count_u8)
                                }
                                Err(soos_vision::VisionError::FaceBelowConfidence { .. }) => {
                                    debug!("Face detected below confidence threshold");
                                    FrameEvaluation::no_face()
                                }
                                // Pre-PAD quality gate (GitHub #218): an unusable capture,
                                // never a pass and never an internal error.
                                Err(
                                    soos_vision::VisionError::FaceTooSmall { .. }
                                    | soos_vision::VisionError::FaceBlurred { .. },
                                ) => {
                                    debug!("Face rejected by the pre-PAD quality gate");
                                    FrameEvaluation::no_face()
                                }
                                Err(soos_vision::VisionError::Inference(err)) => {
                                    warn!(error = %err, "Vision neural inference failure");
                                    let encoded = self.build_response(
                                        req.request_id,
                                        Verdict::Unavailable,
                                        ReasonClass::ModelUnavailable,
                                    )?;
                                    return Ok(ResponseOutput {
                                        encoded_response: encoded,
                                        completion_error: None,
                                    });
                                }
                                Err(err) => {
                                    warn!(error = %err, "Vision pipeline processing error");
                                    let encoded = self.build_response(
                                        req.request_id,
                                        Verdict::Unavailable,
                                        ReasonClass::InternalError,
                                    )?;
                                    return Ok(ResponseOutput {
                                        encoded_response: encoded,
                                        completion_error: None,
                                    });
                                }
                            };

                            let class = aggregator.record(&evaluation);
                            if class == FrameClass::Spoof && spoof_capture.is_none() {
                                spoof_capture = Some(Arc::clone(&frame));
                            }
                            match aggregator.decision() {
                                ConsensusDecision::Allow => break,
                                ConsensusDecision::SpoofVetoed => {
                                    warn!(
                                        uid = req.uid_hint,
                                        captures_evaluated = aggregator.frames_evaluated(),
                                        "Presentation attack detected; vetoing request"
                                    );
                                    break;
                                }
                                ConsensusDecision::Pending(_) => {
                                    debug!(
                                        class = ?class,
                                        consecutive_live = aggregator.consecutive_passing(),
                                        required = aggregator.config().required(),
                                        "Capture recorded; consensus pending"
                                    );
                                }
                            }
                        } else {
                            last_capture_stale = true;
                        }
                    }
                }

                // Deadline-aware poll: never sleep past the decision budget so the response
                // is always rendered before the connection timeout.
                let remaining = deadline.remaining(self.now_nanos().unwrap_or(u64::MAX));
                if remaining.is_zero() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(FRAME_POLL_INTERVAL_MS).min(remaining))
                    .await;
            }

            drop(enrolled_template);

            // 8f: Render the aggregate verdict. The attempt was already recorded by the
            // reservation in 8c (exactly one per request), so nothing is recorded here.
            let decision = aggregator.decision();
            let (final_verdict, final_reason) = match decision {
                ConsensusDecision::Allow => {
                    info!(
                        uid = req.uid_hint,
                        captures_evaluated = aggregator.frames_evaluated(),
                        consecutive_live = aggregator.consecutive_passing(),
                        "Face verification consensus reached; authorizing authentication"
                    );
                    decision.verdict()
                }
                ConsensusDecision::Pending(_) if last_capture_stale => {
                    (Verdict::Unavailable, ReasonClass::StaleFrame)
                }
                ConsensusDecision::SpoofVetoed | ConsensusDecision::Pending(_) => {
                    decision.verdict()
                }
            };

            // 8g: Opt-in spoof evidence (GitHub #261 / PAD-14): one snapshot of the capture
            // that vetoed the request, sealed off the response path.
            if decision == ConsensusDecision::SpoofVetoed {
                self.capture_spoof_evidence(pipe, req.uid_hint, spoof_capture);
            }

            if req.service.contains("gdm")
                || req.service.contains("lock")
                || req.service.contains("screen")
            {
                pipe.camera.notify_activity();
            }

            let encoded = self.build_response(req.request_id, final_verdict, final_reason)?;
            return Ok(ResponseOutput {
                encoded_response: encoded,
                completion_error: None,
            });
        }

        // Fail-closed fallback: never authorize authentication without an initialized pipeline
        warn!(
            request_id = %short_request_id(&req.request_id),
            "Rejecting authentication request: daemon pipeline is not initialized"
        );
        let encoded = self.build_response(
            req.request_id,
            Verdict::Unavailable,
            ReasonClass::InternalError,
        )?;
        Ok(ResponseOutput {
            encoded_response: encoded,
            completion_error: None,
        })
    }

    /// Seals the capture that vetoed a request as a presentation attack into the evidence
    /// store (GitHub #261 / PAD-14).
    ///
    /// Opt-in: nothing happens unless `[pipeline.evidence] enabled` is set. The write runs
    /// on the blocking pool without delaying the `Deny`/`PadFailed` response, tracked in
    /// [`ConnectionDispatcher::evidence_writes`] so that shutdown can drain it (GitHub
    /// #287); the evidence store enforces `daily_cap_per_uid`, the global daily cap,
    /// encryption and retention. At most one snapshot per request: the consensus loop stops
    /// at the first spoof.
    fn capture_spoof_evidence(
        &self,
        pipe: &PipelineComponents,
        target_uid: u32,
        capture: Option<Arc<Frame>>,
    ) {
        if !pipe.evidence_store.config().enabled {
            return;
        }
        let Some(frame) = capture else {
            warn!(
                uid = target_uid,
                "No spoof capture retained for the evidence snapshot"
            );
            return;
        };
        info!(
            uid = target_uid,
            "Presentation attack vetoed the request; capturing evidence snapshot"
        );
        let store = Arc::clone(&pipe.evidence_store);
        self.evidence_writes.spawn_blocking(move || {
            store_evidence_capture(&store, target_uid, PAD_FAILED_EVIDENCE_REASON, &frame);
        });
    }

    /// Authorizes and serves one `RequestKind::PreviewFrame` request.
    ///
    /// Order of checks (each one fails closed with zero pixel bytes on the wire):
    /// 1. `authorize_preview`: root peer, or `enabled` + allow-listed + `peer_uid == uid_hint`;
    /// 2. active logind session for unprivileged peers (same validator as Step 6b);
    /// 3. per-peer-UID rate limit (`[preview] max_requests_per_sec`), root included.
    ///
    /// Only then is the camera woken (`notify_activity`) and the latest capture copied.
    async fn handle_preview_request(
        &self,
        peer_uid: u32,
        req: &Request,
    ) -> Result<ResponseOutput, DaemonError> {
        if let Err(denied) = authorize_preview(&self.preview, peer_uid, req.uid_hint) {
            warn!(
                peer_uid = peer_uid,
                target_uid = req.uid_hint,
                reason = %denied,
                "Preview request refused by authorization policy"
            );
            let encoded = self.build_response(
                req.request_id,
                Verdict::ProtocolError,
                ReasonClass::UidMismatch,
            )?;
            return Ok(ResponseOutput {
                encoded_response: encoded,
                completion_error: None,
            });
        }

        if peer_uid != 0 && !self.session_validator.is_active_session(peer_uid) {
            warn!(
                peer_uid = peer_uid,
                "Preview peer has no active logind session; refusing preview request"
            );
            let encoded = self.build_response(
                req.request_id,
                Verdict::ProtocolError,
                ReasonClass::UidMismatch,
            )?;
            return Ok(ResponseOutput {
                encoded_response: encoded,
                completion_error: None,
            });
        }

        let now_ns = match self.now_nanos() {
            Ok(ns) => ns,
            Err(err) => {
                warn!(error = %err, "Failed to query monotonic clock; refusing preview request");
                let encoded = self.build_response(
                    req.request_id,
                    Verdict::Unavailable,
                    ReasonClass::InternalError,
                )?;
                return Ok(ResponseOutput {
                    encoded_response: encoded,
                    completion_error: Some(err),
                });
            }
        };

        {
            let mut limiter = self.preview_limiter.lock().await;
            if let Err(err) = limiter.check_and_record(peer_uid, now_ns) {
                warn!(
                    peer_uid = peer_uid,
                    error = %err,
                    "Preview request quota exceeded; refusing preview request"
                );
                let encoded = self.build_response(
                    req.request_id,
                    Verdict::ProtocolError,
                    ReasonClass::RateLimited,
                )?;
                return Ok(ResponseOutput {
                    encoded_response: encoded,
                    completion_error: None,
                });
            }
        }

        let Some(ref pipe) = self.pipeline else {
            let preview_resp = PreviewResponse {
                version: CURRENT_VERSION,
                sequence: 0,
                width: 0,
                height: 0,
                format: PREVIEW_FORMAT_EMPTY,
                timestamp_monotonic_ns: 0,
                data: Vec::new(),
            };
            let encoded = Zeroizing::new(encode_preview(&preview_resp)?);
            return Ok(ResponseOutput {
                encoded_response: encoded,
                completion_error: None,
            });
        };

        pipe.camera.notify_activity();
        if !pipe.camera.is_ready() {
            let max_wake = self
                .config
                .connection_timeout
                .saturating_sub(Duration::from_millis(100))
                .min(Duration::from_millis(1000));
            let wake_start = Instant::now();
            while !pipe.camera.is_ready() && wake_start.elapsed() < max_wake {
                tokio::time::sleep(Duration::from_millis(15)).await;
            }
        }

        let empty_preview = || PreviewResponse {
            version: CURRENT_VERSION,
            sequence: 0,
            width: 0,
            height: 0,
            format: PREVIEW_FORMAT_EMPTY,
            timestamp_monotonic_ns: 0,
            data: Vec::new(),
        };
        let preview_resp = if let Some(captured) = pipe.camera.latest_frame() {
            // Frames wider than the preview width or above the 2 MiB preview budget are
            // downscaled; unconvertible ones become an explicit empty image (GitHub #196).
            let mut image = preview_image_for_frame(&captured);
            PreviewResponse {
                version: CURRENT_VERSION,
                sequence: captured.sequence,
                width: image.width,
                height: image.height,
                format: image.format,
                timestamp_monotonic_ns: captured.timestamp_mono_ns,
                data: std::mem::take(&mut image.data),
            }
        } else {
            empty_preview()
        };

        // `PreviewResponse` zeroizes its pixel buffer on drop; the encoded copy is wrapped
        // in `Zeroizing` so both copies are erased once written to the peer. An oversize
        // payload is answered with an empty preview instead of closing the connection.
        let encoded = match encode_preview(&preview_resp) {
            Ok(bytes) => Zeroizing::new(bytes),
            Err(CodecError::MessageTooLarge { .. }) => {
                warn!(
                    peer_uid = peer_uid,
                    "Preview frame exceeds the preview message limit; sending an empty preview"
                );
                Zeroizing::new(encode_preview(&empty_preview())?)
            }
            Err(err) => return Err(err.into()),
        };
        debug!(
            peer_uid = peer_uid,
            bytes = encoded.len(),
            "Generated preview response"
        );
        Ok(ResponseOutput {
            encoded_response: encoded,
            completion_error: None,
        })
    }

    /// Encodes one `Response`, stamping it from the dispatcher's monotonic clock through
    /// [`stamp_response`] (GitHub #258).
    fn build_response(
        &self,
        request_id: RequestId,
        verdict: Verdict,
        reason_class: ReasonClass,
    ) -> Result<Zeroizing<Vec<u8>>, DaemonError> {
        let resp = stamp_response(request_id, verdict, reason_class, self.now_nanos());

        info!(
            verdict = ?resp.verdict,
            reason = ?resp.reason_class,
            "Rendered authentication response"
        );

        encode(&resp).map(Zeroizing::new).map_err(DaemonError::from)
    }

    async fn write_response(
        &self,
        stream: &mut UnixStream,
        encoded_resp: &[u8],
    ) -> Result<(), DaemonError> {
        timeout(self.config.connection_timeout, async {
            stream.write_all(encoded_resp).await?;
            stream.flush().await?;
            Ok::<(), std::io::Error>(())
        })
        .await
        .map_err(|_| {
            warn!("Response transmission timed out");
            DaemonError::Timeout
        })?
        .map_err(DaemonError::Io)?;

        debug!("Response delivered successfully");
        Ok(())
    }
}
