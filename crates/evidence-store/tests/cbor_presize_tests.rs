//! Contract (candid review 2026-09-30, Finding 4): `EvidenceRecord::to_cbor` serializes the
//! plaintext record into a buffer reserved up front at its exact encoded size, so the buffer
//! never reallocates while holding pixels (a reallocation would free an unzeroized copy).
//! Observable effect: the returned buffer's capacity equals its length.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions"
)]

use soos_evidence_store::{
    EvidencePixelFormat, EvidenceRecord, FrameBytes, FrameMetadata, EVIDENCE_RECORD_VERSION,
};

fn record((width, height): (u32, u32), with_frame: bool, reason: &str) -> EvidenceRecord {
    let payload_len = usize::try_from(width * height).unwrap();
    let frame = with_frame.then_some(FrameMetadata {
        width,
        height,
        pixel_format: EvidencePixelFormat::Gray8,
        captured_at_mono_ns: u64::MAX,
        sequence: u64::MAX,
    });
    EvidenceRecord {
        format_version: EVIDENCE_RECORD_VERSION,
        snapshot_id: "0b0f7b4e-1c52-4b8e-9d0e-6a1f2c3d4e5f".to_string(),
        uid: u32::MAX,
        timestamp: u64::MAX,
        reason: reason.to_string(),
        frame,
        image_data: FrameBytes::new(vec![0xA5; payload_len]),
    }
}

#[test]
fn test_evidence_to_cbor_reserves_the_exact_size_and_never_reallocates() {
    let long_reason = "r".repeat(300);
    for dims in [
        (1, 1),
        (23, 1),
        (24, 1),
        (255, 1),
        (16, 16),
        (4097, 1),
        (256, 256),
        (1027, 1021),
    ] {
        for with_frame in [false, true] {
            for reason in ["auth_failure", long_reason.as_str()] {
                let rec = record(dims, with_frame, reason);
                let cbor = rec.to_cbor().unwrap();
                assert_eq!(
                    cbor.capacity(),
                    cbor.len(),
                    "frame {dims:?} (metadata {with_frame}): buffer must be pre-sized exactly"
                );
                assert_eq!(EvidenceRecord::from_cbor(&cbor).unwrap(), rec);
            }
        }
    }
}
