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

#![forbid(unsafe_code)]
#![deny(clippy::all, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod codec;
pub mod types;

pub use codec::{decode, encode};
pub use types::{
    EventKind, ReasonClass, Request, RequestKind, Response, Verdict, CURRENT_VERSION,
    MAX_MESSAGE_SIZE, MAX_SERVICE_LEN, REQUEST_ID_LEN,
};
