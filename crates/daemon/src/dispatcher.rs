//! Connection dispatcher with bounded concurrency and request routing.

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::Semaphore;
use tokio::time::timeout;
use tracing::{debug, info, warn};

use crate::config::DispatcherConfig;
use crate::error::DaemonError;
use crate::health::HealthState;
use crate::peercred::{get_peer_credentials, verify_peer_credentials};
use crate::pipeline::{
    current_monotonic_nanos, PipelineComponents, DECISION_BUDGET_MS, MAX_FRAME_AGE_NS,
};
use crate::session::SessionValidator;
use soos_protocol::codec::encode;
use soos_protocol::types::{
    Event, EventKind, ReasonClass, Request, RequestId, RequestKind, Response, StatusResponse,
    Verdict, CURRENT_VERSION, MAX_MESSAGE_SIZE,
};

/// Internal representation of processed connection output before socket transmission.
#[derive(Debug)]
struct ProcessedOutput {
    /// Serialized wire response frame (including 4-byte BE length prefix), if a response is expected.
    encoded_response: Option<Vec<u8>>,
    /// Deferred error to return after response transmission (e.g. wire validation error).
    completion_error: Option<DaemonError>,
}

/// Internal representation of a request response before transmission.
#[derive(Debug)]
struct ResponseOutput {
    encoded_response: Vec<u8>,
    completion_error: Option<DaemonError>,
}

/// Connection dispatcher managing concurrent incoming client requests.
pub struct ConnectionDispatcher {
    config: DispatcherConfig,
    health: Arc<HealthState>,
    pipeline: Option<PipelineComponents>,
    semaphore: Arc<Semaphore>,
    start_time: Instant,
    session_validator: SessionValidator,
    clock_fn: fn() -> Result<u64, DaemonError>,
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
        Self {
            config,
            health,
            pipeline: None,
            semaphore,
            start_time: Instant::now(),
            session_validator,
            clock_fn: current_monotonic_nanos,
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
        Self {
            config,
            health,
            pipeline: Some(pipeline),
            semaphore,
            start_time: Instant::now(),
            session_validator,
            clock_fn: current_monotonic_nanos,
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
        self.session_validator = validator;
        self
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

        // Phase 1: Request Reading & Processing Phase.
        // Bounded by connection_timeout. Strictly performs socket reading and pipeline computation,
        // producing a fully encoded in-memory response buffer (Option<Vec<u8>>).
        // Zero response bytes are written during this phase, guaranteeing that async cancellation
        // upon timeout will never leave partial response bytes on the wire.
        let output = timeout(
            self.config.connection_timeout,
            self.read_and_process(&mut stream),
        )
        .await
        .map_err(|_| {
            warn!("Connection timed out");
            DaemonError::Timeout
        })??;

        // Phase 2: Response Transmission Phase.
        // Runs outside the request processing timeout. Writes the pre-encoded response frame
        // atomically using a dedicated write timeout to prevent slow client stalls.
        if let Some(ref encoded_resp) = output.encoded_response {
            self.write_response(&mut stream, encoded_resp).await?;
        }

        if let Some(err) = output.completion_error {
            return Err(err);
        }

        Ok(())
    }

    async fn read_and_process(
        &self,
        stream: &mut UnixStream,
    ) -> Result<ProcessedOutput, DaemonError> {
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
                    let res = self.handle_request(peer.uid, req).await?;
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
                let res = self.handle_request(peer.uid, req).await?;
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
        req: Request,
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
            let encoded = encode(&status_resp)?;
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
            pid: None,
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

        // Step 6b: Verify active logind session
        if req.kind == RequestKind::Auth && !self.session_validator.is_active_session(req.uid_hint)
        {
            warn!(
                uid = req.uid_hint,
                "Target UID has no active logind session; rejecting auth request"
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
            let auth_start = Instant::now();

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

            // 8a: Verify camera readiness
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

            // 8b: Notify activity and grab latest frame
            pipe.camera.notify_activity();
            let frame = match pipe.camera.latest_frame() {
                Some(f) => f,
                None => {
                    warn!("No camera capture available");
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
            };

            // 8c: Validate frame age against staleness threshold
            let cur_ns = match self.now_nanos() {
                Ok(ns) => ns,
                Err(err) => {
                    warn!(error = %err, "Monotonic clock query failed checking capture freshness");
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
            if frame.timestamp_mono_ns > 0 && cur_ns > frame.timestamp_mono_ns {
                let age_ns = cur_ns.saturating_sub(frame.timestamp_mono_ns);
                if age_ns > MAX_FRAME_AGE_NS {
                    warn!(
                        age_ms = age_ns / 1_000_000,
                        "Latest camera capture exceeds freshness threshold"
                    );
                    let encoded = self.build_response(
                        req.request_id,
                        Verdict::Unavailable,
                        ReasonClass::StaleFrame,
                        cur_ns,
                    )?;
                    return Ok(ResponseOutput {
                        encoded_response: encoded,
                        completion_error: None,
                    });
                }
            }

            // 8d: Deadline check before template retrieval
            if req.deadline_monotonic_ns > 0 && cur_ns >= req.deadline_monotonic_ns {
                let encoded = self.build_response(
                    req.request_id,
                    Verdict::Unavailable,
                    ReasonClass::Timeout,
                    cur_ns,
                )?;
                return Ok(ResponseOutput {
                    encoded_response: encoded,
                    completion_error: None,
                });
            }

            // 8e: Retrieve enrolled biometric template
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
                        cur_ns,
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
                        cur_ns,
                    )?;
                    return Ok(ResponseOutput {
                        encoded_response: encoded,
                        completion_error: None,
                    });
                }
            };

            // 8f: Deadline check before neural inference
            let cur_ns = match self.now_nanos() {
                Ok(ns) => ns,
                Err(err) => {
                    warn!(error = %err, "Monotonic clock query failed checking inference deadline");
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
            if req.deadline_monotonic_ns > 0 && cur_ns >= req.deadline_monotonic_ns {
                let encoded = self.build_response(
                    req.request_id,
                    Verdict::Unavailable,
                    ReasonClass::Timeout,
                    cur_ns,
                )?;
                return Ok(ResponseOutput {
                    encoded_response: encoded,
                    completion_error: None,
                });
            }

            // 8g: Execute neural vision verification pipeline
            let (score, face_count, pad_passed) = match pipe.vision.process_frame(&frame) {
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
                    (sim, 1u8, output.pad_result.is_live)
                }
                Err(soos_vision::VisionError::PadFailed { score, threshold }) => {
                    debug!(
                        score = score,
                        threshold = threshold,
                        "Presentation attack detected (PAD failed)"
                    );
                    (0.0, 1u8, false)
                }
                Err(soos_vision::VisionError::NoFaceDetected) => {
                    debug!("Zero faces detected in capture");
                    (0.0, 0u8, false)
                }
                Err(soos_vision::VisionError::MultipleFacesDetected { count }) => {
                    let count_u8 = u8::try_from(count).unwrap_or(u8::MAX);
                    debug!(count = count, "Multiple faces detected in capture");
                    (0.0, count_u8, false)
                }
                Err(soos_vision::VisionError::FaceBelowConfidence { .. }) => {
                    debug!("Face detected below confidence threshold");
                    (0.0, 0u8, false)
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

            // Security hardening: immediately scrub and discard raw camera capture and enrolled template
            drop(frame);
            drop(enrolled_template);

            // 8h: Decision budget (< 150ms) and deadline check
            let cur_ns = match self.now_nanos() {
                Ok(ns) => ns,
                Err(err) => {
                    warn!(error = %err, "Monotonic clock query failed checking decision deadline");
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
            if req.deadline_monotonic_ns > 0 && cur_ns >= req.deadline_monotonic_ns {
                let encoded = self.build_response(
                    req.request_id,
                    Verdict::Unavailable,
                    ReasonClass::Timeout,
                    cur_ns,
                )?;
                return Ok(ResponseOutput {
                    encoded_response: encoded,
                    completion_error: None,
                });
            }
            if auth_start.elapsed() > Duration::from_millis(DECISION_BUDGET_MS) {
                warn!(
                    elapsed_ms = auth_start.elapsed().as_millis(),
                    budget_ms = DECISION_BUDGET_MS,
                    "Exceeded total decision budget"
                );
                let encoded = self.build_response(
                    req.request_id,
                    Verdict::Unavailable,
                    ReasonClass::Timeout,
                    cur_ns,
                )?;
                return Ok(ResponseOutput {
                    encoded_response: encoded,
                    completion_error: None,
                });
            }

            // 8i: Evaluate through AuthorizationEngine with per-UID rate limiting
            let ctx =
                soos_policy::AuthContext::new(score, pad_passed, face_count, req.uid_hint, true);
            let mut engine = pipe.policy.write().await;
            let (verdict, reason_class) = engine.evaluate_with_rate_limit(&ctx, cur_ns);

            let encoded = self.build_response(req.request_id, verdict, reason_class, cur_ns)?;
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

    fn build_response(
        &self,
        request_id: RequestId,
        verdict: Verdict,
        reason_class: ReasonClass,
        now_ns: u64,
    ) -> Result<Vec<u8>, DaemonError> {
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

        encode(&resp).map_err(DaemonError::from)
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
