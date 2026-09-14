//! Connection dispatcher with bounded concurrency and request routing.

use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::Semaphore;
use tokio::time::timeout;
use tracing::{debug, info, warn};

use crate::config::DispatcherConfig;
use crate::error::DaemonError;
use crate::health::HealthState;
use crate::peercred::{get_peer_credentials, verify_peer_credentials};
use soos_protocol::codec::{decode, encode};
use soos_protocol::types::{
    ReasonClass, Request, RequestKind, Response, StatusResponse, Verdict, CURRENT_VERSION,
    MAX_MESSAGE_SIZE,
};
use std::time::Instant;

/// Connection dispatcher managing concurrent incoming client requests.
pub struct ConnectionDispatcher {
    config: DispatcherConfig,
    health: Arc<HealthState>,
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

        // Step 5: Decode Request
        let req: Request = match decode(&full_buffer) {
            Ok(r) => r,
            Err(err) => {
                warn!(error = %err, "Failed to decode request");
                return Err(DaemonError::Codec(err));
            }
        };

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
        let (verdict, reason_class) = match verify_peer_credentials(&peer, req.uid_hint) {
            Ok(()) => {
                let status = self.health.snapshot();
                if status.socket_ready {
                    (Verdict::Allow, ReasonClass::FaceMatch)
                } else {
                    (Verdict::Unavailable, ReasonClass::CameraUnavailable)
                }
            }
            Err(DaemonError::UidMismatch {
                peer_uid,
                requested_uid,
            }) => {
                warn!(
                    peer_uid = peer_uid,
                    target_uid = requested_uid,
                    "Peer UID mismatch detected; rejecting"
                );
                (Verdict::ProtocolError, ReasonClass::UidMismatch)
            }
            Err(e) => {
                warn!(error = %e, "Peer verification error");
                (Verdict::ProtocolError, ReasonClass::MalformedRequest)
            }
        };

        // Step 7: Build response
        let resp = Response {
            version: CURRENT_VERSION,
            request_id: req.request_id,
            verdict,
            reason_class,
            issued_monotonic_ns: 0,
            expires_monotonic_ns: 0,
        };

        info!(
            peer_uid = peer.uid,
            verdict = ?verdict,
            reason = ?reason_class,
            "Rendered authentication response"
        );

        // Step 8: Encode and transmit framed response
        let encoded_resp = encode(&resp)?;
        stream.write_all(&encoded_resp).await?;
        stream.flush().await?;

        debug!("Response delivered successfully");
        Ok(())
    }
}
