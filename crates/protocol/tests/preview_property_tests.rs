//! Property tests for the preview channel (GitHub #227, review PAM-15 item 4):
//! `RequestKind::PreviewFrame` requests, `PreviewResponse` round-trips through
//! `encode_preview` / `decode_preview`, and decoder resilience on arbitrary input up to
//! `MAX_PREVIEW_MESSAGE_SIZE + 4` bytes. Case counts are bounded because preview
//! buffers are up to 2 MiB.

#![forbid(unsafe_code)]

#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Property tests use assertions, panics, casts, and slicing in test harnesses"
)]
#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use soos_protocol::codec::{decode_preview, encode_preview, CodecError};
    use soos_protocol::types::{
        PreviewResponse, Request, RequestKind, Response, CURRENT_VERSION, MAX_MESSAGE_SIZE,
        MAX_PREVIEW_MESSAGE_SIZE, REQUEST_ID_LEN,
    };
    use soos_protocol::{decode, encode};

    /// Every `RequestKind`, including `PreviewFrame` (absent from `arb_request_kind` in
    /// `property_tests.rs`).
    fn arb_any_request_kind() -> impl Strategy<Value = RequestKind> {
        prop_oneof![
            Just(RequestKind::Auth),
            Just(RequestKind::Status),
            Just(RequestKind::PreviewFrame),
        ]
    }

    fn arb_request(kind: impl Strategy<Value = RequestKind>) -> impl Strategy<Value = Request> {
        (
            kind,
            proptest::collection::vec(any::<u8>(), REQUEST_ID_LEN),
            any::<u32>(),
            proptest::string::string_regex("[a-zA-Z0-9._-]{1,64}").expect("regex"),
            any::<u64>(),
        )
            .prop_map(|(kind, id, uid_hint, service, deadline)| {
                let mut request_id = [0u8; REQUEST_ID_LEN];
                request_id.copy_from_slice(&id);
                Request {
                    version: CURRENT_VERSION,
                    kind,
                    request_id,
                    uid_hint,
                    service,
                    deadline_monotonic_ns: deadline,
                }
            })
    }

    /// Preview responses with frames up to 64 KiB (full-size frames are covered by the
    /// boundary tests below).
    fn arb_preview_response() -> impl Strategy<Value = PreviewResponse> {
        (
            any::<u64>(),
            any::<u32>(),
            any::<u32>(),
            0u8..=4,
            any::<u64>(),
            proptest::collection::vec(any::<u8>(), 0..=64 * 1024),
        )
            .prop_map(
                |(sequence, width, height, format, ts, data)| PreviewResponse {
                    version: CURRENT_VERSION,
                    sequence,
                    width,
                    height,
                    format,
                    timestamp_monotonic_ns: ts,
                    data,
                },
            )
    }

    /// Arbitrary preview-sized input: a random length prefix followed by a body whose
    /// length ranges over `0..=MAX_PREVIEW_MESSAGE_SIZE`, so the total spans
    /// `4..=MAX_PREVIEW_MESSAGE_SIZE + 4` bytes. Large bodies are filled with one
    /// repeated random byte to keep generation cheap; small ones are fully random.
    fn arb_preview_bytes() -> impl Strategy<Value = Vec<u8>> {
        prop_oneof![
            proptest::collection::vec(any::<u8>(), 0..=4096),
            (any::<u32>(), 0usize..=MAX_PREVIEW_MESSAGE_SIZE, any::<u8>()).prop_map(
                |(prefix, len, fill)| {
                    let mut buf = Vec::with_capacity(len + 4);
                    buf.extend_from_slice(&prefix.to_be_bytes());
                    buf.resize(len + 4, fill);
                    buf
                }
            ),
            (0usize..=MAX_PREVIEW_MESSAGE_SIZE, any::<u8>()).prop_map(|(len, fill)| {
                // Self-consistent prefix: the decoder must parse the whole body.
                let mut buf = Vec::with_capacity(len + 4);
                buf.extend_from_slice(&(len as u32).to_be_bytes());
                buf.resize(len + 4, fill);
                buf
            }),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// PHS8: every `RequestKind`, including `PreviewFrame`, round-trips.
        #[test]
        fn prop_request_with_any_kind_roundtrip(req in arb_request(arb_any_request_kind())) {
            let bytes = encode(&req).unwrap();
            let back: Request = decode(&bytes).unwrap();
            prop_assert_eq!(back, req);
        }

        /// PHS8: `PreviewResponse` round-trips through the preview codec.
        #[test]
        fn prop_preview_response_roundtrip(resp in arb_preview_response()) {
            let bytes = encode_preview(&resp).unwrap();
            prop_assert!(bytes.len() <= MAX_PREVIEW_MESSAGE_SIZE + 4);
            let back: PreviewResponse = decode_preview(&bytes).unwrap();
            prop_assert_eq!(back, resp);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        /// PHS8: `decode_preview` never panics on arbitrary input up to
        /// `MAX_PREVIEW_MESSAGE_SIZE + 4` bytes, and never accepts a declared size above
        /// the preview limit.
        #[test]
        fn prop_decode_preview_never_panics(bytes in arb_preview_bytes()) {
            let result = decode_preview::<PreviewResponse>(&bytes);
            if bytes.len() >= 4 {
                let declared = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
                if declared > MAX_PREVIEW_MESSAGE_SIZE {
                    let is_size_error = matches!(result, Err(CodecError::DeclaredSizeTooLarge { .. }));
                    prop_assert!(is_size_error);
                }
            } else {
                let is_small = matches!(result, Err(CodecError::BufferTooSmall));
                prop_assert!(is_small);
            }
            // The 4 KiB control codec must refuse every preview-sized declared frame.
            if bytes.len() >= 4 {
                let declared = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
                if declared > MAX_MESSAGE_SIZE {
                    prop_assert!(decode::<Response>(&bytes).is_err());
                }
            }
        }
    }

    /// PHS8: a frame whose payload is exactly at the preview limit is accepted; one byte
    /// more is refused by the encoder, and a prefix declaring limit + 1 by the decoder.
    #[test]
    fn test_preview_codec_boundary_at_max_preview_message_size() {
        let empty = PreviewResponse {
            version: CURRENT_VERSION,
            sequence: 1,
            width: 640,
            height: 480,
            format: 4,
            timestamp_monotonic_ns: 7,
            data: Vec::new(),
        };
        let overhead = encode_preview(&empty).unwrap().len() - 4;
        // postcard encodes the Vec length as a varint: 3 bytes for lengths in 2^14..2^21.
        let data_len = MAX_PREVIEW_MESSAGE_SIZE - overhead - 2;
        let mut at_limit = empty.clone();
        at_limit.data = vec![0x5A; data_len];
        let bytes = encode_preview(&at_limit).unwrap();
        assert_eq!(bytes.len(), MAX_PREVIEW_MESSAGE_SIZE + 4);
        let back: PreviewResponse = decode_preview(&bytes).unwrap();
        assert_eq!(back, at_limit);

        let mut over = empty;
        over.data = vec![0x5A; data_len + 1];
        assert!(matches!(
            encode_preview(&over),
            Err(CodecError::MessageTooLarge { .. })
        ));

        let mut declared_over = vec![0u8; MAX_PREVIEW_MESSAGE_SIZE + 4 + 1];
        declared_over[..4].copy_from_slice(&((MAX_PREVIEW_MESSAGE_SIZE + 1) as u32).to_be_bytes());
        assert!(matches!(
            decode_preview::<PreviewResponse>(&declared_over),
            Err(CodecError::DeclaredSizeTooLarge { .. })
        ));
    }
}
