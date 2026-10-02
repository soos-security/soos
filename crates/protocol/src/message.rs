//! Client-to-daemon message discriminator (GitHub #204, review finding DMN-15).
//!
//! A client (PAM module, `soos-admin`, `soos-gui`) sends either a [`Request`] or an
//! [`Event`] on the daemon socket. Codec v1 serialized both without a type tag, so the
//! daemon had to double-decode every frame and pick a handler with a UID heuristic.
//!
//! # Tagged frame (protocol v1 trailer, not readable by pre-#204 daemons)
//!
//! ```text
//! u32 BE length | postcard(Request | Event) | message_tag:u8
//! ```
//!
//! The one-byte **trailer** names the message type: [`MESSAGE_TAG_REQUEST`] or
//! [`MESSAGE_TAG_EVENT`]. Both tags have the high bit set (`>= 0x80`). A complete
//! postcard v1 `Request` or `Event` always ends with the terminating byte of the
//! `u64` varint of its last field (`deadline_monotonic_ns` / `timestamp_monotonic_ns`),
//! whose high bit is always clear. Therefore:
//!
//! - a payload whose last byte is `>= 0x80` can only be a tagged frame, and the tag
//!   alone selects the type (never ambiguous);
//! - a payload whose last byte is `< 0x80` can only be a legacy untagged frame, which is
//!   accepted only when it decodes as exactly one type ([`MessageError::Ambiguous`]
//!   otherwise, fail closed).
//!
//! A trailer (instead of a leading tag) keeps the message body byte-identical to codec v1
//! and [`crate::types::CURRENT_VERSION`] stays `1`, but tagged frames are **not** backward
//! compatible with older readers: only a lenient `postcard::from_bytes::<Request>` would
//! ignore the extra byte, and no shipped reader is lenient. A daemon built before this change
//! (strict codec, GitHub #224) rejects a tagged frame as malformed, so `pam_soos.so` fails
//! closed to the password and must be upgraded together with `soos-daemon`
//! (`Docs/IPC_PROTOCOL.md` §12). In the other direction, this daemon keeps accepting untagged
//! v1 frames. The strict [`crate::codec::decode_payload`] tolerates exactly this one trailer
//! byte, and only the tag matching the decoded type.

use crate::codec::CodecError;
use crate::types::{Event, Request, MAX_MESSAGE_SIZE};

/// Trailer byte of a tagged frame carrying a [`Request`].
pub const MESSAGE_TAG_REQUEST: u8 = 0xA0;

/// Trailer byte of a tagged frame carrying an [`Event`].
pub const MESSAGE_TAG_EVENT: u8 = 0xA1;

/// Lowest byte value reserved for message tags: no complete legacy v1 frame can end on
/// a byte `>= MESSAGE_TAG_MIN` (varint terminating bytes have the high bit clear).
pub const MESSAGE_TAG_MIN: u8 = 0x80;

/// A decoded client-to-daemon message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMessage {
    /// Request expecting a daemon response (`Auth`, `Status`, `PreviewFrame`).
    Request(Request),
    /// Best-effort telemetry event (no response).
    Event(Event),
}

/// Wire format a [`ClientMessage`] was received in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameFormat {
    /// Frame carried an explicit message tag trailer.
    Tagged,
    /// Legacy untagged codec v1 frame, accepted because it decoded as exactly one type.
    Legacy,
}

/// Client message classification errors. Every variant is a rejection (fail closed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageError {
    /// Empty payload.
    Empty,
    /// Payload ends with a reserved tag byte (`>= 0x80`) that names no message type.
    UnknownTag(u8),
    /// Payload does not decode exactly (no trailing bytes) as the announced type, or a
    /// legacy payload decodes as no type at all.
    Malformed,
    /// Legacy untagged payload decodes exactly as both a `Request` and an `Event`.
    Ambiguous,
}

impl core::fmt::Display for MessageError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => write!(f, "empty client message"),
            Self::UnknownTag(tag) => write!(f, "unknown client message tag: {tag:#04x}"),
            Self::Malformed => write!(f, "malformed client message"),
            Self::Ambiguous => write!(
                f,
                "ambiguous untagged client message (decodes as both Request and Event)"
            ),
        }
    }
}

impl std::error::Error for MessageError {}

/// Encodes a [`Request`] as a tagged, length-prefixed frame bounded by [`MAX_MESSAGE_SIZE`].
///
/// # Errors
///
/// Returns [`CodecError::Serialize`] or [`CodecError::MessageTooLarge`].
pub fn encode_request(req: &Request) -> Result<Vec<u8>, CodecError> {
    encode_tagged(req, MESSAGE_TAG_REQUEST)
}

/// Encodes an [`Event`] as a tagged, length-prefixed frame bounded by [`MAX_MESSAGE_SIZE`].
///
/// # Errors
///
/// Returns [`CodecError::Serialize`] or [`CodecError::MessageTooLarge`].
pub fn encode_event(event: &Event) -> Result<Vec<u8>, CodecError> {
    encode_tagged(event, MESSAGE_TAG_EVENT)
}

/// Single-allocation tagged frame: sized first, oversize rejected before allocation,
/// serialized in place, zeroized on error (GitHub #311 PAM-NEW-3, same path as
/// `codec::encode_with_limit`, GitHub #225).
fn encode_tagged<T: serde::Serialize>(msg: &T, tag: u8) -> Result<Vec<u8>, CodecError> {
    crate::codec::encode_with_limit_and_trailer(msg, MAX_MESSAGE_SIZE, Some(tag))
}

/// Decodes `bytes` as exactly one `T`, rejecting any unconsumed trailing byte.
fn decode_exact<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Option<T> {
    crate::codec::decode_payload_exact(bytes).ok()
}

/// Classifies and decodes one client payload (the bytes after the 4-byte length prefix).
///
/// Rules (see the module documentation):
/// 1. last byte `>= MESSAGE_TAG_MIN`: tagged frame; the tag selects the type and the body
///    must decode exactly as that type;
/// 2. otherwise: legacy v1 frame, accepted only if it decodes exactly as one type.
///
/// # Errors
///
/// Returns a [`MessageError`]; the caller must reject the frame without dispatching it.
pub fn decode_client_message(payload: &[u8]) -> Result<(ClientMessage, FrameFormat), MessageError> {
    let (&last, body) = payload.split_last().ok_or(MessageError::Empty)?;

    if last >= MESSAGE_TAG_MIN {
        return match last {
            MESSAGE_TAG_REQUEST => decode_exact::<Request>(body)
                .map(|req| (ClientMessage::Request(req), FrameFormat::Tagged))
                .ok_or(MessageError::Malformed),
            MESSAGE_TAG_EVENT => decode_exact::<Event>(body)
                .map(|event| (ClientMessage::Event(event), FrameFormat::Tagged))
                .ok_or(MessageError::Malformed),
            other => Err(MessageError::UnknownTag(other)),
        };
    }

    match (
        decode_exact::<Request>(payload),
        decode_exact::<Event>(payload),
    ) {
        (Some(req), None) => Ok((ClientMessage::Request(req), FrameFormat::Legacy)),
        (None, Some(event)) => Ok((ClientMessage::Event(event), FrameFormat::Legacy)),
        (Some(_), Some(_)) => Err(MessageError::Ambiguous),
        (None, None) => Err(MessageError::Malformed),
    }
}
