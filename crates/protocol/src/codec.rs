//! Bounded binary codec for IPC Protocol v1.
//!
//! Employs `postcard` (compact binary serde) for payload serialization.
//! Every message is framed with a 4-byte `u32` Big-Endian length prefix
//! for streaming framing over `SOCK_STREAM`.
//!
//! # Security Invariants
//!
//! - Declared message size is verified BEFORE any buffer allocation or deserialization.
//! - Oversized messages are rejected with zero allocation.
//! - The codec performs zero direct I/O: operations strictly process `&[u8]` slices.
//! - Decoding is strict: the declared payload must be consumed exactly; unconsumed bytes
//!   inside the declared length are rejected with [`CodecError::TrailingBytes`] (GitHub #224).
//!   The single tolerated remainder is the one-byte client message tag trailer of the decoded
//!   type (GitHub #204): a [`Request`] may be followed by exactly
//!   [`MESSAGE_TAG_REQUEST`](crate::message::MESSAGE_TAG_REQUEST) and an [`Event`] by exactly
//!   [`MESSAGE_TAG_EVENT`](crate::message::MESSAGE_TAG_EVENT); any other remainder, and any
//!   remainder after any other type, is rejected. The daemon classifies client payloads with
//!   [`crate::message::decode_client_message`], which strips the tag and decodes the body
//!   through the zero-remainder [`decode_payload_exact`].
//! - Encoding sizes the frame first (`postcard::ser_flavors::Size`) and serializes in place
//!   into the single returned buffer: no intermediate allocation holding the request nonce
//!   is ever freed without zeroization, and oversized messages are rejected before the
//!   frame is allocated (GitHub #225).

use crate::message::{MESSAGE_TAG_EVENT, MESSAGE_TAG_REQUEST};
use crate::types::{Event, Request, MAX_MESSAGE_SIZE, MAX_PREVIEW_MESSAGE_SIZE};
use core::any::TypeId;
use serde::{de::DeserializeOwned, Serialize};
use zeroize::Zeroize;

// ---------------------------------------------------------------------------
// Codec Errors
// ---------------------------------------------------------------------------

/// Protocol serialization and deserialization errors.
#[derive(Debug)]
pub enum CodecError {
    /// Serialized payload exceeds the maximum allowed byte boundary.
    MessageTooLarge { size: usize, max: usize },
    /// Postcard serialization failure.
    Serialize(postcard::Error),
    /// Postcard deserialization failure.
    Deserialize(postcard::Error),
    /// Supplied buffer is too small to contain the 4-byte length prefix.
    BufferTooSmall,
    /// Length prefix claims a size exceeding the maximum message size.
    DeclaredSizeTooLarge { declared: usize, max: usize },
    /// The payload decoded successfully but left `unconsumed` bytes inside the declared
    /// length. Rejected so that a payload has exactly one interpretation (GitHub #224).
    TrailingBytes { unconsumed: usize },
}

impl core::fmt::Display for CodecError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::MessageTooLarge { size, max } => {
                write!(f, "message too large: {size} bytes (max {max})")
            }
            Self::Serialize(e) => write!(f, "serialization error: {e}"),
            Self::Deserialize(e) => write!(f, "deserialization error: {e}"),
            Self::BufferTooSmall => write!(f, "buffer too small for size prefix"),
            Self::DeclaredSizeTooLarge { declared, max } => {
                write!(
                    f,
                    "declared message size too large: {declared} bytes (max {max})"
                )
            }
            Self::TrailingBytes { unconsumed } => {
                write!(f, "payload has {unconsumed} unconsumed trailing bytes")
            }
        }
    }
}

impl std::error::Error for CodecError {}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

/// Serializes a message into a byte vector with a Big-Endian `u32` length prefix,
/// strictly bounded by [`MAX_MESSAGE_SIZE`] (4,096 bytes).
///
/// # Errors
///
/// Returns [`CodecError::Serialize`] if serialization fails, or
/// [`CodecError::MessageTooLarge`] if the serialized payload exceeds [`MAX_MESSAGE_SIZE`].
pub fn encode<T: Serialize>(msg: &T) -> Result<Vec<u8>, CodecError> {
    encode_with_limit(msg, MAX_MESSAGE_SIZE)
}

/// Serializes a video preview message into a byte vector with a Big-Endian `u32` length prefix,
/// bounded by [`MAX_PREVIEW_MESSAGE_SIZE`] (2 MiB).
///
/// # Errors
///
/// Returns [`CodecError::Serialize`] if serialization fails, or
/// [`CodecError::MessageTooLarge`] if the serialized payload exceeds [`MAX_PREVIEW_MESSAGE_SIZE`].
pub fn encode_preview<T: Serialize>(msg: &T) -> Result<Vec<u8>, CodecError> {
    encode_with_limit(msg, MAX_PREVIEW_MESSAGE_SIZE)
}

/// Serializes a message with a custom maximum size limit.
///
/// # Errors
///
/// Returns [`CodecError::Serialize`] if serialization fails, or
/// [`CodecError::MessageTooLarge`] if the serialized payload exceeds `max_size`.
pub fn encode_with_limit<T: Serialize>(msg: &T, max_size: usize) -> Result<Vec<u8>, CodecError> {
    // Size first so the oversize check happens before any allocation, and the frame is
    // allocated exactly once at its final size (no reallocation leaving stale copies).
    let payload_len = postcard::serialize_with_flavor(msg, postcard::ser_flavors::Size::default())
        .map_err(CodecError::Serialize)?;

    if payload_len > max_size {
        return Err(CodecError::MessageTooLarge {
            size: payload_len,
            max: max_size,
        });
    }

    let size_prefix = u32::try_from(payload_len)
        .map_err(|_| CodecError::MessageTooLarge {
            size: payload_len,
            max: max_size,
        })?
        .to_be_bytes();

    let total_len = payload_len
        .checked_add(4)
        .ok_or(CodecError::MessageTooLarge {
            size: usize::MAX,
            max: max_size,
        })?;

    let mut buf = vec![0u8; total_len];
    let written = match buf.split_at_mut_checked(4) {
        Some((prefix, body)) => {
            prefix.copy_from_slice(&size_prefix);
            postcard::to_slice(msg, body).map(|used| used.len())
        }
        None => Err(postcard::Error::SerializeBufferFull),
    };

    match written {
        Ok(len) if len == payload_len => Ok(buf),
        Ok(_) => {
            // A non-deterministic `Serialize` impl produced a different size: fail closed.
            buf.zeroize();
            Err(CodecError::Serialize(postcard::Error::SerializeBufferFull))
        }
        Err(e) => {
            buf.zeroize();
            Err(CodecError::Serialize(e))
        }
    }
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

/// Deserializes a message from a byte slice framed by a Big-Endian `u32` length prefix,
/// strictly bounded by [`MAX_MESSAGE_SIZE`] (4,096 bytes).
///
/// # Errors
///
/// Returns [`CodecError::BufferTooSmall`] if `buf` is too short,
/// [`CodecError::DeclaredSizeTooLarge`] if declared size exceeds [`MAX_MESSAGE_SIZE`],
/// [`CodecError::Deserialize`] if deserialization fails, or [`CodecError::TrailingBytes`] if
/// bytes remain unconsumed inside the declared payload.
pub fn decode<T: DeserializeOwned + 'static>(buf: &[u8]) -> Result<T, CodecError> {
    decode_with_limit(buf, MAX_MESSAGE_SIZE)
}

/// Deserializes a video preview message from a byte slice framed by a Big-Endian `u32` length prefix,
/// bounded by [`MAX_PREVIEW_MESSAGE_SIZE`] (2 MiB).
///
/// # Errors
///
/// Returns [`CodecError::BufferTooSmall`] if `buf` is too short,
/// [`CodecError::DeclaredSizeTooLarge`] if declared size exceeds [`MAX_PREVIEW_MESSAGE_SIZE`],
/// [`CodecError::Deserialize`] if deserialization fails, or [`CodecError::TrailingBytes`] if
/// bytes remain unconsumed inside the declared payload.
pub fn decode_preview<T: DeserializeOwned + 'static>(buf: &[u8]) -> Result<T, CodecError> {
    decode_with_limit(buf, MAX_PREVIEW_MESSAGE_SIZE)
}

/// Deserializes a message with a custom maximum size limit.
///
/// # Errors
///
/// Returns [`CodecError::BufferTooSmall`] if `buf` is too short,
/// [`CodecError::DeclaredSizeTooLarge`] if declared size exceeds `max_size`,
/// [`CodecError::Deserialize`] if deserialization fails, or [`CodecError::TrailingBytes`] if
/// bytes remain unconsumed inside the declared payload.
pub fn decode_with_limit<T: DeserializeOwned + 'static>(
    buf: &[u8],
    max_size: usize,
) -> Result<T, CodecError> {
    let size_slice = buf.get(..4).ok_or(CodecError::BufferTooSmall)?;
    let size_bytes: [u8; 4] = size_slice
        .try_into()
        .map_err(|_| CodecError::BufferTooSmall)?;
    let declared_size =
        usize::try_from(u32::from_be_bytes(size_bytes)).map_err(|_| CodecError::BufferTooSmall)?;

    if declared_size > max_size {
        return Err(CodecError::DeclaredSizeTooLarge {
            declared: declared_size,
            max: max_size,
        });
    }

    let payload_end = declared_size
        .checked_add(4)
        .ok_or(CodecError::BufferTooSmall)?;
    if buf.len() < payload_end {
        return Err(CodecError::BufferTooSmall);
    }

    let payload_slice = buf.get(4..payload_end).ok_or(CodecError::BufferTooSmall)?;
    decode_payload(payload_slice)
}

/// Strictly deserializes an unframed payload (the bytes after the 4-byte length prefix).
///
/// The payload must be consumed exactly: this is the single decoder shared by the PAM
/// client (through [`decode`]) and the daemon-side mock servers (GitHub #224). The only
/// tolerated remainder is ONE byte equal to the client message tag of `T` (GitHub #204):
/// [`MESSAGE_TAG_REQUEST`] after a [`Request`], [`MESSAGE_TAG_EVENT`] after an [`Event`].
/// Every other remainder (another byte, the other type's tag, two or more bytes, or any
/// byte after a type that is never tagged, such as `Response`) is rejected.
///
/// # Errors
///
/// Returns [`CodecError::Deserialize`] if deserialization fails, or
/// [`CodecError::TrailingBytes`] if bytes remain after the decoded message other than the
/// single matching message tag.
pub fn decode_payload<T: DeserializeOwned + 'static>(payload: &[u8]) -> Result<T, CodecError> {
    let (msg, rest) = postcard::take_from_bytes(payload).map_err(CodecError::Deserialize)?;
    match rest {
        [] => Ok(msg),
        [tag] if Some(*tag) == client_message_tag::<T>() => Ok(msg),
        _ => Err(CodecError::TrailingBytes {
            unconsumed: rest.len(),
        }),
    }
}

/// Deserializes an unframed payload that must be consumed exactly, with no tag tolerance.
///
/// Used by [`crate::message::decode_client_message`] on a body whose tag was already
/// stripped, so a doubled tag is never accepted.
///
/// # Errors
///
/// Returns [`CodecError::Deserialize`] if deserialization fails, or
/// [`CodecError::TrailingBytes`] if any byte remains after the decoded message.
pub(crate) fn decode_payload_exact<T: DeserializeOwned>(payload: &[u8]) -> Result<T, CodecError> {
    let (msg, rest) = postcard::take_from_bytes(payload).map_err(CodecError::Deserialize)?;
    if rest.is_empty() {
        Ok(msg)
    } else {
        Err(CodecError::TrailingBytes {
            unconsumed: rest.len(),
        })
    }
}

/// Client message tag trailer that may follow a `T` payload, if `T` is a client message.
fn client_message_tag<T: 'static>() -> Option<u8> {
    let id = TypeId::of::<T>();
    if id == TypeId::of::<Request>() {
        Some(MESSAGE_TAG_REQUEST)
    } else if id == TypeId::of::<Event>() {
        Some(MESSAGE_TAG_EVENT)
    } else {
        None
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Unit tests use unwrap, expect, panic, and slicing for test assertions"
)]
mod tests {
    use super::*;
    use crate::types::*;

    /// Test helper creating a standard valid authentication request.
    fn test_request() -> Request {
        Request {
            version: CURRENT_VERSION,
            kind: RequestKind::Auth,
            request_id: [0xAB; REQUEST_ID_LEN],
            uid_hint: 1000,
            service: "sudo".to_string(),
            deadline_monotonic_ns: 250_000_000,
        }
    }

    /// Test helper creating a standard valid response with designated verdict.
    fn test_response(verdict: Verdict) -> Response {
        Response {
            version: CURRENT_VERSION,
            request_id: [0xAB; REQUEST_ID_LEN],
            verdict,
            reason_class: ReasonClass::FaceMatch,
            issued_monotonic_ns: 100_000_000,
            expires_monotonic_ns: 102_000_000_000,
        }
    }

    // --- P1: Serialization / Deserialization Round-Trip ---

    #[test]
    fn request_roundtrip() {
        let req = test_request();
        let encoded = encode(&req).expect("encode should succeed");
        let decoded: Request = decode(&encoded).expect("decode should succeed");

        assert_eq!(decoded.version, req.version);
        assert_eq!(decoded.kind, req.kind);
        assert_eq!(decoded.request_id, req.request_id);
        assert_eq!(decoded.uid_hint, req.uid_hint);
        assert_eq!(decoded.service, req.service);
        assert_eq!(decoded.deadline_monotonic_ns, req.deadline_monotonic_ns);
    }

    #[test]
    fn response_roundtrip() {
        let resp = test_response(Verdict::Allow);
        let encoded = encode(&resp).expect("encode should succeed");
        let decoded: Response = decode(&encoded).expect("decode should succeed");

        assert_eq!(decoded.version, resp.version);
        assert_eq!(decoded.request_id, resp.request_id);
        assert_eq!(decoded.verdict, resp.verdict);
        assert_eq!(decoded.reason_class, resp.reason_class);
        assert_eq!(decoded.issued_monotonic_ns, resp.issued_monotonic_ns);
        assert_eq!(decoded.expires_monotonic_ns, resp.expires_monotonic_ns);
    }

    #[test]
    fn event_roundtrip() {
        let evt = Event {
            version: CURRENT_VERSION,
            kind: EventKind::PasswordFailed,
            request_id: Some([0xCD; REQUEST_ID_LEN]),
            uid: Some(1000),
            service: "gdm".to_string(),
            timestamp_monotonic_ns: 500_000_000,
        };
        let encoded = encode(&evt).expect("encode should succeed");
        let decoded: Event = decode(&encoded).expect("decode should succeed");

        assert_eq!(decoded.version, evt.version);
        assert_eq!(decoded.kind, evt.kind);
        assert_eq!(decoded.request_id, evt.request_id);
        assert_eq!(decoded.uid, evt.uid);
        assert_eq!(decoded.service, evt.service);
    }

    // --- P2: Oversized Message Rejection ---

    #[test]
    fn rejects_oversized_service_name() {
        let req = Request {
            version: CURRENT_VERSION,
            kind: RequestKind::Auth,
            request_id: [0; REQUEST_ID_LEN],
            uid_hint: 1000,
            service: "a".repeat(MAX_SERVICE_LEN + 1),
            deadline_monotonic_ns: 0,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn rejects_declared_size_too_large() {
        let mut buf = vec![0u8; 8];
        #[allow(
            clippy::cast_possible_truncation,
            reason = "Test boundary deliberately exceeds maximum message size"
        )]
        let fake_size: u32 = (MAX_MESSAGE_SIZE as u32) + 1;
        buf[..4].copy_from_slice(&fake_size.to_be_bytes());

        let result: Result<Request, _> = decode(&buf);
        assert!(result.is_err());
    }

    // --- P3: RequestId is 256 Bits ---

    #[test]
    fn request_id_is_256_bits() {
        assert_eq!(REQUEST_ID_LEN, 32);
        let req = test_request();
        assert_eq!(req.request_id.len(), 32);
    }

    // --- P4: Versioned Protocol ---

    #[test]
    fn rejects_unsupported_version() {
        let req = Request {
            version: 99,
            kind: RequestKind::Auth,
            request_id: [0; REQUEST_ID_LEN],
            uid_hint: 1000,
            service: "test".to_string(),
            deadline_monotonic_ns: 0,
        };
        assert!(req.validate().is_err());
        if let Err(ValidationError::UnsupportedVersion { version }) = req.validate() {
            assert_eq!(version, 99);
        } else {
            panic!("expected UnsupportedVersion error");
        }
    }

    // --- Verdict Tests ---

    #[test]
    fn verdict_allow_does_not_ignore() {
        assert!(!Verdict::Allow.should_ignore());
    }

    #[test]
    fn verdict_deny_should_ignore() {
        assert!(Verdict::Deny.should_ignore());
    }

    #[test]
    fn verdict_unavailable_should_ignore() {
        assert!(Verdict::Unavailable.should_ignore());
    }

    #[test]
    fn verdict_protocol_error_should_ignore() {
        assert!(Verdict::ProtocolError.should_ignore());
    }

    // --- Response Tests ---

    #[test]
    fn response_is_allow_only_with_current_version() {
        let resp = test_response(Verdict::Allow);
        assert!(resp.is_allow());

        let mut bad_version = test_response(Verdict::Allow);
        bad_version.version = 99;
        assert!(!bad_version.is_allow());
    }

    #[test]
    fn response_matches_request_id() {
        let resp = test_response(Verdict::Allow);
        let matching_id = [0xAB; REQUEST_ID_LEN];
        let wrong_id = [0x00; REQUEST_ID_LEN];

        assert!(resp.matches_request(&matching_id));
        assert!(!resp.matches_request(&wrong_id));
    }

    // --- Malformed Buffer Tests ---

    #[test]
    fn decode_empty_buffer_fails() {
        let result: Result<Request, _> = decode(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn decode_short_buffer_fails() {
        let result: Result<Request, _> = decode(&[0, 0, 0]);
        assert!(result.is_err());
    }

    #[test]
    fn decode_truncated_payload_fails() {
        let req = test_request();
        let encoded = encode(&req).expect("encode should succeed");
        let truncated = &encoded[..encoded.len() - 5];
        let result: Result<Request, _> = decode(truncated);
        assert!(result.is_err());
    }
}
