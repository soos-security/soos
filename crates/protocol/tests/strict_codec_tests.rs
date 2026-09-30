//! Strict payload decoding and single-buffer encoding contract (GitHub #224, #225).
//!
//! - `decode` / `decode_preview` / `decode_with_limit` must consume the declared payload
//!   exactly: bytes left over INSIDE the declared length are a protocol error
//!   (`CodecError::TrailingBytes`), so the PAM client and the daemon apply one strictness.
//!   The only exception is the one-byte client message tag of the decoded type (#204).
//! - `decode_payload` is the shared strict decoder of an unframed payload used by the daemon.
//! - Bytes after the end of the declared frame are not part of the frame and stay ignored.
//! - `encode` produces exactly `u32 BE length || postcard payload` (wire compatibility with
//!   the codec before #225, which serialized into an intermediate buffer).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Contract tests use direct assertions, panics, and slicing"
)]

use proptest::prelude::*;
use soos_protocol::codec::{
    decode, decode_payload, decode_preview, encode, encode_preview, CodecError,
};
use soos_protocol::message::MESSAGE_TAG_REQUEST;
use soos_protocol::types::{
    Event, EventKind, PreviewResponse, ReasonClass, Request, RequestKind, Response, Verdict,
    CURRENT_VERSION, REQUEST_ID_LEN,
};

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

/// Re-frames `frame` so that `extra` garbage bytes sit INSIDE the declared payload length.
fn append_inside_declared_length(frame: &[u8], extra: &[u8]) -> Vec<u8> {
    let payload = &frame[4..];
    let new_len = (payload.len() + extra.len()) as u32;
    let mut out = Vec::with_capacity(4 + payload.len() + extra.len());
    out.extend_from_slice(&new_len.to_be_bytes());
    out.extend_from_slice(payload);
    out.extend_from_slice(extra);
    out
}

#[test]
fn decode_rejects_trailing_bytes() {
    let frame = encode(&allow_response()).expect("encode");
    let tampered = append_inside_declared_length(&frame, &[0xDE, 0xAD, 0x00]);

    let result: Result<Response, CodecError> = decode(&tampered);
    assert!(
        result.is_err(),
        "an Allow response followed by garbage inside the declared length must not decode"
    );
}

#[test]
fn decode_trailing_bytes_error_reports_unconsumed_count() {
    let frame = encode(&allow_response()).expect("encode");
    let tampered = append_inside_declared_length(&frame, &[1, 2, 3]);

    match decode::<Response>(&tampered) {
        Err(CodecError::TrailingBytes { unconsumed }) => assert_eq!(unconsumed, 3),
        other => panic!("expected TrailingBytes {{ unconsumed: 3 }}, got {other:?}"),
    }
}

#[test]
fn decode_preview_rejects_trailing_bytes() {
    let preview = PreviewResponse {
        version: CURRENT_VERSION,
        sequence: 7,
        width: 2,
        height: 1,
        format: 0,
        timestamp_monotonic_ns: 42,
        data: vec![1, 2, 3, 4, 5, 6],
    };
    let frame = encode_preview(&preview).expect("encode preview");
    let tampered = append_inside_declared_length(&frame, &[0]);

    assert!(matches!(
        decode_preview::<PreviewResponse>(&tampered),
        Err(CodecError::TrailingBytes { unconsumed: 1 })
    ));
}

#[test]
fn decode_ignores_bytes_after_the_declared_frame() {
    // A caller may hand a larger read buffer; only the declared frame is parsed.
    let mut frame = encode(&allow_response()).expect("encode");
    frame.extend_from_slice(&[0xFF; 16]);

    let decoded: Response = decode(&frame).expect("bytes after the frame are not the payload");
    assert!(decoded.is_allow());
}

#[test]
fn decode_payload_is_strict_and_exact() {
    let frame = encode(&auth_request()).expect("encode");
    let payload = &frame[4..];

    let decoded: Request = decode_payload(payload).expect("exact payload decodes");
    assert_eq!(decoded, auth_request());

    let mut longer = payload.to_vec();
    longer.push(0);
    assert!(matches!(
        decode_payload::<Request>(&longer),
        Err(CodecError::TrailingBytes { unconsumed: 1 })
    ));
}

#[test]
fn decode_payload_rejects_a_request_payload_as_event_when_bytes_remain() {
    // A genuine Request payload must never be accepted as an Event with leftover bytes.
    let frame = encode(&auth_request()).expect("encode");
    let as_event = decode_payload::<Event>(&frame[4..]);
    assert!(as_event.is_err());
}

#[test]
fn encode_is_length_prefix_followed_by_postcard_payload() {
    let resp = allow_response();
    let reference = postcard::to_allocvec(&resp).expect("reference serialization");
    let frame = encode(&resp).expect("encode");

    assert_eq!(&frame[..4], &(reference.len() as u32).to_be_bytes());
    assert_eq!(&frame[4..], reference.as_slice());
}

#[test]
fn encode_allocates_the_frame_exactly_once_at_its_final_size() {
    let evt = Event {
        version: CURRENT_VERSION,
        kind: EventKind::PasswordFailed,
        request_id: Some([0xCD; REQUEST_ID_LEN]),
        uid: Some(1000),
        service: "gdm-password".to_string(),
        timestamp_monotonic_ns: 9,
    };
    let frame = encode(&evt).expect("encode");
    assert_eq!(
        frame.capacity(),
        frame.len(),
        "the frame is sized up front, so no reallocation leaves a stale copy behind"
    );
}

fn arb_response() -> impl Strategy<Value = Response> {
    (
        any::<[u8; REQUEST_ID_LEN]>(),
        0u8..4,
        any::<u64>(),
        any::<u64>(),
    )
        .prop_map(|(request_id, v, issued, expires)| Response {
            version: CURRENT_VERSION,
            request_id,
            verdict: match v {
                0 => Verdict::Allow,
                1 => Verdict::Deny,
                2 => Verdict::Unavailable,
                _ => Verdict::ProtocolError,
            },
            reason_class: ReasonClass::FaceMatch,
            issued_monotonic_ns: issued,
            expires_monotonic_ns: expires,
        })
}

fn arb_request() -> impl Strategy<Value = Request> {
    (
        any::<[u8; REQUEST_ID_LEN]>(),
        any::<u32>(),
        "[a-z-]{0,64}",
        any::<u64>(),
    )
        .prop_map(|(request_id, uid_hint, service, deadline)| Request {
            version: CURRENT_VERSION,
            kind: RequestKind::Auth,
            request_id,
            uid_hint,
            service,
            deadline_monotonic_ns: deadline,
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Any valid Response with 1..=64 extra bytes inside the declared length is rejected.
    #[test]
    fn prop_response_with_trailing_bytes_never_decodes(
        resp in arb_response(),
        extra in proptest::collection::vec(any::<u8>(), 1..=64),
    ) {
        let frame = encode(&resp).expect("encode");
        let tampered = append_inside_declared_length(&frame, &extra);
        let is_trailing = matches!(
            decode::<Response>(&tampered),
            Err(CodecError::TrailingBytes { unconsumed }) if unconsumed == extra.len()
        );
        prop_assert!(is_trailing);
    }

    /// Any valid Request with extra bytes inside the declared length is rejected, and the
    /// exact frame still round-trips. The single `MESSAGE_TAG_REQUEST` trailer of a tagged
    /// client frame (#204) is the one accepted remainder, pinned by
    /// `tag_trailer_codec_tests.rs` (matrix BBX1-BBX3).
    #[test]
    fn prop_request_strict_roundtrip(
        req in arb_request(),
        extra in proptest::collection::vec(any::<u8>(), 1..=64),
    ) {
        prop_assume!(extra != [MESSAGE_TAG_REQUEST]);
        let frame = encode(&req).expect("encode");
        let decoded: Request = decode(&frame).expect("exact frame decodes");
        prop_assert_eq!(&decoded, &req);

        let tampered = append_inside_declared_length(&frame, &extra);
        prop_assert!(decode::<Request>(&tampered).is_err());
    }
}
