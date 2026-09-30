//! "Exactly one interpretation" regression contract for the v1 wire format
//! (GitHub #224, review finding PAM-12; matrix rows PCX1-PCX3).
//!
//! PAM-12 reported that the codec had two decoders of different strictness and that the
//! daemon guessed `Request` vs `Event` with a UID heuristic. Both are fixed on `main`
//! (strict `decode_payload`, tagged client frames of GitHub #204). These tests pin the
//! property the review said fuzz/property tests could not express: a frame produced by
//! the canonical encoder of one message type is accepted by exactly one decoder.

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
use soos_protocol::codec::{decode, encode, CodecError};
use soos_protocol::message::{
    decode_client_message, encode_event, encode_request, ClientMessage, FrameFormat, MessageError,
    MESSAGE_TAG_EVENT, MESSAGE_TAG_REQUEST,
};
use soos_protocol::types::{
    Event, EventKind, ReasonClass, Request, RequestKind, Response, StatusResponse, Verdict,
    CURRENT_VERSION, REQUEST_ID_LEN,
};

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

fn arb_response() -> impl Strategy<Value = Response> {
    (
        proptest::array::uniform32(any::<u8>()),
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

fn arb_status() -> impl Strategy<Value = StatusResponse> {
    (any::<[bool; 4]>(), any::<u32>(), any::<u64>()).prop_map(|(flags, pid, uptime_secs)| {
        StatusResponse {
            version: CURRENT_VERSION,
            socket_ready: flags[0],
            camera_ready: flags[1],
            models_verified: flags[2],
            is_healthy: flags[3],
            pid,
            uptime_secs,
        }
    })
}

/// Re-frames `payload` with a correct big-endian length prefix.
fn frame_of(payload: &[u8]) -> Vec<u8> {
    let mut frame = (payload.len() as u32).to_be_bytes().to_vec();
    frame.extend_from_slice(payload);
    frame
}

/// A Request whose untagged bytes also decode exactly as an Event, presented with a
/// `uid_hint` (0) that differs from a non-root peer: the exact shape the removed UID
/// heuristic routed to the Event handler (no response, client timeout).
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

/// PCX2: the tagged form of the PAM-12 collision is classified as a Request, whatever
/// the peer UID, and every other decoder rejects it.
#[test]
fn test_pcx_tagged_ambiguous_request_has_exactly_one_interpretation() {
    let req = ambiguous_request();
    // Precondition: the untagged body really is the PAM-12 collision.
    assert_eq!(
        decode_client_message(&encode(&req).expect("encode")[4..]),
        Err(MessageError::Ambiguous)
    );

    let frame = encode_request(&req).expect("encode");
    let (msg, format) = decode_client_message(&frame[4..]).expect("tagged frame classifies");
    assert_eq!(format, FrameFormat::Tagged);
    assert_eq!(msg, ClientMessage::Request(req.clone()));

    assert_eq!(
        decode::<Request>(&frame).expect("strict Request decode"),
        req
    );
    assert!(decode::<Event>(&frame).is_err(), "never an Event");
    assert!(decode::<Response>(&frame).is_err(), "never a Response");
    assert!(
        decode::<StatusResponse>(&frame).is_err(),
        "never a StatusResponse"
    );
}

/// PCX2: schema evolution. A future Request carrying an extra field that this reader does
/// not know is rejected as malformed, never re-routed to the Event handler.
#[test]
fn test_pcx_tagged_request_with_unknown_extra_field_is_malformed_not_event() {
    for extra in [&[0x00][..], &[0x01, 0x02], &[0x7F; 8]] {
        let body = &encode(&ambiguous_request()).expect("encode")[4..];
        let mut payload = body.to_vec();
        payload.extend_from_slice(extra);
        payload.push(MESSAGE_TAG_REQUEST);
        assert_eq!(
            decode_client_message(&payload),
            Err(MessageError::Malformed),
            "extra field {extra:?} must not be accepted or re-routed"
        );
    }
}

/// PCX3: a server frame never tolerates a client message tag, so the only remainder the
/// strict codec accepts cannot turn a tampered `Allow` into an accepted `Response`.
#[test]
fn test_pcx_response_with_any_client_tag_is_rejected() {
    let resp = Response {
        version: CURRENT_VERSION,
        request_id: [0x11; REQUEST_ID_LEN],
        verdict: Verdict::Allow,
        reason_class: ReasonClass::FaceMatch,
        issued_monotonic_ns: 1,
        expires_monotonic_ns: 2,
    };
    for tag in [MESSAGE_TAG_REQUEST, MESSAGE_TAG_EVENT] {
        let mut payload = encode(&resp).expect("encode")[4..].to_vec();
        payload.push(tag);
        assert!(matches!(
            decode::<Response>(&frame_of(&payload)),
            Err(CodecError::TrailingBytes { unconsumed: 1 })
        ));
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// PCX1: a tagged Request frame is accepted only by the Request decoder.
    #[test]
    fn prop_pcx_tagged_request_decodes_as_no_other_type(req in arb_request()) {
        let frame = encode_request(&req).expect("encode");
        prop_assert_eq!(decode::<Request>(&frame).expect("strict decode"), req);
        prop_assert!(decode::<Event>(&frame).is_err());
        prop_assert!(decode::<Response>(&frame).is_err());
        prop_assert!(decode::<StatusResponse>(&frame).is_err());
    }

    /// PCX1: a tagged Event frame is accepted only by the Event decoder.
    #[test]
    fn prop_pcx_tagged_event_decodes_as_no_other_type(event in arb_event()) {
        let frame = encode_event(&event).expect("encode");
        prop_assert_eq!(decode::<Event>(&frame).expect("strict decode"), event);
        prop_assert!(decode::<Request>(&frame).is_err());
        prop_assert!(decode::<Response>(&frame).is_err());
        prop_assert!(decode::<StatusResponse>(&frame).is_err());
    }

    /// PCX1: the two daemon replies a CLI client may receive on the same socket never
    /// decode as each other, so a client expecting one type cannot accept the other.
    #[test]
    fn prop_pcx_response_and_status_response_are_mutually_exclusive(
        resp in arb_response(),
        status in arb_status(),
    ) {
        let resp_frame = encode(&resp).expect("encode");
        prop_assert_eq!(decode::<Response>(&resp_frame).expect("roundtrip"), resp);
        prop_assert!(decode::<StatusResponse>(&resp_frame).is_err());

        let status_frame = encode(&status).expect("encode");
        prop_assert_eq!(decode::<StatusResponse>(&status_frame).expect("roundtrip"), status);
        prop_assert!(decode::<Response>(&status_frame).is_err());
    }

    /// PCX3: any valid Response followed by one arbitrary byte (tags included) inside the
    /// declared length is rejected: the client-side decoder has no tolerated remainder.
    #[test]
    fn prop_pcx_response_rejects_every_single_trailing_byte(
        resp in arb_response(),
        byte in any::<u8>(),
    ) {
        let mut payload = encode(&resp).expect("encode")[4..].to_vec();
        payload.push(byte);
        let rejected = matches!(
            decode::<Response>(&frame_of(&payload)),
            Err(CodecError::TrailingBytes { unconsumed: 1 })
        );
        prop_assert!(rejected);
    }
}
