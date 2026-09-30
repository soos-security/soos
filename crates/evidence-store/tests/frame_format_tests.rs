//! Contractual tests for the self-describing evidence snapshot format (review finding STO-08,
//! GitHub #181).
//!
//! Contract:
//! - `store_frame_snapshot` persists width, height, pixel format, capture time and sequence
//!   inside the authenticated AES-256-GCM envelope, in a versioned record
//!   (`EVIDENCE_RECORD_VERSION`), under `<uuid>.frame.enc` (no misleading `.webp` name).
//! - A stored frame can be reloaded and reconstructed as an RGB image of the stored dimensions.
//! - Inconsistent or unbounded frames are refused before anything is written.
//! - Legacy records (no version, no metadata) still decode as version 1; unknown future
//!   versions and records whose metadata contradicts the payload are refused.
//! - `Debug` never prints frame bytes; `load_snapshot` bounds the file it reads.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions"
)]

use std::fs;

use serde::Serialize;
use soos_evidence_store::crypto::encrypt_payload;
use soos_evidence_store::{
    EvidenceConfig, EvidenceFrame, EvidencePixelFormat, EvidenceStore, EvidenceStoreError,
    FrameMetadata, MasterKey, EVIDENCE_RECORD_VERSION, FRAME_SNAPSHOT_EXTENSION,
    LEGACY_EVIDENCE_RECORD_VERSION, MAX_EVIDENCE_DIMENSION, MAX_EVIDENCE_FILE_BYTES,
};
use tempfile::TempDir;

fn open_store(temp: &TempDir) -> (EvidenceStore, MasterKey) {
    let key = MasterKey::generate().unwrap();
    let config = EvidenceConfig {
        enabled: true,
        base_dir: temp.path().join("evidence"),
        key_path: temp.path().join("evidence.key"),
        retention_days: 7,
        daily_cap_per_uid: 100,
    };
    (EvidenceStore::new(config, key.clone()), key)
}

fn metadata(width: u32, height: u32, pixel_format: EvidencePixelFormat) -> FrameMetadata {
    FrameMetadata {
        width,
        height,
        pixel_format,
        captured_at_mono_ns: 123_456_789,
        sequence: 42,
    }
}

#[test]
fn test_frame_snapshot_roundtrip_is_self_describing() {
    let temp = TempDir::new().unwrap();
    let (store, _) = open_store(&temp);
    let data: Vec<u8> = (0..24u8).collect(); // 4 x 2 RGB24
    let meta = metadata(4, 2, EvidencePixelFormat::Rgb24);

    let stored = store
        .store_frame_snapshot(
            1000,
            "PasswordFailed",
            &EvidenceFrame {
                metadata: meta.clone(),
                data: &data,
            },
            Some("2026-09-30"),
            Some(1_790_000_000),
        )
        .unwrap();

    let name = stored
        .path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert_eq!(FRAME_SNAPSHOT_EXTENSION, ".frame.enc");
    assert!(
        name.ends_with(FRAME_SNAPSHOT_EXTENSION) && !name.contains("webp"),
        "frame snapshots must not claim a WebP encoding: {name}"
    );

    let record = store.load_snapshot(&stored.path).unwrap();
    assert_eq!(record.format_version, EVIDENCE_RECORD_VERSION);
    assert_eq!(EVIDENCE_RECORD_VERSION, 2);
    assert_eq!(record.frame, Some(meta));
    assert_eq!(record.uid, 1000);
    assert_eq!(record.timestamp, 1_790_000_000);
    assert_eq!(record.image_data, data);
}

#[test]
fn test_frame_snapshot_reconstructs_rgb_image_with_stored_dimensions() {
    let temp = TempDir::new().unwrap();
    let (store, _) = open_store(&temp);

    // Grayscale 3 x 2: every channel equals the luma byte.
    let grey = vec![0u8, 50, 100, 150, 200, 255];
    let stored = store
        .store_frame_snapshot(
            1000,
            "PasswordFailed",
            &EvidenceFrame {
                metadata: metadata(3, 2, EvidencePixelFormat::Gray8),
                data: &grey,
            },
            None,
            None,
        )
        .unwrap();
    let record = store.load_snapshot(&stored.path).unwrap();
    let meta = record.frame.clone().unwrap();
    let rgb = record.to_rgb24().unwrap();
    assert_eq!(rgb.len(), (meta.width * meta.height * 3) as usize);
    for (i, luma) in grey.iter().enumerate() {
        assert_eq!(&rgb[i * 3..i * 3 + 3], &[*luma, *luma, *luma]);
    }

    // YUYV 2 x 2 with neutral chroma: RGB equals luma.
    let yuyv = vec![100u8, 128, 100, 128, 200, 128, 200, 128];
    let stored = store
        .store_frame_snapshot(
            1000,
            "PasswordFailed",
            &EvidenceFrame {
                metadata: metadata(2, 2, EvidencePixelFormat::Yuyv),
                data: &yuyv,
            },
            None,
            None,
        )
        .unwrap();
    let rgb = store
        .load_snapshot(&stored.path)
        .unwrap()
        .to_rgb24()
        .unwrap();
    assert_eq!(rgb.len(), 2 * 2 * 3);
    assert!(rgb[..6].iter().all(|&c| c.abs_diff(100) <= 1), "{rgb:?}");
    assert!(rgb[6..].iter().all(|&c| c.abs_diff(200) <= 1), "{rgb:?}");

    // NV12 2 x 2 with neutral chroma.
    let nv12 = vec![10u8, 20, 30, 40, 128, 128];
    let stored = store
        .store_frame_snapshot(
            1000,
            "PasswordFailed",
            &EvidenceFrame {
                metadata: metadata(2, 2, EvidencePixelFormat::Nv12),
                data: &nv12,
            },
            None,
            None,
        )
        .unwrap();
    let rgb = store
        .load_snapshot(&stored.path)
        .unwrap()
        .to_rgb24()
        .unwrap();
    assert_eq!(rgb.len(), 2 * 2 * 3);
    assert!(rgb[..3].iter().all(|&c| c.abs_diff(10) <= 1), "{rgb:?}");
    assert!(rgb[9..].iter().all(|&c| c.abs_diff(40) <= 1), "{rgb:?}");
}

#[test]
fn test_compressed_frame_is_stored_but_not_decoded() {
    let temp = TempDir::new().unwrap();
    let (store, _) = open_store(&temp);
    let mjpeg = vec![0xFFu8, 0xD8, 0x00, 0x11, 0xFF, 0xD9];
    let stored = store
        .store_frame_snapshot(
            1000,
            "PasswordFailed",
            &EvidenceFrame {
                metadata: metadata(640, 480, EvidencePixelFormat::Mjpeg),
                data: &mjpeg,
            },
            None,
            None,
        )
        .unwrap();
    let record = store.load_snapshot(&stored.path).unwrap();
    assert_eq!(record.image_data, mjpeg);
    assert_eq!(
        record.frame.as_ref().unwrap().pixel_format,
        EvidencePixelFormat::Mjpeg
    );
    assert!(matches!(
        record.to_rgb24(),
        Err(EvidenceStoreError::InvalidFrame(_))
    ));
}

#[test]
fn test_frame_snapshot_rejects_inconsistent_or_unbounded_frames() {
    let temp = TempDir::new().unwrap();
    let (store, _) = open_store(&temp);
    let cases: Vec<(FrameMetadata, Vec<u8>)> = vec![
        // RGB24 4 x 2 needs 24 bytes.
        (metadata(4, 2, EvidencePixelFormat::Rgb24), vec![0u8; 23]),
        (metadata(0, 2, EvidencePixelFormat::Gray8), vec![]),
        (metadata(2, 0, EvidencePixelFormat::Gray8), vec![]),
        (
            metadata(MAX_EVIDENCE_DIMENSION + 1, 1, EvidencePixelFormat::Gray8),
            vec![0u8; MAX_EVIDENCE_DIMENSION as usize + 1],
        ),
        // Compressed payloads must not be empty.
        (metadata(640, 480, EvidencePixelFormat::Mjpeg), vec![]),
    ];
    for (meta, data) in cases {
        let res = store.store_frame_snapshot(
            1000,
            "PasswordFailed",
            &EvidenceFrame {
                metadata: meta.clone(),
                data: &data,
            },
            Some("2026-09-30"),
            None,
        );
        assert!(
            matches!(res, Err(EvidenceStoreError::InvalidFrame(_))),
            "{meta:?} with {} bytes must be refused, got {res:?}",
            data.len()
        );
    }
    assert!(
        store
            .list_snapshots_for_date("2026-09-30")
            .unwrap()
            .is_empty(),
        "refused frames must not produce files"
    );
    assert_eq!(
        store.daily_count(1000, "2026-09-30"),
        0,
        "refused frames must not consume the daily cap"
    );
}

/// Byte-for-byte shape of the pre-#181 record (no version, no frame metadata).
#[derive(Serialize)]
struct LegacyRecord {
    snapshot_id: String,
    uid: u32,
    timestamp: u64,
    reason: String,
    image_data: Vec<u8>,
}

fn write_encrypted_cbor<T: Serialize>(
    key: &MasterKey,
    dir: &std::path::Path,
    value: &T,
) -> std::path::PathBuf {
    let mut cbor = Vec::new();
    ciborium::into_writer(value, &mut cbor).unwrap();
    let ciphertext = encrypt_payload(key, &cbor).unwrap();
    fs::create_dir_all(dir).unwrap();
    let path = dir.join("crafted.enc");
    fs::write(&path, ciphertext).unwrap();
    path
}

#[test]
fn test_legacy_v1_record_still_decodes() {
    let temp = TempDir::new().unwrap();
    let (store, key) = open_store(&temp);
    let legacy = LegacyRecord {
        snapshot_id: "4b5f8e32-0000-4000-8000-9f8c12a45b67".to_string(),
        uid: 1000,
        timestamp: 1_726_315_000,
        reason: "PasswordFailed".to_string(),
        image_data: vec![1, 2, 3, 4],
    };
    let path = write_encrypted_cbor(&key, &temp.path().join("legacy"), &legacy);
    let record = store.load_snapshot(&path).unwrap();
    assert_eq!(record.format_version, LEGACY_EVIDENCE_RECORD_VERSION);
    assert_eq!(LEGACY_EVIDENCE_RECORD_VERSION, 1);
    assert_eq!(record.frame, None);
    assert_eq!(record.uid, 1000);
    assert_eq!(record.image_data, vec![1u8, 2, 3, 4]);
    assert!(
        matches!(record.to_rgb24(), Err(EvidenceStoreError::InvalidFrame(_))),
        "a legacy record has no dimensions and cannot be decoded"
    );
}

#[derive(Serialize)]
struct CraftedRecord {
    format_version: u16,
    snapshot_id: String,
    uid: u32,
    timestamp: u64,
    reason: String,
    frame: Option<FrameMetadata>,
    image_data: Vec<u8>,
}

#[test]
fn test_unknown_version_and_contradictory_metadata_are_refused() {
    let temp = TempDir::new().unwrap();
    let (store, key) = open_store(&temp);

    let future = CraftedRecord {
        format_version: 99,
        snapshot_id: "id".to_string(),
        uid: 1000,
        timestamp: 0,
        reason: "PasswordFailed".to_string(),
        frame: None,
        image_data: vec![0u8; 4],
    };
    let path = write_encrypted_cbor(&key, &temp.path().join("future"), &future);
    assert!(matches!(
        store.load_snapshot(&path),
        Err(EvidenceStoreError::InvalidFrame(_))
    ));

    let contradictory = CraftedRecord {
        format_version: EVIDENCE_RECORD_VERSION,
        snapshot_id: "id".to_string(),
        uid: 1000,
        timestamp: 0,
        reason: "PasswordFailed".to_string(),
        frame: Some(metadata(4, 4, EvidencePixelFormat::Rgb24)),
        image_data: vec![0u8; 5],
    };
    let path = write_encrypted_cbor(&key, &temp.path().join("contradictory"), &contradictory);
    assert!(matches!(
        store.load_snapshot(&path),
        Err(EvidenceStoreError::InvalidFrame(_))
    ));

    let v2_without_metadata = CraftedRecord {
        format_version: EVIDENCE_RECORD_VERSION,
        snapshot_id: "id".to_string(),
        uid: 1000,
        timestamp: 0,
        reason: "PasswordFailed".to_string(),
        frame: None,
        image_data: vec![0u8; 5],
    };
    let path = write_encrypted_cbor(&key, &temp.path().join("nometa"), &v2_without_metadata);
    assert!(
        store.load_snapshot(&path).is_ok(),
        "a v2 record without frame metadata is an opaque legacy-API payload"
    );
}

#[test]
fn test_load_snapshot_refuses_oversized_file() {
    let temp = TempDir::new().unwrap();
    let (store, _) = open_store(&temp);
    let path = temp.path().join("huge.enc");
    let file = fs::File::create(&path).unwrap();
    file.set_len(MAX_EVIDENCE_FILE_BYTES + 1).unwrap(); // sparse, no real allocation
    assert!(matches!(
        store.load_snapshot(&path),
        Err(EvidenceStoreError::CorruptPayload(_))
    ));
}

#[test]
fn test_record_debug_never_prints_frame_bytes() {
    let temp = TempDir::new().unwrap();
    let (store, _) = open_store(&temp);
    let data = vec![0xABu8; 12];
    let stored = store
        .store_frame_snapshot(
            1000,
            "PasswordFailed",
            &EvidenceFrame {
                metadata: metadata(2, 2, EvidencePixelFormat::Rgb24),
                data: &data,
            },
            None,
            None,
        )
        .unwrap();
    let record = store.load_snapshot(&stored.path).unwrap();
    let text = format!("{record:?}");
    assert!(text.contains("1000"), "{text}");
    assert!(
        !text.contains("171, 171") && !text.contains("0xab") && !text.contains("AB, AB"),
        "frame bytes must never reach Debug output: {text}"
    );
    let frame_text = format!(
        "{:?}",
        EvidenceFrame {
            metadata: metadata(2, 2, EvidencePixelFormat::Rgb24),
            data: &data,
        }
    );
    assert!(!frame_text.contains("171, 171"), "{frame_text}");
}

#[test]
fn test_large_frame_roundtrip_beyond_decoder_scratch_buffer() {
    let temp = TempDir::new().unwrap();
    let (store, _) = open_store(&temp);
    let data: Vec<u8> = (0..320u32 * 240 * 2).map(|i| (i % 251) as u8).collect();
    let stored = store
        .store_frame_snapshot(
            1000,
            "PasswordFailed",
            &EvidenceFrame {
                metadata: metadata(320, 240, EvidencePixelFormat::Yuyv),
                data: &data,
            },
            None,
            None,
        )
        .unwrap();
    let record = store.load_snapshot(&stored.path).unwrap();
    assert_eq!(record.image_data, data);
    assert_eq!(record.to_rgb24().unwrap().len(), 320 * 240 * 3);
    let on_disk = fs::metadata(&stored.path).unwrap().len();
    assert!(
        on_disk < (data.len() as u64) + 1024,
        "v2 payload must be a compact CBOR byte string, file is {on_disk} bytes"
    );
}
