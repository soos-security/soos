//! Single-allocation tagged client encoders (GitHub #311 PAM-NEW-3; matrix row PUR12).
//!
//! `encode_request` / `encode_event` size the payload with `postcard::ser_flavors::Size`,
//! reject an oversize frame (payload + tag trailer) before allocating, serialize in place
//! into the one frame buffer, and zeroize that buffer on a serialization failure, exactly
//! like `codec::encode_with_limit` (GitHub #225).

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contract tests use assertions, unwrap and slicing"
)]

use serde::ser::{Error as _, SerializeTuple};
use serde::{Serialize, Serializer};
use soos_protocol::codec::{encode_with_limit_and_trailer, CodecError};
use soos_protocol::message::{
    encode_event, encode_request, MESSAGE_TAG_EVENT, MESSAGE_TAG_REQUEST,
};
use soos_protocol::types::{
    Event, EventKind, Request, RequestKind, CURRENT_VERSION, MAX_MESSAGE_SIZE, REQUEST_ID_LEN,
};

fn request_with_service(service_len: usize) -> Request {
    Request {
        version: CURRENT_VERSION,
        kind: RequestKind::Auth,
        request_id: [0x5A; REQUEST_ID_LEN],
        uid_hint: 1000,
        service: "s".repeat(service_len),
        deadline_monotonic_ns: 250_000_000,
    }
}

fn payload_len(req: &Request) -> usize {
    postcard::to_allocvec(req).unwrap().len()
}

/// Service length giving a `Request` payload of exactly `target` bytes.
fn service_len_for_payload(target: usize) -> usize {
    (0..2 * MAX_MESSAGE_SIZE)
        .find(|n| payload_len(&request_with_service(*n)) == target)
        .expect("a service length reaching the target payload size")
}

/// PUR12: the frame is `u32 BE length | postcard payload | tag`, allocated at its exact size.
#[test]
fn test_pur_tagged_request_frame_layout_and_exact_capacity() {
    let req = request_with_service(4);
    let frame = encode_request(&req).unwrap();
    let payload = postcard::to_allocvec(&req).unwrap();
    assert_eq!(frame.len(), 4 + payload.len() + 1);
    assert_eq!(frame.capacity(), frame.len(), "one exact-size allocation");
    assert_eq!(
        u32::from_be_bytes(frame[..4].try_into().unwrap()) as usize,
        payload.len() + 1
    );
    assert_eq!(&frame[4..frame.len() - 1], payload.as_slice());
    assert_eq!(frame.last(), Some(&MESSAGE_TAG_REQUEST));

    let event = Event {
        version: CURRENT_VERSION,
        kind: EventKind::PasswordFailed,
        request_id: None,
        uid: Some(1000),
        service: "sudo".to_string(),
        timestamp_monotonic_ns: 1,
    };
    let frame = encode_event(&event).unwrap();
    assert_eq!(frame.capacity(), frame.len());
    assert_eq!(frame.last(), Some(&MESSAGE_TAG_EVENT));
}

/// PUR12: payload + tag exactly `MAX_MESSAGE_SIZE` is accepted; one byte more is
/// `MessageTooLarge` counting the tag.
#[test]
fn test_pur_tagged_request_boundary_counts_the_tag() {
    let fits = request_with_service(service_len_for_payload(MAX_MESSAGE_SIZE - 1));
    let frame = encode_request(&fits).unwrap();
    assert_eq!(frame.len(), MAX_MESSAGE_SIZE + 4);

    let over = request_with_service(service_len_for_payload(MAX_MESSAGE_SIZE));
    match encode_request(&over) {
        Err(CodecError::MessageTooLarge { size, max }) => {
            assert_eq!(size, MAX_MESSAGE_SIZE + 1);
            assert_eq!(max, MAX_MESSAGE_SIZE);
        }
        other => panic!("expected MessageTooLarge, got {other:?}"),
    }
}

/// PUR12: a huge payload is refused from its computed size (no frame is built).
#[test]
fn test_pur_tagged_request_oversize_is_rejected_from_the_computed_size() {
    let huge = request_with_service(1 << 20);
    match encode_request(&huge) {
        Err(CodecError::MessageTooLarge { size, .. }) => {
            assert_eq!(size, payload_len(&huge) + 1);
        }
        other => panic!("expected MessageTooLarge, got {other:?}"),
    }
}

/// Serializes some bytes, then fails.
struct FailingAfterBytes;

impl Serialize for FailingAfterBytes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut tuple = serializer.serialize_tuple(2)?;
        tuple.serialize_element(&[0xEEu8; 8])?;
        Err(S::Error::custom("simulated serializer failure"))
    }
}

/// Serializes a different length on every call (sizing pass vs. writing pass).
struct Growing(std::cell::Cell<usize>);

impl Serialize for Growing {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let n = self.0.get();
        self.0.set(n + 1);
        let mut tuple = serializer.serialize_tuple(n)?;
        for _ in 0..n {
            tuple.serialize_element(&0xEEu8)?;
        }
        tuple.end()
    }
}

/// PUR12: a serialization failure or a size mismatch fails closed (the partially written
/// frame is zeroized before it is dropped).
#[test]
fn test_pur_tagged_encoder_fails_closed_on_serializer_errors() {
    assert!(matches!(
        encode_with_limit_and_trailer(&FailingAfterBytes, MAX_MESSAGE_SIZE, Some(0xA1)),
        Err(CodecError::Serialize(_))
    ));
    assert!(matches!(
        encode_with_limit_and_trailer(
            &Growing(std::cell::Cell::new(3)),
            MAX_MESSAGE_SIZE,
            Some(0xA1)
        ),
        Err(CodecError::Serialize(_))
    ));
}

/// PUR12: without a trailer the helper is exactly `encode_with_limit`.
#[test]
fn test_pur_untagged_helper_matches_encode_with_limit() {
    let req = request_with_service(4);
    assert_eq!(
        encode_with_limit_and_trailer(&req, MAX_MESSAGE_SIZE, None).unwrap(),
        soos_protocol::codec::encode_with_limit(&req, MAX_MESSAGE_SIZE).unwrap()
    );
}
