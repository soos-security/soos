//! Strict codec x client message tag trailer contract (GitHub #224 x #204, matrix BBX1-BBX4).
//!
//! `#224` made `codec::decode` / `decode_payload` reject any byte left inside the declared
//! payload. `#204` appends a one-byte message tag trailer (`MESSAGE_TAG_REQUEST` /
//! `MESSAGE_TAG_EVENT`) to every client-to-daemon payload. The strict decoder therefore
//! accepts exactly ONE remainder byte, and only when it is the tag of the decoded type:
//!
//! - `Request` + `MESSAGE_TAG_REQUEST` and `Event` + `MESSAGE_TAG_EVENT` decode;
//! - any other single trailing byte (including the other type's tag, and any tag after a
//!   daemon-to-client `Response`) is rejected with `CodecError::TrailingBytes { 1 }`;
//! - two or more trailing bytes are always rejected, whatever their values.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Contract tests use assertions, unwrap and slicing"
)]

use proptest::prelude::*;
use soos_protocol::codec::{decode, decode_payload, encode, CodecError};
use soos_protocol::message::{
    decode_client_message, encode_event, encode_request, MessageError, MESSAGE_TAG_EVENT,
    MESSAGE_TAG_REQUEST,
};
use soos_protocol::types::{
    Event, EventKind, ReasonClass, Request, RequestKind, Response, Verdict, CURRENT_VERSION,
    REQUEST_ID_LEN,
};

fn auth_request() -> Request {
    Request {
        version: CURRENT_VERSION,
        kind: RequestKind::Auth,
        request_id: [0x5A; REQUEST_ID_LEN],
        uid_hint: 1000,
        service: "sudo".to_string(),
        deadline_monotonic_ns: 250_000_000,
    }
}

fn pam_event() -> Event {
    Event {
        version: CURRENT_VERSION,
        kind: EventKind::PasswordFailed,
        request_id: None,
        uid: Some(1000),
        service: "sudo".to_string(),
        timestamp_monotonic_ns: 42,
    }
}

fn allow_response() -> Response {
    Response {
        version: CURRENT_VERSION,
        request_id: [0xAB; REQUEST_ID_LEN],
        verdict: Verdict::Allow,
        reason_class: ReasonClass::FaceMatch,
        issued_monotonic_ns: 100_000_000,
        expires_monotonic_ns: 102_000_000_000,
    }
}

/// Untagged postcard body of a codec v1 frame.
fn body_of(frame: &[u8]) -> Vec<u8> {
    frame[4..].to_vec()
}

fn with_suffix(body: &[u8], suffix: &[u8]) -> Vec<u8> {
    let mut out = body.to_vec();
    out.extend_from_slice(suffix);
    out
}

fn assert_trailing<T: std::fmt::Debug>(result: Result<T, CodecError>, expected: usize) {
    match result {
        Err(CodecError::TrailingBytes { unconsumed }) => assert_eq!(unconsumed, expected),
        other => panic!("expected TrailingBytes {{ unconsumed: {expected} }}, got {other:?}"),
    }
}

// --- BBX1: the matching tag trailer is accepted ------------------------------------------

#[test]
fn test_strict_decode_accepts_tagged_request_frame() {
    let req = auth_request();
    let decoded: Request = decode(&encode_request(&req).expect("encode")).expect("decode");
    assert_eq!(decoded, req);
}

#[test]
fn test_strict_decode_accepts_tagged_event_frame() {
    let event = pam_event();
    let decoded: Event = decode(&encode_event(&event).expect("encode")).expect("decode");
    assert_eq!(decoded, event);
}

#[test]
fn test_decode_payload_accepts_exactly_the_matching_tag() {
    let req_body = body_of(&encode(&auth_request()).expect("encode"));
    let decoded: Request =
        decode_payload(&with_suffix(&req_body, &[MESSAGE_TAG_REQUEST])).expect("tagged request");
    assert_eq!(decoded, auth_request());

    let event_body = body_of(&encode(&pam_event()).expect("encode"));
    let decoded: Event =
        decode_payload(&with_suffix(&event_body, &[MESSAGE_TAG_EVENT])).expect("tagged event");
    assert_eq!(decoded, pam_event());
}

// --- BBX2: any other single trailing byte is rejected ------------------------------------

#[test]
fn test_request_followed_by_the_event_tag_is_rejected() {
    let body = body_of(&encode(&auth_request()).expect("encode"));
    assert_trailing(
        decode_payload::<Request>(&with_suffix(&body, &[MESSAGE_TAG_EVENT])),
        1,
    );
}

#[test]
fn test_event_followed_by_the_request_tag_is_rejected() {
    let body = body_of(&encode(&pam_event()).expect("encode"));
    assert_trailing(
        decode_payload::<Event>(&with_suffix(&body, &[MESSAGE_TAG_REQUEST])),
        1,
    );
}

#[test]
fn test_response_never_accepts_a_client_message_tag() {
    let body = body_of(&encode(&allow_response()).expect("encode"));
    for tag in [MESSAGE_TAG_REQUEST, MESSAGE_TAG_EVENT] {
        assert_trailing(decode_payload::<Response>(&with_suffix(&body, &[tag])), 1);
    }
}

#[test]
fn test_every_non_tag_single_trailing_byte_is_rejected() {
    let req_body = body_of(&encode(&auth_request()).expect("encode"));
    let event_body = body_of(&encode(&pam_event()).expect("encode"));
    for byte in 0..=u8::MAX {
        if byte != MESSAGE_TAG_REQUEST {
            assert_trailing(
                decode_payload::<Request>(&with_suffix(&req_body, &[byte])),
                1,
            );
        }
        if byte != MESSAGE_TAG_EVENT {
            assert_trailing(
                decode_payload::<Event>(&with_suffix(&event_body, &[byte])),
                1,
            );
        }
    }
}

// --- BBX3: two or more trailing bytes are always rejected --------------------------------

#[test]
fn test_two_trailing_bytes_are_rejected_even_when_one_is_a_tag() {
    let body = body_of(&encode(&auth_request()).expect("encode"));
    for suffix in [
        [MESSAGE_TAG_REQUEST, MESSAGE_TAG_REQUEST],
        [0x00, MESSAGE_TAG_REQUEST],
        [MESSAGE_TAG_REQUEST, 0x00],
        [MESSAGE_TAG_EVENT, MESSAGE_TAG_REQUEST],
    ] {
        assert_trailing(decode_payload::<Request>(&with_suffix(&body, &suffix)), 2);
    }
}

#[test]
fn test_framed_request_with_tag_and_extra_byte_is_rejected() {
    let mut frame = encode_request(&auth_request()).expect("encode");
    frame.push(0x00);
    let len = (frame.len() - 4) as u32;
    frame[..4].copy_from_slice(&len.to_be_bytes());
    assert_trailing(decode::<Request>(&frame), 2);
}

// --- BBX4: the daemon classifier stays exact (no double tag) -----------------------------

#[test]
fn test_client_message_with_a_doubled_tag_is_rejected() {
    let body = body_of(&encode(&auth_request()).expect("encode"));
    let doubled = with_suffix(&body, &[MESSAGE_TAG_REQUEST, MESSAGE_TAG_REQUEST]);
    assert_eq!(
        decode_client_message(&doubled),
        Err(MessageError::Malformed)
    );

    let event_body = body_of(&encode(&pam_event()).expect("encode"));
    let doubled = with_suffix(&event_body, &[MESSAGE_TAG_EVENT, MESSAGE_TAG_EVENT]);
    assert_eq!(
        decode_client_message(&doubled),
        Err(MessageError::Malformed)
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Any remainder of 2..=64 bytes after a Request is rejected with its exact length.
    #[test]
    fn prop_multi_byte_remainder_after_request_is_rejected(
        extra in proptest::collection::vec(any::<u8>(), 2..=64),
    ) {
        let body = body_of(&encode(&auth_request()).expect("encode"));
        let is_trailing = matches!(
            decode_payload::<Request>(&with_suffix(&body, &extra)),
            Err(CodecError::TrailingBytes { unconsumed }) if unconsumed == extra.len()
        );
        prop_assert!(is_trailing);
    }
}
