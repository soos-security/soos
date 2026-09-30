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
//! ```text
//! Request v1:  version | kind=AUTH | request_id[32] | uid_hint:u32 |
//!              service_len:u8 | service[<=64] | deadline_monotonic_ns:u64
//! Response v1: version | request_id[32] | verdict:u8 | reason_class:u8 |
//!              issued_monotonic_ns:u64 | expires_monotonic_ns:u64
//! ```
//!
//! Client-to-daemon frames (`Request`, `Event`) carry a one-byte message tag trailer
//! (GitHub #204); see [`message`].

#![forbid(unsafe_code)]
#![deny(clippy::all, clippy::pedantic)]
#![allow(
    clippy::module_name_repetitions,
    reason = "Idiomatic protocol type naming"
)]

pub mod codec;
pub mod message;
pub mod types;

pub use codec::{decode, encode};
pub use message::{
    decode_client_message, encode_event, encode_request, ClientMessage, FrameFormat, MessageError,
    MESSAGE_TAG_EVENT, MESSAGE_TAG_REQUEST,
};
pub use types::{
    Event, EventKind, ReasonClass, Request, RequestKind, Response, StatusResponse, Verdict,
    CURRENT_VERSION, MAX_MESSAGE_SIZE, MAX_SERVICE_LEN, REQUEST_ID_LEN,
};
