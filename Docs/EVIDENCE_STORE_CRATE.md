# `soos-evidence-store` Crate Documentation

## 1. Overview & Purpose

The `soos-evidence-store` crate provides local, authenticated, AES-256-GCM encrypted persistence for anti-intrusion camera snapshots in the `soos` Linux Biometric PAM monorepo. It serves privileged daemon components to capture, store, rotate, and manage anti-intrusion evidence when facial verification fails or an anomaly is detected, while enforcing zero-trust privacy and security invariants:
- **Strictly Opt-In**: Disabled by default in daemon configuration (`EvidenceConfig.enabled == false`).
- **Authenticated Encryption at Rest**: AES-256-GCM authenticated cipher with 128-bit authentication tags and `SOOSEVD1` magic header.
- **Unique Nonces**: Fresh 96-bit CSPRNG nonces generated on every snapshot capture.
- **Strict POSIX Permissions**: Snapshot files written with mode `0600` (`-rw-------`) and date partition directories with mode `0700` (`drwx------`).
- **Atomic Operations & Mode 0600 Inception**: Files are created atomically with mode `0600` using `create_new(true)` (`O_CREAT | O_EXCL`) with CSPRNG nonce salt in temporary names, followed by `sync_all` and atomic rename.
- **Symlink Traversal Prevention**: Pre-creation and pre-write `symlink_metadata` inspections reject symbolic links targeting base or date directories, blocking symlink redirection attacks.
- **Concurrent Rotation File Locking**: Retention rotation synchronizes across threads and daemon instances via exclusive RAII `nix::fcntl::Flock` on the evidence base directory.
- **POSIX UID Bounds Validation**: Validates user identifiers against `MAX_VALID_UID = 2_147_483_647`, rejecting negative values cast to unsigned (sign bit set) and sentinel invalid UIDs (`(uid_t)-1`).
- **Automated 7-Day Retention Rotation**: Scheduled cleanup purging dated directories older than the configured retention threshold (default: 7 days).
- **Per-UID Daily Quotas**: Configurable daily cap (default: 10 snapshots per UID per day) to prevent disk exhaustion attacks.
- **Zero Network Transmission**: Crate contains zero network dependencies (`std::net`, `tokio::net`, `reqwest`), strictly confining evidence to local disk.
- **Memory Zeroization**: Cryptographic keys implement `Zeroize` and `ZeroizeOnDrop`; decrypted plaintext buffers, the CBOR plaintext built before encryption and the reconstructed RGB image use `zeroize::Zeroizing`; the frame payload of an `EvidenceRecord` (`FrameBytes`) is zeroized on drop and redacted from `Debug`.
- **Self-Describing Frames (GitHub #181, review finding STO-08)**: camera frames are stored with their width, height, pixel format, monotonic capture time and sequence inside the authenticated envelope (record version 2), so any snapshot can be decoded without out-of-band knowledge.

---

## 2. Architecture & Modules

```text
crates/evidence-store/
├── Cargo.toml
└── src/
    ├── lib.rs          # #![forbid(unsafe_code)], public re-exports
    ├── error.rs        # EvidenceStoreError enum using thiserror
    ├── config.rs       # EvidenceConfig with defaults (enabled = false)
    ├── crypto.rs       # AES-256-GCM encryption/decryption, MasterKey, CSPRNG nonces
    ├── frame.rs        # FrameMetadata, EvidencePixelFormat, FrameBytes, bounds, RGB reconstruction
    ├── snapshot.rs     # EvidenceRecord (versioned), UUID v4 generation, Gregorian affine calendar math
    └── store.rs        # EvidenceStore engine, retention rotation, daily cap enforcement
```

---

## 3. Storage Layout & Wire Format

### 3.1 Directory and File Structure

Evidence snapshots are partitioned by calendar date under `/var/lib/soos/evidence/`:
```text
/var/lib/soos/evidence/
├── 2026-09-13/                              # Mode 0700 (drwx------)
│   ├── 4b5f8e32-...-9f8c12a45b67.frame.enc  # Mode 0600, camera frame (record v2 + metadata)
│   └── d8a1c490-...-01e4b9347892.frame.enc  # Mode 0600 (-rw-------)
├── 2026-09-14/                              # Mode 0700 (drwx------)
│   └── 9c3b12ef-...-38a4d701e56b.webp.enc   # Mode 0600, opaque payload (legacy API / pre-#181)
└── /var/lib/soos/evidence.key               # Mode 0600 (-rw-------)
```

### 3.2 Wire Payload Layout

Every encrypted snapshot file follows a strict binary layout:

| Offset | Length | Field | Description |
|---|---|---|---|
| `0..8` | 8 bytes | `MAGIC_HEADER` | ASCII magic constant `b"SOOSEVD1"` |
| `8..20` | 12 bytes | `Nonce` | Unique CSPRNG 96-bit initialization vector |
| `20..N-16` | Variable | `Ciphertext` | AES-256-GCM encrypted CBOR payload (`EvidenceRecord`) |
| `N-16..N` | 16 bytes | `Tag` | Poly1305 / GCM 128-bit authentication tag |

File names do not describe an image encoding: the store never encodes WebP or JPEG.
`FRAME_SNAPSHOT_EXTENSION` (`.frame.enc`) marks self-describing camera frames written by
`store_frame_snapshot`; `OPAQUE_SNAPSHOT_EXTENSION` (`.webp.enc`) is kept only for the opaque
`store_snapshot(&[u8])` API and for files written before GitHub #181 (the historical name is
retained for backward compatibility of that API and its contract tests; the payload is whatever
the caller passed, not WebP). `list_snapshots_for_date` returns both.

### 3.3 Record Format (CBOR inside the envelope)

| Field | Type | Notes |
|---|---|---|
| `format_version` | `u16` | `EVIDENCE_RECORD_VERSION` = 2; absent in pre-#181 records, which decode as `LEGACY_EVIDENCE_RECORD_VERSION` = 1 |
| `snapshot_id` | text | UUID v4 |
| `uid` | `u32` | target user |
| `timestamp` | `u64` | wall-clock capture time, Unix seconds |
| `reason` | text | e.g. `PasswordFailed` |
| `frame` | map or absent | `FrameMetadata { width, height, pixel_format, captured_at_mono_ns, sequence }`; absent for opaque and legacy records |
| `image_data` | byte string | raw payload exactly as captured (legacy records: array of integers, still accepted) |

`pixel_format` is serialized as its V4L2 fourcc: `GREY` (1 byte/pixel), `RGB3` (3), `YUYV`
(2, even width), `NV12` (1.5, even width and height) or `MJPG` (compressed, stored verbatim).
Because the whole record is sealed by AES-256-GCM, the version and the metadata are
authenticated together with the pixels; a modified byte fails decryption.

Bounds and validation:
- `MAX_EVIDENCE_DIMENSION` = 8192 pixels per side, `MAX_EVIDENCE_IMAGE_BYTES` = 32 MiB per
  payload; for uncompressed formats the payload length must equal `width x height x bpp`, MJPEG
  payloads must be non-empty. Invalid frames are refused with
  `EvidenceStoreError::InvalidFrame` **before** the daily cap is consumed or anything is written.
- `load_snapshot` refuses files larger than `MAX_EVIDENCE_FILE_BYTES` (65 MiB) before reading
  them, then validates the decoded record: unknown versions, legacy records carrying metadata,
  and metadata that contradicts the payload length are refused.
- `EvidenceRecord::to_rgb24()` reconstructs a packed RGB 8:8:8 image
  (`width * height * 3` bytes, BT.601 for YUV) from `GREY`, `RGB3`, `YUYV` and `NV12` frames.
  MJPEG frames are returned verbatim in `image_data` for an external decoder; legacy and opaque
  records have no dimensions and return `InvalidFrame`.

Migration: no rewrite is needed. Existing `.webp.enc` files keep decrypting with the same key
(version 1, `frame == None`); new daemon snapshots are written as `.frame.enc` version 2 records.
Retention rotation removes both kinds after the retention window.
---

## 4. Configuration & Usage

### 4.1 Configuration (`EvidenceConfig`)

```rust
use soos_evidence_store::EvidenceConfig;
use std::path::PathBuf;

// Default configuration is strictly opt-in (disabled)
let default_cfg = EvidenceConfig::default();
assert!(!default_cfg.enabled);
assert_eq!(default_cfg.retention_days, 7);
assert_eq!(default_cfg.daily_cap_per_uid, 10);

// Custom configuration enabling storage
let custom_cfg = EvidenceConfig {
    enabled: true,
    base_dir: PathBuf::from("/var/lib/soos/evidence"),
    key_path: PathBuf::from("/var/lib/soos/evidence.key"),
    retention_days: 7,
    daily_cap_per_uid: 10,
};
```

### 4.1.1 Global Daily Cap and Counter Pruning (GitHub #276)

The per-UID cap alone would let `daily_cap_per_uid` snapshots accumulate for every UID a
caller names. `EvidenceStore` therefore also enforces a global cap across all UIDs:
`DEFAULT_DAILY_CAP_TOTAL` (100) snapshots per day, overridable with
`EvidenceStore::with_daily_cap_total(cap)` and, in the daemon, with
`[pipeline.evidence] daily_cap_total` in `/etc/soos/daemon.toml`. Exceeding it returns
`EvidenceStoreError::GlobalDailyCapExceeded { cap, date }` before anything is written, and a
globally refused snapshot does not consume the per-UID quota.

The in-memory counters track a single day: a snapshot for another date resets them to that
date, so the map never holds more entries than the global cap (`tracked_daily_counters()`),
and `daily_count` / `daily_total` report `0` for any date that is not the tracked day. A wall
clock stepped back across midnight therefore starts a fresh day budget; the on-disk
retention rotation is unaffected.

### 4.2 Storing an Anti-Intrusion Snapshot

```rust
use soos_evidence_store::{EvidenceFrame, EvidencePixelFormat, EvidenceStore, FrameMetadata};

let store = EvidenceStore::open(custom_cfg)?;
let frame = EvidenceFrame {
    metadata: FrameMetadata {
        width: 640,
        height: 480,
        pixel_format: EvidencePixelFormat::Yuyv,
        captured_at_mono_ns: 1_234_567_890,
        sequence: 42,
    },
    data: &raw_v4l2_buffer, // 640 * 480 * 2 bytes
};

// Store snapshot on authentication failure (the daemon does this on `PasswordFailed`)
let result = store.store_frame_snapshot(
    1000,                // uid
    "PasswordFailed",    // reason
    &frame,              // pixels + metadata
    None,                // date_override (None for today)
    None,                // timestamp_override (None for now)
)?;

// Later, as root: decode it again
let record = store.load_snapshot(&result.path)?;
let rgb = record.to_rgb24()?; // 640 * 480 * 3 bytes, zeroized on drop
```

`store_snapshot(uid, reason, &bytes, ..)` remains available for opaque payloads without
metadata (`.webp.enc`, version 2 record with `frame == None`).

### 4.3 Retention Rotation

```rust
// Runs periodically (e.g. daily daemon task):
let report = store.rotate_retention("2026-09-14")?;
println!("Pruned {} expired date directories", report.directories_pruned);
```

---

## 5. Security & Invariant Verification

- **Criterion E1 (Opt-in)**: Verified by `tests/opt_in_tests.rs`.
- **Criterion E2 (Retention Rotation)**: Verified by `tests/retention_tests.rs`.
- **Criterion E3 (Daily Cap)**: Verified by `tests/daily_cap_tests.rs`.
- **Criterion E4 (Permissions & Encryption)**: Verified by `tests/permissions_tests.rs` and `tests/encryption_tests.rs`.
- **Criterion E5 (Zero Network Transmission)**: Verified by `tests/zero_network_tests.rs` and workspace invariant test `test_evidence_store_has_no_network_dependencies`.
- **Criterion E6 (Symlink Traversal Prevention)**: Verified by `tests/safety_hardening_tests.rs::test_evidence_store_rejects_symlink_date_directory`.
- **Criterion E7 (Concurrent Rotation File Locking)**: Verified by `tests/safety_hardening_tests.rs::test_concurrent_rotation_does_not_corrupt`.
- **Criterion E8 (POSIX UID Bounds Validation)**: Verified by `tests/safety_hardening_tests.rs::test_evidence_store_rejects_path_traversal_uid`.
- **Criteria ESF1–ESF5 (Self-Describing Frames, GitHub #181)**: Verified by `tests/frame_format_tests.rs` and `crates/daemon/tests/pipeline_integration_tests.rs::test_181_password_failed_snapshot_records_frame_metadata`.
