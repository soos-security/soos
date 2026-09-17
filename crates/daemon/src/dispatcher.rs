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
use soos_protocol::codec::encode;
use soos_protocol::types::{
    Event, EventKind, ReasonClass, Request, RequestId, RequestKind, Response, StatusResponse,
    Verdict, CURRENT_VERSION, MAX_MESSAGE_SIZE,
};

/// Connection dispatcher managing concurrent incoming client requests.
pub struct ConnectionDispatcher {
    config: DispatcherConfig,
    health: Arc<HealthState>,
    pipeline: Option<PipelineComponents>,
    semaphore: Arc<Semaphore>,
    start_time: Instant,
}

impl ConnectionDispatcher {
    /// Creates a new connection dispatcher wrapping configuration and health state.
    pub fn new(config: DispatcherConfig, health: Arc<HealthState>) -> Self {
        let semaphore = Arc::new(Semaphore::new(config.max_concurrent_connections));
        Self {
            config,
            health,
            pipeline: None,
            semaphore,
            start_time: Instant::now(),
        }
    }

    /// Creates a new connection dispatcher with an active pipeline.
    pub fn with_pipeline(
        config: DispatcherConfig,
        health: Arc<HealthState>,
        pipeline: PipelineComponents,
    ) -> Self {
        let semaphore = Arc::new(Semaphore::new(config.max_concurrent_connections));
        Self {
            config,
            health,
            pipeline: Some(pipeline),
            semaphore,
            start_time: Instant::now(),
        }
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

        timeout(
            self.config.connection_timeout,
            self.process_stream(&mut stream),
        )
        .await
        .map_err(|_| {
            warn!("Connection timed out");
            DaemonError::Timeout
        })?
    }

    async fn process_stream(&self, stream: &mut UnixStream) -> Result<(), DaemonError> {
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

        let total_capacity = declared_size
            .checked_add(4)
            .ok_or(DaemonError::OversizedPayload {
                size: usize::MAX,
                max: MAX_MESSAGE_SIZE,
            })?;
        let mut full_buffer = Vec::with_capacity(total_capacity);
        full_buffer.extend_from_slice(&len_bytes);

        let mut body_buffer = vec![0u8; declared_size];
        stream.read_exact(&mut body_buffer).await?;
        full_buffer.extend_from_slice(&body_buffer);

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
                    self.handle_request(stream, peer.uid, req).await
                } else {
                    self.handle_event(peer.uid, event).await
                }
            }
            (Some(req), None) => self.handle_request(stream, peer.uid, req).await,
            (None, Some(event)) => self.handle_event(peer.uid, event).await,
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
            info!(
                peer_uid = peer_uid,
                "Processing telemetry auth failure event"
            );
            if let Some(ref pipe) = self.pipeline {
                if pipe.evidence_store.config().enabled {
                    if let Some(frame) = pipe.camera.latest_frame() {
                        match pipe.evidence_store.store_snapshot(
                            peer_uid,
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
        stream: &mut UnixStream,
        peer_uid: u32,
        req: Request,
    ) -> Result<(), DaemonError> {
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
            let encoded_resp = encode(&status_resp)?;
            stream.write_all(&encoded_resp).await?;
            stream.flush().await?;
            debug!("Delivered diagnostic status response");
            return Ok(());
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
            return self
                .send_response(stream, req.request_id, verdict, reason_class, 0)
                .await;
        }

        // Step 7: Monotonic deadline propagation check
        let now_ns = current_monotonic_nanos();
        if req.deadline_monotonic_ns > 0 && now_ns >= req.deadline_monotonic_ns {
            warn!(
                now_ns = now_ns,
                deadline = req.deadline_monotonic_ns,
                "Request exceeded monotonic deadline before processing"
            );
            return self
                .send_response(
                    stream,
                    req.request_id,
                    Verdict::Unavailable,
                    ReasonClass::Timeout,
                    now_ns,
                )
                .await;
        }

        // Step 8: Full pipeline processing
        if let Some(ref pipe) = self.pipeline {
            let auth_start = Instant::now();

            // 8a: Verify camera readiness
            if !pipe.camera.is_ready() {
                warn!("Camera is not ready; rejecting auth request");
                return self
                    .send_response(
                        stream,
                        req.request_id,
                        Verdict::Unavailable,
                        ReasonClass::CameraUnavailable,
                        now_ns,
                    )
                    .await;
            }

            // 8b: Notify activity and grab latest frame
            pipe.camera.notify_activity();
            let frame = match pipe.camera.latest_frame() {
                Some(f) => f,
                None => {
                    warn!("No camera capture available");
                    return self
                        .send_response(
                            stream,
                            req.request_id,
                            Verdict::Unavailable,
                            ReasonClass::CameraUnavailable,
                            now_ns,
                        )
                        .await;
                }
            };

            // 8c: Validate frame age against staleness threshold
            let cur_ns = current_monotonic_nanos();
            if frame.timestamp_mono_ns > 0 && cur_ns > frame.timestamp_mono_ns {
                let age_ns = cur_ns.saturating_sub(frame.timestamp_mono_ns);
                if age_ns > MAX_FRAME_AGE_NS {
                    warn!(
                        age_ms = age_ns / 1_000_000,
                        "Latest camera capture exceeds freshness threshold"
                    );
                    return self
                        .send_response(
                            stream,
                            req.request_id,
                            Verdict::Unavailable,
                            ReasonClass::StaleFrame,
                            cur_ns,
                        )
                        .await;
                }
            }

            // 8d: Deadline check before template retrieval
            if req.deadline_monotonic_ns > 0 && cur_ns >= req.deadline_monotonic_ns {
                return self
                    .send_response(
                        stream,
                        req.request_id,
                        Verdict::Unavailable,
                        ReasonClass::Timeout,
                        cur_ns,
                    )
                    .await;
            }

            // 8e: Retrieve enrolled biometric template
            let enrolled_template = match pipe.biometric_store.get(req.uid_hint) {
                Ok(Some(tmpl)) => tmpl,
                Ok(None) => {
                    info!(
                        uid = req.uid_hint,
                        "Target UID is not enrolled; returning Unavailable"
                    );
                    return self
                        .send_response(
                            stream,
                            req.request_id,
                            Verdict::Unavailable,
                            ReasonClass::InternalError,
                            cur_ns,
                        )
                        .await;
                }
                Err(err) => {
                    warn!(
                        error = %err,
                        uid = req.uid_hint,
                        "Biometric store error retrieving template"
                    );
                    return self
                        .send_response(
                            stream,
                            req.request_id,
                            Verdict::Unavailable,
                            ReasonClass::InternalError,
                            cur_ns,
                        )
                        .await;
                }
            };

            // 8f: Deadline check before neural inference
            let cur_ns = current_monotonic_nanos();
            if req.deadline_monotonic_ns > 0 && cur_ns >= req.deadline_monotonic_ns {
                return self
                    .send_response(
                        stream,
                        req.request_id,
                        Verdict::Unavailable,
                        ReasonClass::Timeout,
                        cur_ns,
                    )
                    .await;
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
                    return self
                        .send_response(
                            stream,
                            req.request_id,
                            Verdict::Unavailable,
                            ReasonClass::ModelUnavailable,
                            cur_ns,
                        )
                        .await;
                }
                Err(err) => {
                    warn!(error = %err, "Vision pipeline processing error");
                    return self
                        .send_response(
                            stream,
                            req.request_id,
                            Verdict::Unavailable,
                            ReasonClass::InternalError,
                            cur_ns,
                        )
                        .await;
                }
            };

            // Security hardening: immediately scrub and discard raw camera capture and enrolled template
            drop(frame);
            drop(enrolled_template);

            // 8h: Decision budget (< 150ms) and deadline check
            let cur_ns = current_monotonic_nanos();
            if req.deadline_monotonic_ns > 0 && cur_ns >= req.deadline_monotonic_ns {
                return self
                    .send_response(
                        stream,
                        req.request_id,
                        Verdict::Unavailable,
                        ReasonClass::Timeout,
                        cur_ns,
                    )
                    .await;
            }
            if auth_start.elapsed() > Duration::from_millis(DECISION_BUDGET_MS) {
                warn!(
                    elapsed_ms = auth_start.elapsed().as_millis(),
                    budget_ms = DECISION_BUDGET_MS,
                    "Exceeded total decision budget"
                );
                return self
                    .send_response(
                        stream,
                        req.request_id,
                        Verdict::Unavailable,
                        ReasonClass::Timeout,
                        cur_ns,
                    )
                    .await;
            }

            // 8i: Evaluate through AuthorizationEngine with per-UID rate limiting
            let ctx =
                soos_policy::AuthContext::new(score, pad_passed, face_count, req.uid_hint, true);
            let mut engine = pipe.policy.lock().await;
            let (verdict, reason_class) = engine.evaluate_with_rate_limit(&ctx, cur_ns);

            return self
                .send_response(stream, req.request_id, verdict, reason_class, cur_ns)
                .await;
        }

        // Fail-closed fallback: never authorize authentication without an initialized pipeline
        warn!(
            request_id = ?req.request_id,
            "Rejecting authentication request: daemon pipeline is not initialized"
        );
        self.send_response(
            stream,
            req.request_id,
            Verdict::Unavailable,
            ReasonClass::InternalError,
            now_ns,
        )
        .await
    }

    async fn send_response(
        &self,
        stream: &mut UnixStream,
        request_id: RequestId,
        verdict: Verdict,
        reason_class: ReasonClass,
        now_ns: u64,
    ) -> Result<(), DaemonError> {
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

        let encoded_resp = encode(&resp)?;
        stream.write_all(&encoded_resp).await?;
        stream.flush().await?;

        debug!("Response delivered successfully");
        Ok(())
    }
}
