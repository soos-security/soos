//! Contract tests of `RequestKind::LockState` and `LockStateResponse` (GitHub #380, PR 1 of
//! #352; ADR 2026-10-13 "Read-Only Lock-Screen State Channel"; matrix LKS1-LKS3).
//!
//! - wire indices: `RequestKind::LockState` is 3, `LockWidgetState` is `Idle` 0, `Scanning` 1,
//!   `NotRecognized` 2, and nothing else decodes (there is no success value on the wire);
//! - bounds: a response payload never exceeds `LOCK_STATE_RESPONSE_MAX_PAYLOAD` (12 bytes);
//! - strict codec: round-trip, truncation and trailing bytes;
//! - exclusivity: a `LockStateResponse` frame never decodes as a `Response`, `StatusResponse`
//!   or `PreviewResponse` and the other way round, so a client can tell an answer from a refusal.

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
use soos_protocol::codec::{decode, decode_preview, encode, CodecError};
use soos_protocol::message::{decode_client_message, encode_request, ClientMessage};
use soos_protocol::types::{
    LockStateResponse, LockWidgetState, PreviewResponse, ReasonClass, Request, RequestKind,
    Response, StatusResponse, Verdict, CURRENT_VERSION, LOCK_STATE_RESPONSE_MAX_PAYLOAD,
    MAX_MESSAGE_SIZE, REQUEST_ID_LEN,
};

const ALL_STATES: [LockWidgetState; 3] = [
    LockWidgetState::Idle,
    LockWidgetState::Scanning,
    LockWidgetState::NotRecognized,
];

fn arb_state() -> impl Strategy<Value = LockWidgetState> {
    prop_oneof![
        Just(LockWidgetState::Idle),
        Just(LockWidgetState::Scanning),
        Just(LockWidgetState::NotRecognized),
    ]
}

fn arb_lock_state() -> impl Strategy<Value = LockStateResponse> {
    (arb_state(), any::<u64>()).prop_map(|(state, sequence)| LockStateResponse {
        version: CURRENT_VERSION,
        state,
        sequence,
    })
}

fn lock_state_request(uid: u32) -> Request {
    Request {
        version: CURRENT_VERSION,
        kind: RequestKind::LockState,
        request_id: [0x5C; REQUEST_ID_LEN],
        uid_hint: uid,
        service: "soos-lockstatus".into(),
        deadline_monotonic_ns: u64::MAX,
    }
}

/// Re-frames `payload` with a correct big-endian length prefix.
fn frame_of(payload: &[u8]) -> Vec<u8> {
    let mut frame = (payload.len() as u32).to_be_bytes().to_vec();
    frame.extend_from_slice(payload);
    frame
}

#[test]
fn test_lks_request_kind_lock_state_is_wire_value_3() {
    assert_eq!(RequestKind::LockState as u8, 3);
    let frame = encode(&lock_state_request(1000)).expect("encode");
    // payload: version (1 raw byte) then the kind discriminant varint.
    assert_eq!(frame[4], CURRENT_VERSION);
    assert_eq!(frame[5], 3, "LockState travels as discriminant 3");
}

#[test]
fn test_lks_widget_state_wire_values() {
    assert_eq!(LockWidgetState::Idle as u8, 0);
    assert_eq!(LockWidgetState::Scanning as u8, 1);
    assert_eq!(LockWidgetState::NotRecognized as u8, 2);
    for (index, state) in ALL_STATES.iter().enumerate() {
        let frame = encode(&LockStateResponse {
            version: CURRENT_VERSION,
            state: *state,
            sequence: 7,
        })
        .expect("encode");
        assert_eq!(frame[5] as usize, index, "state {state:?} wire byte");
    }
}

#[test]
fn test_lks_unknown_state_byte_is_a_decode_error() {
    for byte in 3u8..=255 {
        let payload = [CURRENT_VERSION, byte, 1, 0x00];
        // The state is one raw byte: every value above 2 is refused.
        let result = decode::<LockStateResponse>(&frame_of(&payload[..3]));
        assert!(
            result.is_err(),
            "state byte {byte} must never decode, got {result:?}"
        );
    }
    // An overlong varint encoding of a valid index (0x81 0x00 = 1) is no state: the state is a
    // raw byte, so 0x81 is refused (plan evaluation F1).
    let result = decode::<LockStateResponse>(&frame_of(&[CURRENT_VERSION, 0x81, 0x00, 0x01]));
    assert!(
        result.is_err(),
        "overlong state encoding must be refused: {result:?}"
    );
}

#[test]
fn test_lks_response_payload_is_bounded() {
    for state in ALL_STATES {
        for sequence in [0, 1, 127, 128, u64::from(u32::MAX), u64::MAX] {
            let frame = encode(&LockStateResponse {
                version: CURRENT_VERSION,
                state,
                sequence,
            })
            .expect("encode");
            let payload_len = frame.len() - 4;
            assert!(payload_len <= LOCK_STATE_RESPONSE_MAX_PAYLOAD);
            assert!(payload_len <= MAX_MESSAGE_SIZE);
        }
    }
    assert_eq!(LOCK_STATE_RESPONSE_MAX_PAYLOAD, 12);
    let max = encode(&LockStateResponse {
        version: CURRENT_VERSION,
        state: LockWidgetState::NotRecognized,
        sequence: u64::MAX,
    })
    .expect("encode");
    assert_eq!(
        max.len() - 4,
        LOCK_STATE_RESPONSE_MAX_PAYLOAD,
        "the bound is tight"
    );
}

#[test]
fn test_lks_is_current_checks_the_version() {
    let mut resp = LockStateResponse {
        version: CURRENT_VERSION,
        state: LockWidgetState::Scanning,
        sequence: 1,
    };
    assert!(resp.is_current());
    resp.version = CURRENT_VERSION.wrapping_add(1);
    assert!(!resp.is_current());
}

#[test]
fn test_lks_truncated_and_trailing_payloads_are_rejected() {
    let frame = encode(&LockStateResponse {
        version: CURRENT_VERSION,
        state: LockWidgetState::Idle,
        sequence: 300,
    })
    .expect("encode");
    let payload = &frame[4..];
    for cut in 0..payload.len() {
        assert!(
            decode::<LockStateResponse>(&frame_of(&payload[..cut])).is_err(),
            "truncated at {cut}"
        );
    }
    let mut longer = payload.to_vec();
    longer.push(0);
    assert!(matches!(
        decode::<LockStateResponse>(&frame_of(&longer)),
        Err(CodecError::TrailingBytes { unconsumed: 1 })
    ));
}

#[test]
fn test_lks_lock_state_request_is_a_tagged_client_request() {
    let req = lock_state_request(4242);
    let frame = encode_request(&req).expect("encode");
    let (msg, _) = decode_client_message(&frame[4..]).expect("classifies");
    assert_eq!(msg, ClientMessage::Request(req.clone()));
    assert!(req.validate().is_ok());
}

#[test]
fn test_lks_refusal_response_never_decodes_as_lock_state() {
    let refusal = Response {
        version: CURRENT_VERSION,
        request_id: [0; REQUEST_ID_LEN],
        verdict: Verdict::ProtocolError,
        reason_class: ReasonClass::UidMismatch,
        issued_monotonic_ns: 1,
        expires_monotonic_ns: 2,
    };
    let frame = encode(&refusal).expect("encode");
    assert!(decode::<LockStateResponse>(&frame).is_err());
}

#[test]
fn test_lks_status_and_preview_never_decode_as_lock_state() {
    let status = StatusResponse {
        version: CURRENT_VERSION,
        socket_ready: true,
        camera_ready: false,
        models_verified: true,
        is_healthy: false,
        pid: 1,
        uptime_secs: 0,
        memory_locked: false,
    };
    assert!(decode::<LockStateResponse>(&encode(&status).expect("encode")).is_err());
    let preview = PreviewResponse {
        version: CURRENT_VERSION,
        sequence: 0,
        width: 0,
        height: 0,
        format: 255,
        timestamp_monotonic_ns: 0,
        data: Vec::new(),
    };
    assert!(decode::<LockStateResponse>(&encode(&preview).expect("encode")).is_err());
}

proptest! {
    #[test]
    fn prop_lks_round_trip(resp in arb_lock_state()) {
        let frame = encode(&resp).expect("encode");
        prop_assert!(frame.len() - 4 <= LOCK_STATE_RESPONSE_MAX_PAYLOAD);
        let back: LockStateResponse = decode(&frame).expect("decode");
        prop_assert_eq!(back, resp);
    }

    #[test]
    fn prop_lks_lock_state_is_never_another_response(resp in arb_lock_state()) {
        let frame = encode(&resp).expect("encode");
        prop_assert!(decode::<Response>(&frame).is_err());
        prop_assert!(decode::<StatusResponse>(&frame).is_err());
        prop_assert!(decode_preview::<PreviewResponse>(&frame).is_err());
        prop_assert!(decode::<Request>(&frame).is_err());
    }

    #[test]
    fn prop_lks_decoder_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..64)) {
        if let Ok(resp) = decode::<LockStateResponse>(&frame_of(&bytes)) {
            prop_assert!(ALL_STATES.contains(&resp.state));
            prop_assert!(bytes.len() <= LOCK_STATE_RESPONSE_MAX_PAYLOAD);
        }
        let _ = decode::<LockStateResponse>(&bytes);
    }

    #[test]
    fn prop_lks_request_round_trip(uid in any::<u32>(), service in "[a-z0-9-]{0,64}") {
        let req = Request { service, ..lock_state_request(uid) };
        let frame = encode_request(&req).expect("encode");
        let (msg, _) = decode_client_message(&frame[4..]).expect("classifies");
        prop_assert_eq!(msg, ClientMessage::Request(req));
    }
}
