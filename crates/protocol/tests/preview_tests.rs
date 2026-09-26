//! Contractual tests for Issue #46: IPC Video Preview Frame Proxy.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite uses direct assertions and unwrap"
)]

use soos_protocol::codec::{decode, decode_preview, encode, encode_preview, CodecError};
use soos_protocol::types::{
    PreviewResponse, Request, RequestKind, CURRENT_VERSION, MAX_MESSAGE_SIZE,
    MAX_PREVIEW_MESSAGE_SIZE,
};

#[test]
fn test_preview_frame_request_and_response_roundtrip() {
    let req = Request {
        version: CURRENT_VERSION,
        kind: RequestKind::PreviewFrame,
        request_id: [0x5A; 32],
        uid_hint: 1000,
        service: "soos-gui".to_string(),
        deadline_monotonic_ns: 1_000_000_000,
    };

    // Standard encode must succeed for PreviewFrame request since Request is small (< 4KB)
    let encoded_req = encode(&req).expect("Encode PreviewFrame request");
    let decoded_req: Request = decode(&encoded_req).expect("Decode PreviewFrame request");
    assert_eq!(decoded_req.kind, RequestKind::PreviewFrame);
    assert_eq!(decoded_req.service, "soos-gui");

    // Construct a synthetic 640x480 RGB frame (921,600 bytes)
    let raw_pixels = vec![200u8; 640 * 480 * 3];
    let preview_resp = PreviewResponse {
        version: CURRENT_VERSION,
        sequence: 42,
        width: 640,
        height: 480,
        format: 0, // Rgb24
        timestamp_monotonic_ns: 999_999_999,
        data: raw_pixels,
    };

    // Standard encode should reject since 921KB > MAX_MESSAGE_SIZE (4,096 bytes)
    assert!(
        matches!(
            encode(&preview_resp),
            Err(CodecError::MessageTooLarge { .. })
        ),
        "Standard encode MUST reject preview frames > 4096 bytes"
    );

    // Preview encode must succeed for large frame up to MAX_PREVIEW_MESSAGE_SIZE (2 MiB)
    let encoded_preview = encode_preview(&preview_resp).expect("Encode preview response");
    assert!(encoded_preview.len() > MAX_MESSAGE_SIZE);
    assert!(encoded_preview.len() <= MAX_PREVIEW_MESSAGE_SIZE);

    let decoded_preview: PreviewResponse =
        decode_preview(&encoded_preview).expect("Decode preview response");
    assert_eq!(decoded_preview.version, CURRENT_VERSION);
    assert_eq!(decoded_preview.sequence, 42);
    assert_eq!(decoded_preview.width, 640);
    assert_eq!(decoded_preview.height, 480);
    assert_eq!(decoded_preview.format, 0);
    assert_eq!(decoded_preview.timestamp_monotonic_ns, 999_999_999);
    assert_eq!(decoded_preview.data.len(), 640 * 480 * 3);
}

#[test]
fn test_preview_codec_rejects_oversized_payload() {
    let huge_data = vec![0u8; MAX_PREVIEW_MESSAGE_SIZE + 100];
    let oversized = PreviewResponse {
        version: CURRENT_VERSION,
        sequence: 1,
        width: 1000,
        height: 1000,
        format: 0,
        timestamp_monotonic_ns: 0,
        data: huge_data,
    };

    assert!(
        matches!(
            encode_preview(&oversized),
            Err(CodecError::MessageTooLarge { .. })
        ),
        "encode_preview must reject payloads exceeding MAX_PREVIEW_MESSAGE_SIZE"
    );
}
