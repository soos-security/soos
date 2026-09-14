//! IPC Protocol v1 Types for communication between the PAM module and the soos daemon.
//!
//! All types are strictly bounded in size and validated upon construction.
//! The `request_id` is a 256-bit (32-byte) cryptographic random identifier
//! that binds a request to its response and prevents logical replay attacks.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Protocol Constants
// ---------------------------------------------------------------------------

/// Current wire protocol version.
pub const CURRENT_VERSION: u8 = 1;

/// Maximum serialized message size in bytes.
/// Any message exceeding this size is rejected BEFORE deserialization.
pub const MAX_MESSAGE_SIZE: usize = 4096;

/// Length of the `request_id` in bytes (256 bits).
pub const REQUEST_ID_LEN: usize = 32;

/// Maximum allowable length of a PAM service name in bytes.
pub const MAX_SERVICE_LEN: usize = 64;

// ---------------------------------------------------------------------------
// Request Types
// ---------------------------------------------------------------------------

/// Unique identifier for an authentication request.
/// Generated via `getrandom`, never reused, and bound to the response.
pub type RequestId = [u8; REQUEST_ID_LEN];

/// Request kind sent by PAM or diagnostic tools to the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum RequestKind {
    /// Facial authentication verification request.
    Auth = 0,
    /// Non-biometric status diagnostic query.
    Status = 1,
}

/// Telemetry event notified by the PAM module to the daemon (best-effort).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum EventKind {
    /// Password authentication attempt failed.
    /// Potentially triggers anti-intrusion evidence capture (opt-in).
    PasswordFailed = 0,
}

/// Authentication request payload sent by the PAM module.
///
/// Note: `uid_hint` is merely a consistency assertion; the authoritative
/// target UID is obtained by the daemon via kernel `SO_PEERCRED`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// Protocol version.
    pub version: u8,
    /// Request operation type.
    pub kind: RequestKind,
    /// Unique 256-bit cryptographic nonce (`getrandom`).
    pub request_id: RequestId,
    /// Declared UID from PAM client (cross-checked against `SO_PEERCRED`).
    pub uid_hint: u32,
    /// PAM service name (e.g. "gdm", "sudo", "login"). Bounded to 64 bytes.
    pub service: String,
    /// Absolute monotonic deadline in nanoseconds. Exceeding this deadline
    /// forces the daemon to return `Unavailable` immediately.
    pub deadline_monotonic_ns: u64,
}

/// Telemetry event notification following standard authentication failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// Protocol version.
    pub version: u8,
    /// Event notification kind.
    pub kind: EventKind,
    /// Associated facial authentication request ID, if applicable.
    pub request_id: Option<RequestId>,
    /// PAM service name.
    pub service: String,
    /// Monotonic timestamp in nanoseconds.
    pub timestamp_monotonic_ns: u64,
}

// ---------------------------------------------------------------------------
// Response Types
// ---------------------------------------------------------------------------

/// Verdict returned by the daemon following facial analysis.
///
/// `Deny` and `Unavailable` are intentionally indistinguishable to the PAM module
/// (both lead to `PAM_IGNORE`). The daemon separates them for internal observability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum Verdict {
    /// Single face matched, PAD validated, score >= threshold, valid context.
    Allow = 0,
    /// No face, multiple faces, low similarity score, or failed PAD anti-spoof.
    Deny = 1,
    /// Camera/model/socket offline, timeout expired, or internal error.
    Unavailable = 2,
    /// Malformed request, mismatched UID, or rate-limit reached.
    ProtocolError = 3,
}

/// Detailed classification category for internal daemon observability.
/// Must NEVER be exposed to unprivileged users or leaked into user logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum ReasonClass {
    /// Facial authentication succeeded.
    FaceMatch = 0,
    /// No face detected in frame.
    NoFace = 1,
    /// Multiple faces detected in frame.
    MultipleFaces = 2,
    /// Similarity score below acceptance threshold.
    ScoreBelowThreshold = 3,
    /// Presentation Attack Detection (PAD) rejected spoof attempt.
    PadFailed = 4,
    /// Camera hardware unavailable or uninitialized.
    CameraUnavailable = 5,
    /// ONNX model session not loaded or corrupted.
    ModelUnavailable = 6,
    /// Frame exceeds freshness budget (> 150ms).
    StaleFrame = 7,
    /// Execution deadline exceeded.
    Timeout = 8,
    /// Rate limit reached for requested UID.
    RateLimited = 9,
    /// Declared UID does not match kernel `SO_PEERCRED`.
    UidMismatch = 10,
    /// Malformed payload or unsupported protocol version.
    MalformedRequest = 11,
    /// Unclassified internal server error.
    InternalError = 12,
}

/// Daemon response sent to the PAM module.
///
/// Single-use: cryptographically bound to `request_id`, UID, service, and
/// short expiration. Never cached by the PAM module.
///
/// Manually implements `Zeroize` because enums `Verdict` and `ReasonClass`
/// do not support automatic derive. On drop, sensitive fields are zeroed out
/// and enums are reset to safe non-authorizing values (`Deny` / `InternalError`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    /// Protocol version.
    pub version: u8,
    /// Identifier matching the original authentication request.
    pub request_id: RequestId,
    /// Authentication verdict rendered by daemon.
    pub verdict: Verdict,
    /// Internal reason classification for daemon observability.
    pub reason_class: ReasonClass,
    /// Monotonic issuance timestamp (nanoseconds).
    pub issued_monotonic_ns: u64,
    /// Monotonic expiration timestamp (nanoseconds, 1-2s after issuance).
    pub expires_monotonic_ns: u64,
}

impl zeroize::Zeroize for Response {
    fn zeroize(&mut self) {
        self.version.zeroize();
        self.request_id.zeroize();
        // Zero-field enums cannot be binary zeroized; reset to safe fallback defaults
        // ensuring an erased response can never be misconstrued as authorized.
        self.verdict = Verdict::Deny;
        self.reason_class = ReasonClass::InternalError;
        self.issued_monotonic_ns.zeroize();
        self.expires_monotonic_ns.zeroize();
    }
}

impl Drop for Response {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.zeroize();
    }
}

/// Non-biometric health and readiness status response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "Component readiness booleans are atomic snapshot flags representing daemon state"
)]
pub struct StatusResponse {
    /// Protocol version.
    pub version: u8,
    /// Whether the Unix domain socket is bound and accepting connections.
    pub socket_ready: bool,
    /// Whether the camera device is initialized and capturing frames.
    pub camera_ready: bool,
    /// Whether all local ONNX models have been verified against manifest checksums.
    pub models_verified: bool,
    /// Aggregated overall health (all components must be ready).
    pub is_healthy: bool,
    /// Daemon process ID.
    pub pid: u32,
    /// Daemon uptime in seconds.
    pub uptime_secs: u64,
}

// ---------------------------------------------------------------------------
// Validation Logic
// ---------------------------------------------------------------------------

/// Validation errors encountered during message parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    /// Service name exceeds maximum permitted byte length.
    ServiceTooLong { len: usize, max: usize },
    /// Protocol version is not supported.
    UnsupportedVersion { version: u8 },
}

impl core::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ServiceTooLong { len, max } => {
                write!(f, "service name too long: {len} bytes (max {max})")
            }
            Self::UnsupportedVersion { version } => {
                write!(f, "unsupported protocol version: {version}")
            }
        }
    }
}

impl std::error::Error for ValidationError {}

impl Request {
    /// Validates the request against wire protocol constraints.
    ///
    /// # Errors
    ///
    /// Returns `ValidationError` if:
    /// - The service name exceeds [`MAX_SERVICE_LEN`] bytes
    /// - The protocol version differs from [`CURRENT_VERSION`]
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.version != CURRENT_VERSION {
            return Err(ValidationError::UnsupportedVersion {
                version: self.version,
            });
        }
        if self.service.len() > MAX_SERVICE_LEN {
            return Err(ValidationError::ServiceTooLong {
                len: self.service.len(),
                max: MAX_SERVICE_LEN,
            });
        }
        Ok(())
    }
}

impl Response {
    /// Evaluates whether this response grants authentication authorization.
    ///
    /// Only an `Allow` verdict matching [`CURRENT_VERSION`] is valid authorization.
    #[must_use]
    pub fn is_allow(&self) -> bool {
        self.version == CURRENT_VERSION && self.verdict == Verdict::Allow
    }

    /// Checks if the response corresponds to the specified request identifier.
    #[must_use]
    pub fn matches_request(&self, request_id: &RequestId) -> bool {
        self.request_id == *request_id
    }
}

impl Verdict {
    /// Returns `true` if the verdict must result in `PAM_IGNORE`.
    ///
    /// All verdicts other than `Allow` fall back to `PAM_IGNORE`.
    #[must_use]
    pub fn should_ignore(self) -> bool {
        self != Self::Allow
    }
}
