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
│   ├── 9c3b12ef-...-38a4d701e56b.opaque.enc # Mode 0600, opaque payload (store_snapshot API)
│   ├── 1f07aa3c-...-6d2e90b1c4f5.webp.enc   # Mode 0600, legacy opaque file (older releases)
│   └── .daily_count.1000                    # Mode 0600, persisted daily cap counter (GitHub #234)
└── /var/lib/soos/evidence.key               # Mode 0600 (-rw-------)
```

### 3.2 Wire Payload Layout

Every snapshot written since GitHub #266 uses the AAD-bound envelope (payload format version 2,
`crypto::PAYLOAD_FORMAT_VERSION`, distinct from the CBOR record version):

| Offset | Length | Field | Description |
|---|---|---|---|
| `0..8` | 8 bytes | `MAGIC_HEADER` | ASCII magic constant `b"SOOSEVD1"` (unchanged) |
| `8..12` | 4 bytes | `BOUND_FORMAT_MARKER` | `b"AAD\x02"`: AAD-bound format, version 2 |
| `12..24` | 12 bytes | `Nonce` | Unique CSPRNG 96-bit initialization vector |
| `24..N-16` | Variable | `Ciphertext` | AES-256-GCM encrypted CBOR payload (`EvidenceRecord`) |
| `N-16..N` | 16 bytes | `Tag` | GCM 128-bit authentication tag |

Snapshots written by earlier releases use the legacy unbound envelope (format version 1):
`MAGIC_HEADER (0..8) || Nonce (8..20) || Ciphertext || Tag`, with no associated data
(`crypto::encrypt_payload` / `decrypt_payload`; the store never writes it).

**Associated Data (GitHub #266, STO-22)**: `crypto::snapshot_aad(date, snapshot_id)` =
`b"soos/evidence-snapshot" || 0x00 || MAGIC_HEADER || BOUND_FORMAT_MARKER || len(date) || date ||
len(id) || id` (lengths as big-endian `u64`). `load_snapshot(path)` derives the date from the
parent directory name and the id from the file name up to its first `.`, so a snapshot moved to
another date partition or renamed to another id fails authentication (`Crypto`). The file suffix is
not bound: the historical `.webp.enc` rename stays readable. Legacy unbound snapshots are still
decrypted (`PayloadFormat::LegacyV1`); the capture and read paths never rewrite them, and they
expire with the 7-day retention unless an operator migrates them first (below). Not detected (rollback): restoring an older
snapshot file at its own path, deleting snapshots or daily counters, restoring a whole partition,
and moving a legacy (unbound) snapshot. Root attackers are out of scope; ADR 2026-09-30 "AES-GCM
Associated Data for Stored Templates and Evidence".

**Legacy migration (GitHub #287, owner decision 2026-10-01)**:
`EvidenceStore::migrate_legacy_snapshots(dry_run)` re-encrypts every legacy snapshot with the bound
envelope, run by the operator through `soos-enroll migrate [--dry-run]` (`Docs/ENROLLMENT_CLI.md`).
It holds the same exclusive `flock` on the base directory as `rotate_retention`, walks the real
`YYYY-MM-DD` partitions and their `*.enc` files (symlinked partitions and files are skipped, never
followed), opens each file with `O_NOFOLLOW`, bounds it by `MAX_EVIDENCE_FILE_BYTES`,
authenticates it against its path binding and decodes its record. A legacy record must carry the
snapshot id of its file name (a renamed legacy file is refused rather than bound to a new id); the
date partition of a legacy file cannot be checked and is bound as found. The decrypted CBOR is
re-sealed byte for byte into a temporary file created exclusively with mode `0600` and synced,
which replaces the original by `rename` only if the path still holds the inode that was read; the
partition is then synced. Bound files are never rewritten, daily counters are not touched, and a
per-file failure is recorded in `SnapshotMigrationReport::failed` (path and error message) and
leaves the file untouched while the other files are still processed. A missing base directory
yields an empty report and nothing is created; a symlinked base directory is refused
(`InvalidPath`). Like a replaced template, the superseded legacy inode is overwritten in place
(best effort: 3 CSPRNG passes, each `fsync`-ed, through a write handle opened on the same inode
before the rename) once the bound file is committed; a dry run never overwrites anything. If that
overwrite fails the snapshot is already migrated and the error is reported for that file (a later
run reports it as already current). The overwrite does not reach the physical blocks on
copy-on-write or journaling filesystems, snapshots, backups or flash media; the guarantee remains
the encryption at rest under the evidence key. `MasterKey::load_existing(path)` loads an existing
evidence key with the same validation as `load_or_create` and never creates one.

File names do not describe an image encoding: the store never encodes WebP or JPEG.
`FRAME_SNAPSHOT_EXTENSION` (`.frame.enc`) marks self-describing camera frames written by
`store_frame_snapshot`; `OPAQUE_SNAPSHOT_EXTENSION` (`.opaque.enc`) marks files written by the
opaque `store_snapshot(&[u8])` API, whose payload is whatever the caller passed. Older releases
named those files `.webp.enc`; such legacy files are still listed, counted, decrypted and purged
(every `*.enc` file matches). `list_snapshots_for_date` returns all of them.

### 3.3 Record Format (CBOR inside the envelope)

| Field | Type | Notes |
|---|---|---|
| `format_version` | `u16` | `EVIDENCE_RECORD_VERSION` = 2; absent in pre-#181 records, which decode as `LEGACY_EVIDENCE_RECORD_VERSION` = 1 |
| `snapshot_id` | text | UUID v4 |
| `uid` | `u32` | target user |
| `timestamp` | `u64` | wall-clock capture time, Unix seconds |
| `reason` | text | `PasswordFailed` (password failure event) or `PadFailed` (capture that vetoed an `Auth` request as a presentation attack, GitHub #261) |
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

Migration: no rewrite is needed. Existing legacy `.webp.enc` files keep decrypting with the same
key (version 1, `frame == None`); new daemon snapshots are written as `.frame.enc` version 2
records and new opaque snapshots as `.opaque.enc`.
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
and `daily_count` / `daily_total` report `0` for any date that is not the tracked day. When
the store switches to a date, the global total is re-derived from the snapshot files already
in that partition and the per-UID cap is always read from the persisted counter (§4.3), so a
wall clock stepped back across midnight does **not** reopen a budget whose partition still
exists (see §4.3.1); the on-disk retention rotation is unaffected.

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

// Store snapshot on authentication failure (the daemon does this on `PasswordFailed`,
// and with reason `PadFailed` for the capture that vetoed a request as a spoof)
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
metadata (`.opaque.enc`, version 2 record with `frame == None`; legacy files use `.webp.enc`).

### 4.3 Daily Cap Persistence (GitHub #234)

The per-UID daily cap is enforced from a counter persisted in the date partition,
`YYYY-MM-DD/.daily_count.<uid>` (`DAILY_COUNT_FILE_PREFIX`, decimal `u32`, mode `0600`,
written with an exclusive temporary file, `fsync` and `rename`). Consequences:

- The cap holds across a daemon restart: a new `EvidenceStore` on the same directory reads
  the consumed quota back (`daily_count` reads the same file).
- Nothing grows with the daemon lifetime: the only in-memory counters are those of the
  tracked day (bounded by the global cap, §4.1.1). The counter file disappears with its
  partition when `rotate_retention` prunes it, and pruning the tracked day also resets the
  in-memory view.
- A slot is consumed only after the snapshot is renamed into place. A failed write or rename
  removes the temporary file and leaves the counter unchanged; if the counter itself cannot
  be persisted, the new snapshot is removed and the error is returned.
- The cap checks, the snapshot write and the counter updates form one critical section per
  store (the `daily_counts` mutex), so concurrent writers never exceed either cap.
- The counter is opened with `O_NOFOLLOW | O_NONBLOCK`, must be a regular file of at most
  16 bytes and hold a decimal count. A symlinked, oversized or unparseable counter fails
  closed (`Io` / `CorruptPayload`) and nothing is written. The file never ends in `.enc`, so
  `list_snapshots_for_date` never lists it.

### 4.3.1 Combined Per-UID and Global Caps (GitHub #234 and #276)

Both caps are enforced in one critical section, in this order, before anything is written:

1. The in-memory view switches to the target date if needed; its global total starts from
   the number of `*.enc` snapshot files already in the partition (persisted, survives a
   restart, counts only stored files).
2. The per-UID count is read from `YYYY-MM-DD/.daily_count.<uid>`; `>= daily_cap_per_uid`
   returns `DailyCapExceeded`.
3. The global total `>= daily_cap_total` returns `GlobalDailyCapExceeded` without touching
   the per-UID counter.
4. The snapshot is written and renamed, then the per-UID counter is persisted; only then are
   the per-UID entry and the global total of the in-memory view incremented. Any failure
   leaves both quotas unconsumed.

### 4.4 Evidence Key File (GitHub #230)

`MasterKey::load_or_create` opens an existing key with `O_NOFOLLOW | O_NONBLOCK` (the
`symlink_metadata` pre-check is no longer the only symlink defense) and validates the open
descriptor: regular file, owned by root or by the effective UID, no group or world bit
(`mode & 0o077 == 0`), exactly 32 bytes; at most 33 bytes are read. Any violation is
`EvidenceStoreError::KeyError` and the file is left unchanged. Missing parent directories are
created with `KEY_PARENT_DIR_MODE` (`0755`, the `/var/lib/soos` contract); an existing parent
is never chmod-ed.

### 4.5 Retention Rotation

```rust
// Runs periodically (e.g. daily daemon task):
let report = store.rotate_retention("2026-09-14")?;
println!("Pruned {} expired date directories", report.directories_pruned);
```

### 4.6 Orphaned Temporary File Sweep (GitHub #291)

`soos-daemon` does not await an evidence write still running when its shutdown drain budget
is exhausted (ADR 2026-10-01 "Daemon Exit Bounded by the Shutdown Drain Budget"), so the
process can exit between the `create_new` of a temporary file and its rename.
`EvidenceStore::sweep_orphaned_temp_files()` removes such leftovers; the daemon calls it once
at startup (`pipeline::sweep_orphaned_store_temp_files`, which logs counts only and never fails
the startup).

- **Scope**: only real `YYYY-MM-DD` partition directories of the base directory (symlinked or
  misnamed entries skipped, never followed), and inside them only the exact names this store
  creates: `.tmp.<uuid>.<pid>.<16 hex>` (snapshot write), `.tmp.migrate.<uuid>.<pid>.<16 hex>`
  (migration) and `.tmp.daily_count.<uid>.<pid>.<16 hex>` (daily counter; `uid` ≤
  `MAX_VALID_UID`), with a lowercase hyphenated UUID, canonical decimal numbers and lowercase hex.
- **Removal rule**: a regular file with one link, owned by root or the effective UID, modified at
  least `TEMP_SWEEP_MIN_AGE` (60 s) ago (a future time is never old). `fstatat` /
  `unlinkat` run relative to the partition descriptor (`O_DIRECTORY | O_NOFOLLOW`) with
  `AT_SYMLINK_NOFOLLOW`; snapshots, counters and every other name are never touched.
- **Bounds**: at most `MAX_TEMP_SWEEP_REMOVALS` (256) removals and 65 536 examined entries per
  call (`TempSweepReport::limit_reached`; the next start continues).
- **Exclusion**: writers of this process by the daily-counter mutex; retention and migration by
  the base-directory `flock`, tried once and never waited for (`TempSweepReport::lock_busy`).
- **No side effect when unused**: a disabled store or a missing base directory returns an empty
  report and creates nothing; a symlinked base directory is refused with `InvalidPath`.

---

## 5. Security & Invariant Verification

- **Criterion E1 (Opt-in)**: Verified by `tests/opt_in_tests.rs`.
- **Criterion E2 (Retention Rotation)**: Verified by `tests/retention_tests.rs`.
- **Criterion E3 (Daily Cap)**: Verified by `tests/daily_cap_tests.rs`.
- **Criteria SRK4–SRK6 (Persisted Daily Cap, GitHub #234)**: Verified by `tests/daily_cap_persistence_tests.rs`.
- **Criteria SRK2–SRK3 (Key File Validation, GitHub #230)**: Verified by `tests/key_hardening_tests.rs` and `crypto::key_open_tests::test_230_key_open_does_not_follow_symlinks`.
- **Criterion E4 (Permissions & Encryption)**: Verified by `tests/permissions_tests.rs` and `tests/encryption_tests.rs`.
- **Criterion E5 (Zero Network Transmission)**: Verified by `tests/zero_network_tests.rs` and workspace invariant test `test_evidence_store_has_no_network_dependencies`.
- **Criterion E6 (Symlink Traversal Prevention)**: Verified by `tests/safety_hardening_tests.rs::test_evidence_store_rejects_symlink_date_directory`.
- **Criterion E7 (Concurrent Rotation File Locking)**: Verified by `tests/safety_hardening_tests.rs::test_concurrent_rotation_does_not_corrupt`.
- **Criterion E8 (POSIX UID Bounds Validation)**: Verified by `tests/safety_hardening_tests.rs::test_evidence_store_rejects_path_traversal_uid`.
- **Criteria ESF1–ESF5 (Self-Describing Frames, GitHub #181)**: Verified by `tests/frame_format_tests.rs` and `crates/daemon/tests/pipeline_integration_tests.rs::test_181_password_failed_snapshot_records_frame_metadata`.
- **Criterion SGU5 (Orphaned Temporary File Sweep, GitHub #291)**: Verified by `tests/temp_sweep_tests.rs`.
