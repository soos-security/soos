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
- **Memory Zeroization**: Cryptographic keys implement `Zeroize` and `ZeroizeOnDrop`, and decrypted plaintext buffers use `zeroize::Zeroizing`.

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
    ├── snapshot.rs     # EvidenceRecord, UUID v4 generation, Gregorian affine calendar math
    └── store.rs        # EvidenceStore engine, retention rotation, daily cap enforcement
```

---

## 3. Storage Layout & Wire Format

### 3.1 Directory and File Structure

Evidence snapshots are partitioned by calendar date under `/var/lib/soos/evidence/`:
```text
/var/lib/soos/evidence/
├── 2026-09-13/                              # Mode 0700 (drwx------)
│   ├── 4b5f8e32-...-9f8c12a45b67.webp.enc   # Mode 0600 (-rw-------)
│   └── d8a1c490-...-01e4b9347892.webp.enc   # Mode 0600 (-rw-------)
├── 2026-09-14/                              # Mode 0700 (drwx------)
│   └── 9c3b12ef-...-38a4d701e56b.webp.enc   # Mode 0600 (-rw-------)
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

### 4.2 Storing an Anti-Intrusion Snapshot

```rust
use soos_evidence_store::EvidenceStore;

let store = EvidenceStore::open(custom_cfg)?;
let frame_bytes = vec![/* JPEG/WebP camera frame bytes */];

// Store snapshot on authentication failure
let result = store.store_snapshot(
    1000,                // uid
    "failed_face_match", // reason
    &frame_bytes,        // frame buffer
    None,                // date_override (None for today)
    None,                // timestamp_override (None for now)
)?;

println!("Snapshot saved: {}", result.path.display());
```

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
