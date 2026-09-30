//! Connection dispatcher with bounded concurrency and request routing.

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::Semaphore;
use tokio::time::timeout;
use tracing::{debug, info, warn};
use zeroize::Zeroizing;

use crate::config::DispatcherConfig;
use crate::error::DaemonError;
use crate::health::HealthState;
use crate::inference::{InferenceGate, RequestDeadline};
use crate::peercred::{get_peer_credentials, verify_peer_credentials};
use crate::pipeline::{
    current_monotonic_nanos, PipelineComponents, FRAME_POLL_INTERVAL_MS, MAX_FRAME_AGE_NS,
};
use crate::preview::{authorize_preview, PreviewConfig};
use crate::session::SessionValidator;
use crate::session_policy::LocalSessionPolicy;
use soos_policy::{ConsensusDecision, FrameEvaluation, PadAggregator, RateLimiter};
use soos_protocol::codec::{encode, encode_preview};
use soos_protocol::types::{
    Event, EventKind, PreviewResponse, ReasonClass, Request, RequestId, RequestKind, Response,
    StatusResponse, Verdict, CURRENT_VERSION, MAX_MESSAGE_SIZE,
};

/// Internal representation of processed connection output before socket transmission.
#[derive(Debug)]
struct ProcessedOutput {
    /// Serialized wire response (including 4-byte BE length prefix), if a response is expected.
    /// Wrapped in `Zeroizing` because preview responses carry camera pixel data.
    encoded_response: Option<Zeroizing<Vec<u8>>>,
    /// Deferred error to return after response transmission (e.g. wire validation error).
    completion_error: Option<DaemonError>,
}

/// Internal representation of a request response before transmission.
#[derive(Debug)]
struct ResponseOutput {
    encoded_response: Zeroizing<Vec<u8>>,
    completion_error: Option<DaemonError>,
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
    semaphore: Arc<Semaphore>,
    start_time: Instant,
    session_validator: SessionValidator,
    session_policy: LocalSessionPolicy,
    clock_fn: fn() -> Result<u64, DaemonError>,
    preview: PreviewConfig,
    preview_limiter: tokio::sync::Mutex<RateLimiter>,
    inference: InferenceGate,
}

impl ConnectionDispatcher {
    /// Creates a new connection dispatcher wrapping configuration and health state.
    pub fn new(config: DispatcherConfig, health: Arc<HealthState>) -> Self {
        let semaphore = Arc::new(Semaphore::new(config.max_concurrent_connections));
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
            semaphore,
            start_time: Instant::now(),
            session_validator,
            session_policy,
            clock_fn: current_monotonic_nanos,
            preview,
            preview_limiter,
            inference: InferenceGate::default(),
        }
    }

    /// Creates a new connection dispatcher with an active pipeline.
    pub fn with_pipeline(
        config: DispatcherConfig,
        health: Arc<HealthState>,
        pipeline: PipelineComponents,
    ) -> Self {
        let semaphore = Arc::new(Semaphore::new(config.max_concurrent_connections));
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
            semaphore,
            start_time: Instant::now(),
            session_validator,
            session_policy,
            clock_fn: current_monotonic_nanos,
            preview,
            preview_limiter,
            inference: InferenceGate::default(),
        }
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

    /// Returns the active preview authorization policy.
    #[must_use]
    pub const fn preview_config(&self) -> &PreviewConfig {
        &self.preview
    }

    fn now_nanos(&self) -> Result<u64, DaemonError> {
        (self.clock_fn)()
    }

    /// Returns the number of currently available concurrency permits.
    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }

    /// Processes an incoming Unix domain stream through authentication,
    /// bounded reading, peer verification, and response generation.
    pub async fn handle_connection(&self, mut stream: UnixStream) -> Result<(), DaemonError> {
        let _permit = match self.semaphore.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                warn!("Concurrency limit reached; rejecting connection");
                return Err(DaemonError::ConcurrencyLimitReached);
            }
        };

        let mut requests_processed: usize = 0;
        loop {
            // Phase 1: Request Reading & Processing Phase.
            // Bounded by connection_timeout. Strictly performs socket reading and pipeline computation,
            // producing a fully encoded in-memory response buffer (Option<Vec<u8>>).
            // Zero response bytes are written during this phase, guaranteeing that async cancellation
            // upon timeout will never leave partial response bytes on the wire.
            let res = timeout(
                self.config.connection_timeout,
                self.read_and_process(&mut stream),
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
        }

        Ok(())
    }

    async fn read_and_process(
        &self,
        stream: &mut UnixStream,
    ) -> Result<ProcessedOutput, DaemonError> {
        // Start of the outer `connection_timeout` window for this request: every later budget
        // (camera wake, consensus loop, inference admission) is measured from here (#159).
        let request_started = Instant::now();

        // Step 1: Extract peer credentials via SO_PEERCRED
        let peer = get_peer_credentials(stream)?;

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

        // Step 4: Decode message — may be Request or Event
        // Uses exact deserialization without unconsumed trailing bytes to reliably differentiate schemas
        let req_opt = postcard::take_from_bytes::<Request>(&body_buffer)
            .ok()
            .and_then(|(r, rest)| if rest.is_empty() { Some(r) } else { None });

        let event_opt = postcard::take_from_bytes::<Event>(&body_buffer)
            .ok()
            .and_then(|(e, rest)| if rest.is_empty() { Some(e) } else { None });

        match (req_opt, event_opt) {
            (Some(req), Some(event)) => {
                // Disambiguate when wire payload matches both Request and Event schemas.
                // An Event decoded as Request will have `req.uid_hint` equal to the 32nd byte
                // of `event.request_id`. If `req.uid_hint == peer.uid`, it is a valid Request.
                // Otherwise, it is an Event notification.
                if req.uid_hint == peer.uid {
                    let res = self.handle_request(peer.uid, peer.pid, req, request_started).await?;
                    Ok(ProcessedOutput {
                        encoded_response: Some(res.encoded_response),
                        completion_error: res.completion_error,
                    })
                } else {
                    self.handle_event(peer.uid, event).await?;
                    Ok(ProcessedOutput {
                        encoded_response: None,
                        completion_error: None,
                    })
                }
            }
            (Some(req), None) => {
                let res = self.handle_request(peer.uid, peer.pid, req, request_started).await?;
                Ok(ProcessedOutput {
                    encoded_response: Some(res.encoded_response),
                    completion_error: res.completion_error,
                })
            }
            (None, Some(event)) => {
                self.handle_event(peer.uid, event).await?;
                Ok(ProcessedOutput {
                    encoded_response: None,
                    completion_error: None,
                })
            }
            (None, None) => {
                warn!("Failed to decode payload as Request or Event");
                Err(DaemonError::Protocol("Malformed wire payload".into()))
            }
        }
    }

    async fn handle_event(&self, peer_uid: u32, event: Event) -> Result<(), DaemonError> {
        if event.version != CURRENT_VERSION {
            warn!(version = event.version, "Unsupported event version");
            return Err(DaemonError::Protocol("Unsupported event version".into()));
        }

        if event.kind == EventKind::PasswordFailed {
            let target_uid = event.uid.unwrap_or(peer_uid);
            info!(
                peer_uid = peer_uid,
                target_uid = target_uid,
                "Processing telemetry auth failure event"
            );
            if let Some(ref pipe) = self.pipeline {
                if pipe.evidence_store.config().enabled {
                    if let Some(frame) = pipe.camera.latest_frame() {
                        match pipe.evidence_store.store_snapshot(
                            target_uid,
                            "PasswordFailed",
                            &frame.data,
                            None,
                            None,
                        ) {
                            Ok(snap_res) => {
                                debug!("Intrusion evidence snapshot stored successfully");
                                let _ = pipe.evidence_store.rotate_retention(&snap_res.date);
                            }
                            Err(err) => {
                                warn!(error = %err, "Failed to store intrusion evidence snapshot");
                            }
                        }
                    } else {
                        warn!("No camera capture available for evidence snapshot");
                    }
                }
            }
        }

        Ok(())
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
                0,
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
            let encoded = self.build_response(req.request_id, verdict, reason_class, 0)?;
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
                0,
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
                    0,
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
            let encoded = self.build_response(
                req.request_id,
                Verdict::Unavailable,
                ReasonClass::Timeout,
                now_ns,
            )?;
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

            // 8-pre: Fail-fast rate limiting check (anti-DoS: avoid neural & camera workload if already blocked)
            {
                let engine = pipe.policy.read().await;
                if let Err(err) = engine.check_allowed(req.uid_hint, now_ns) {
                    warn!(
                        uid = req.uid_hint,
                        error = %err,
                        "UID has exceeded rate limit quota; rejecting request immediately"
                    );
                    let encoded = self.build_response(
                        req.request_id,
                        Verdict::ProtocolError,
                        ReasonClass::RateLimited,
                        now_ns,
                    )?;
                    return Ok(ResponseOutput {
                        encoded_response: encoded,
                        completion_error: None,
                    });
                }
            }

            // 8a: Notify activity to wake camera from auto-standby
            pipe.camera.notify_activity();

            // 8b: If camera is resuming from auto-standby, wait up to 1000-1200ms for it to become ready
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
                    now_ns,
                )?;
                return Ok(ResponseOutput {
                    encoded_response: encoded,
                    completion_error: None,
                });
            }

            // 8c: Retrieve enrolled biometric template once
            let enrolled_template = match pipe.biometric_store.get(req.uid_hint) {
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
                        now_ns,
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
                        now_ns,
                    )?;
                    return Ok(ResponseOutput {
                        encoded_response: encoded,
                        completion_error: None,
                    });
                }
            };

            // 8e: Multi-frame PAD consensus loop (GitHub #147 / PAD-02).
            // Allow requires k consecutive passing captures (live at or above the PAD
            // threshold and matching at or above the cosine threshold) inside a bounded
            // window; any spoof-classified capture vetoes the whole request (fail closed).
            let consensus_thresholds = *pipe.policy.read().await.thresholds();
            let mut aggregator = PadAggregator::with_defaults(consensus_thresholds);
            let mut last_sequence: Option<u64> = None;
            let mut last_capture_stale = false;

            loop {
                let cur_ns = match self.now_nanos() {
                    Ok(ns) => ns,
                    Err(err) => {
                        warn!(error = %err, "Monotonic clock query failed checking loop deadline");
                        let encoded = self.build_response(
                            req.request_id,
                            Verdict::Unavailable,
                            ReasonClass::InternalError,
                            0,
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
                                        cur_ns,
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
                                Err(soos_vision::VisionError::Inference(err)) => {
                                    warn!(error = %err, "Vision neural inference failure");
                                    let encoded = self.build_response(
                                        req.request_id,
                                        Verdict::Unavailable,
                                        ReasonClass::ModelUnavailable,
                                        cur_ns,
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
                                        cur_ns,
                                    )?;
                                    return Ok(ResponseOutput {
                                        encoded_response: encoded,
                                        completion_error: None,
                                    });
                                }
                            };

                            let class = aggregator.record(&evaluation);
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

            // 8f: Record exactly one attempt per request, then render the aggregate verdict.
            // A rate-limit rejection at this point downgrades Allow (fail closed).
            let cur_ns = self.now_nanos().unwrap_or(0);
            let rate_limited = {
                let mut engine_write = pipe.policy.write().await;
                engine_write.record_attempt(req.uid_hint, cur_ns).is_err()
            };

            let decision = aggregator.decision();
            let (final_verdict, final_reason) = match decision {
                ConsensusDecision::Allow if rate_limited => {
                    warn!(
                        uid = req.uid_hint,
                        "Rate limit reached while recording attempt; withholding authorization"
                    );
                    (Verdict::ProtocolError, ReasonClass::RateLimited)
                }
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

            if req.service.contains("gdm")
                || req.service.contains("lock")
                || req.service.contains("screen")
            {
                pipe.camera.notify_activity();
            }

            let encoded =
                self.build_response(req.request_id, final_verdict, final_reason, cur_ns)?;
            return Ok(ResponseOutput {
                encoded_response: encoded,
                completion_error: None,
            });
        }

        // Fail-closed fallback: never authorize authentication without an initialized pipeline
        warn!(
            request_id = ?req.request_id,
            "Rejecting authentication request: daemon pipeline is not initialized"
        );
        let encoded = self.build_response(
            req.request_id,
            Verdict::Unavailable,
            ReasonClass::InternalError,
            now_ns,
        )?;
        Ok(ResponseOutput {
            encoded_response: encoded,
            completion_error: None,
        })
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
                0,
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
                0,
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
                    0,
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
                    now_ns,
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
                format: 255,
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

        let preview_resp = if let Some(captured) = pipe.camera.latest_frame() {
            let fmt_u8 = match captured.format {
                soos_camera_v4l::PixelFormat::Rgb24 => 0,
                soos_camera_v4l::PixelFormat::Grey => 1,
                soos_camera_v4l::PixelFormat::Yuyv => 2,
                soos_camera_v4l::PixelFormat::Nv12 => 3,
                soos_camera_v4l::PixelFormat::Mjpeg => 4,
            };
            PreviewResponse {
                version: CURRENT_VERSION,
                sequence: captured.sequence,
                width: captured.width,
                height: captured.height,
                format: fmt_u8,
                timestamp_monotonic_ns: captured.timestamp_mono_ns,
                data: captured.data.clone(),
            }
        } else {
            PreviewResponse {
                version: CURRENT_VERSION,
                sequence: 0,
                width: 0,
                height: 0,
                format: 255,
                timestamp_monotonic_ns: 0,
                data: Vec::new(),
            }
        };

        // `PreviewResponse` zeroizes its pixel buffer on drop; the encoded copy is wrapped
        // in `Zeroizing` so both copies are erased once written to the peer.
        let encoded = Zeroizing::new(encode_preview(&preview_resp)?);
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

    fn build_response(
        &self,
        request_id: RequestId,
        verdict: Verdict,
        reason_class: ReasonClass,
        now_ns: u64,
    ) -> Result<Zeroizing<Vec<u8>>, DaemonError> {
        let resp = Response {
            version: CURRENT_VERSION,
            request_id,
            verdict,
            reason_class,
            issued_monotonic_ns: now_ns,
            expires_monotonic_ns: now_ns.saturating_add(2_000_000_000),
        };

        info!(
            verdict = ?verdict,
            reason = ?reason_class,
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
