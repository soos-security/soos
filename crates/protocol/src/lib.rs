//! # soos-protocol
//!
//! Bounded IPC schemas and codec v1 for communication between the PAM module
//! (`pam_soos.so`) and the background daemon (`soos-daemon`).
//!
//! ## Design Principles
//!
//! - **Zero I/O**: This crate contains zero file, socket, or network dependencies.
//! - **Strictly Bounded**: Maximum serialized payload is constrained to [`MAX_MESSAGE_SIZE`] bytes.
//! - **Versioned**: Every message payload includes an explicit `version` field.
//! - **Memory Safe**: `#![forbid(unsafe_code)]` is strictly enforced.
//!
//! ## Protocol v1 Frame Layout
//!
//! Every frame is a 4-byte Big-Endian `u32` payload length followed by a `postcard`
//! payload. Postcard writes the struct fields in declaration order with no tags: `u8` is
//! one raw byte, `[u8; 32]` is 32 raw bytes, every other integer (`u32`, `u64`) and every
//! enum discriminant is an unsigned LEB128 varint, a `String` is a varint length followed
//! by UTF-8 bytes, and an `Option` is a one-byte tag (0 = None, 1 = Some) followed by the
//! value. Field widths therefore vary with the values (`uid_hint = 1` takes one byte,
//! `uid_hint = 1000` two).
//!
//! ```text
//! Frame:       payload_len (u32 BE, <= MAX_MESSAGE_SIZE) | payload
//! Request v1:  version (u8) | kind (varint) | request_id ([u8; 32]) | uid_hint (varint) |
//!              service (varint len + <= 64 bytes) | deadline_monotonic_ns (varint)
//! Response v1: version (u8) | request_id ([u8; 32]) | verdict (varint) |
//!              reason_class (varint) | issued_monotonic_ns (varint) |
//!              expires_monotonic_ns (varint)
//! ```
//!
//! Decoding is strict: the declared payload must be consumed exactly
//! ([`codec::CodecError::TrailingBytes`] otherwise). The v1 wire format carries no
//! message-type discriminator; see `Docs/IPC_PROTOCOL.md` section 2.

#![forbid(unsafe_code)]
#![deny(clippy::all, clippy::pedantic)]
#![allow(
    clippy::module_name_repetitions,
    reason = "Idiomatic protocol type naming"
)]

pub mod codec;
pub mod types;

pub use codec::{decode, encode};
pub use types::{
    Event, EventKind, ReasonClass, Request, RequestKind, Response, StatusResponse, Verdict,
    CURRENT_VERSION, MAX_MESSAGE_SIZE, MAX_SERVICE_LEN, REQUEST_ID_LEN,
};
