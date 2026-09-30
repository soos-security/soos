//! Contract tests for the client message discriminator (GitHub #204 / DMN-15).
//!
//! Every client-to-daemon frame must be classified by a protocol rule, never by a
//! heuristic: tagged frames by their trailer, legacy untagged frames only when they
//! decode as exactly one type; any frame decoding as both types is rejected.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contract tests use assertions, unwrap and slicing"
)]

use proptest::prelude::*;
use soos_protocol::codec::{decode, encode};
use soos_protocol::message::{
    decode_client_message, encode_event, encode_request, ClientMessage, FrameFormat, MessageError,
    MESSAGE_TAG_EVENT, MESSAGE_TAG_MIN, MESSAGE_TAG_REQUEST,
};
use soos_protocol::types::{
    Event, EventKind, Request, RequestKind, CURRENT_VERSION, MAX_MESSAGE_SIZE, REQUEST_ID_LEN,
};

fn auth_request(uid: u32) -> Request {
    Request {
        version: CURRENT_VERSION,
        kind: RequestKind::Auth,
        request_id: [0x5A; REQUEST_ID_LEN],
        uid_hint: uid,
        service: "sudo".to_string(),
        deadline_monotonic_ns: 250_000_000,
    }
}

fn pam_event(uid: u32) -> Event {
    Event {
        version: CURRENT_VERSION,
        kind: EventKind::PasswordFailed,
        request_id: None,
        uid: Some(uid),
        service: "gdm-password".to_string(),
        timestamp_monotonic_ns: 123_456_789,
    }
}

/// A legacy (untagged) `Request` whose bytes also decode exactly as an `Event`
/// (the DMN-15 collision: `uid_hint == 0`, as a root PAM peer would present).
///
/// Request bytes: `01 00 | rid[32] | uid_hint=00 | service_len=00 | deadline=00` (37 bytes).
/// Read as an Event: `01 00 | request_id=None(00) | uid=Some(01) 05 | service_len=30 |
/// 30 service bytes (rid[4..], uid_hint, service_len) | timestamp=00`.
fn ambiguous_request() -> Request {
    let mut request_id = [b'a'; REQUEST_ID_LEN];
    request_id[0] = 0x00;
    request_id[1] = 0x01;
    request_id[2] = 0x05;
    request_id[3] = 30;
    Request {
        version: CURRENT_VERSION,
        kind: RequestKind::Auth,
        request_id,
        uid_hint: 0,
        service: String::new(),
        deadline_monotonic_ns: 0,
    }
}

fn payload(frame: &[u8]) -> &[u8] {
    &frame[4..]
}

#[test]
fn test_tagged_request_decodes_as_request() {
    let req = auth_request(1000);
    let frame = encode_request(&req).expect("encode");
    assert_eq!(*frame.last().unwrap(), MESSAGE_TAG_REQUEST);
    let (msg, format) = decode_client_message(payload(&frame)).expect("decode");
    assert_eq!(format, FrameFormat::Tagged);
    assert_eq!(msg, ClientMessage::Request(req));
}

#[test]
fn test_tagged_event_decodes_as_event() {
    let event = pam_event(1000);
    let frame = encode_event(&event).expect("encode");
    assert_eq!(*frame.last().unwrap(), MESSAGE_TAG_EVENT);
    let (msg, format) = decode_client_message(payload(&frame)).expect("decode");
    assert_eq!(format, FrameFormat::Tagged);
    assert_eq!(msg, ClientMessage::Event(event));
}

#[test]
fn test_tags_are_distinct_and_reserved() {
    assert_ne!(MESSAGE_TAG_REQUEST, MESSAGE_TAG_EVENT);
    const { assert!(MESSAGE_TAG_REQUEST >= MESSAGE_TAG_MIN) };
    const { assert!(MESSAGE_TAG_EVENT >= MESSAGE_TAG_MIN) };
    assert_eq!(MESSAGE_TAG_MIN, 0x80);
}

#[test]
fn test_legacy_untagged_request_still_accepted() {
    let req = auth_request(1000);
    let frame = encode(&req).expect("encode");
    let (msg, format) = decode_client_message(payload(&frame)).expect("decode");
    assert_eq!(format, FrameFormat::Legacy);
    assert_eq!(msg, ClientMessage::Request(req));
}

#[test]
fn test_legacy_untagged_event_still_accepted() {
    let event = pam_event(1000);
    let frame = encode(&event).expect("encode");
    let (msg, format) = decode_client_message(payload(&frame)).expect("decode");
    assert_eq!(format, FrameFormat::Legacy);
    assert_eq!(msg, ClientMessage::Event(event));
}

#[test]
fn test_legacy_frame_decoding_as_both_types_is_rejected() {
    let frame = encode(&ambiguous_request()).expect("encode");
    let body = payload(&frame);
    // Precondition: the crafted payload really is a double match.
    assert!(postcard::take_from_bytes::<Request>(body).is_ok_and(|(_, r)| r.is_empty()));
    assert!(postcard::take_from_bytes::<Event>(body).is_ok_and(|(_, r)| r.is_empty()));
    assert_eq!(decode_client_message(body), Err(MessageError::Ambiguous));
}

#[test]
fn test_tagged_form_of_ambiguous_request_is_unambiguous() {
    let req = ambiguous_request();
    let frame = encode_request(&req).expect("encode");
    let (msg, _) = decode_client_message(payload(&frame)).expect("decode");
    assert_eq!(msg, ClientMessage::Request(req));
}

#[test]
fn test_unknown_tag_is_rejected() {
    let mut frame = encode_request(&auth_request(1000)).expect("encode");
    *frame.last_mut().unwrap() = 0xFF;
    assert_eq!(
        decode_client_message(payload(&frame)),
        Err(MessageError::UnknownTag(0xFF))
    );
}

#[test]
fn test_tag_mismatching_body_is_rejected() {
    // A Request body announced as an Event must not be dispatched as either.
    let mut frame = encode_request(&auth_request(1000)).expect("encode");
    *frame.last_mut().unwrap() = MESSAGE_TAG_EVENT;
    assert_eq!(
        decode_client_message(payload(&frame)),
        Err(MessageError::Malformed)
    );
}

#[test]
fn test_tagged_body_with_trailing_bytes_is_rejected() {
    let mut body = postcard::to_allocvec(&auth_request(1000)).expect("serialize");
    body.push(0x00);
    body.push(MESSAGE_TAG_REQUEST);
    assert_eq!(decode_client_message(&body), Err(MessageError::Malformed));
}

#[test]
fn test_empty_payload_is_rejected() {
    assert_eq!(decode_client_message(&[]), Err(MessageError::Empty));
}

#[test]
fn test_tagged_frame_stays_readable_by_v1_decoders() {
    // Backward compatibility: a v1 reader (`decode::<Request>`) ignores the trailer.
    let req = auth_request(1000);
    let decoded: Request = decode(&encode_request(&req).expect("encode")).expect("decode");
    assert_eq!(decoded, req);
    let event = pam_event(42);
    let decoded: Event = decode(&encode_event(&event).expect("encode")).expect("decode");
    assert_eq!(decoded, event);
}

#[test]
fn test_tagged_frame_respects_max_message_size() {
    let mut req = auth_request(1000);
    req.service = "s".repeat(MAX_MESSAGE_SIZE);
    assert!(encode_request(&req).is_err());
}

fn arb_request() -> impl Strategy<Value = Request> {
    (
        prop_oneof![
            Just(RequestKind::Auth),
            Just(RequestKind::Status),
            Just(RequestKind::PreviewFrame)
        ],
        proptest::array::uniform32(any::<u8>()),
        any::<u32>(),
        "[a-z0-9-]{0,64}",
        any::<u64>(),
    )
        .prop_map(|(kind, request_id, uid_hint, service, deadline)| Request {
            version: CURRENT_VERSION,
            kind,
            request_id,
            uid_hint,
            service,
            deadline_monotonic_ns: deadline,
        })
}

fn arb_event() -> impl Strategy<Value = Event> {
    (
        proptest::option::of(proptest::array::uniform32(any::<u8>())),
        proptest::option::of(any::<u32>()),
        "[a-z0-9-]{0,64}",
        any::<u64>(),
    )
        .prop_map(|(request_id, uid, service, ts)| Event {
            version: CURRENT_VERSION,
            kind: EventKind::PasswordFailed,
            request_id,
            uid,
            service,
            timestamp_monotonic_ns: ts,
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Every tagged Request is classified as that exact Request, whatever its bytes.
    #[test]
    fn prop_tagged_request_always_classified_as_request(req in arb_request()) {
        let frame = encode_request(&req).expect("encode");
        let (msg, format) = decode_client_message(payload(&frame)).expect("decode");
        prop_assert_eq!(format, FrameFormat::Tagged);
        prop_assert_eq!(msg, ClientMessage::Request(req));
    }

    /// Every tagged Event is classified as that exact Event, whatever its bytes.
    #[test]
    fn prop_tagged_event_always_classified_as_event(event in arb_event()) {
        let frame = encode_event(&event).expect("encode");
        let (msg, format) = decode_client_message(payload(&frame)).expect("decode");
        prop_assert_eq!(format, FrameFormat::Tagged);
        prop_assert_eq!(msg, ClientMessage::Event(event));
    }

    /// A complete legacy v1 frame never ends on a reserved tag byte, so tagged and
    /// legacy frames can never be confused.
    #[test]
    fn prop_legacy_frames_never_end_on_a_tag_byte(req in arb_request(), event in arb_event()) {
        prop_assert!(*encode(&req).expect("encode").last().unwrap() < MESSAGE_TAG_MIN);
        prop_assert!(*encode(&event).expect("encode").last().unwrap() < MESSAGE_TAG_MIN);
    }

    /// Arbitrary bytes never panic and never produce more than one classification.
    #[test]
    fn prop_arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..160)) {
        if let Ok((msg, FrameFormat::Legacy)) = decode_client_message(&bytes) {
            let as_request = postcard::take_from_bytes::<Request>(&bytes)
                .is_ok_and(|(_, rest)| rest.is_empty());
            let as_event = postcard::take_from_bytes::<Event>(&bytes)
                .is_ok_and(|(_, rest)| rest.is_empty());
            prop_assert!(as_request != as_event, "accepted legacy frames match exactly one type");
            match msg {
                ClientMessage::Request(_) => prop_assert!(as_request),
                ClientMessage::Event(_) => prop_assert!(as_event),
            }
        }
    }
}
