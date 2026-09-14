# `soos-biometric-store` Crate Documentation

## 1. Overview & Purpose

The `soos-biometric-store` crate provides secure, hardware-free, AES-256-GCM encrypted persistence for biometric facial templates in the `soos` Linux Biometric PAM monorepo. It serves privileged components (`soos-daemon`, `enrollment-cli`) to persist, retrieve, update, and delete enrolled biometric identity templates while upholding strict security and zero-trust invariants:
- **Authenticated Encryption at Rest**: AES-256-GCM authenticated cipher with 128-bit authentication tags.
- **Unique Nonces**: Unique 96-bit cryptographically secure pseudorandom nonces generated on every write.
- **Strict POSIX Permissions**: Template files written with mode `0600` (`-rw-------`) and directories with mode `0700` (`drwx------`).
- **Atomic File Operations**: Safe write-to-temporary (`.tmp`) followed by `fsync` and atomic POSIX rename to prevent incomplete reads.
- **Model Migration Support**: Explicit tracking of `model_id`, `model_version`, `enrollment_timestamp`, and `embedding_dim`.
- **Memory Zeroization**: Plaintext embeddings stored in `zeroize::Zeroizing<Vec<f32>>` and master keys zeroized on drop.
- **Fail-Closed Safety**: `#![forbid(unsafe_code)]` with robust `BiometricStoreError` handling.

---

## 2. Architecture & Modules

```text
crates/biometric-store/
├── Cargo.toml
└── src/
    ├── lib.rs          # #![forbid(unsafe_code)], public re-exports
    ├── error.rs        # BiometricStoreError enum using thiserror
    ├── crypto.rs       # AES-256-GCM encryption/decryption, MasterKey, CSPRNG nonces
    ├── template.rs     # BiometricTemplate schema, validation, CBOR serialization
    └── store.rs        # BiometricStore CRUD operations, atomic writes, POSIX permissions
```

---

## 3. Storage Layout & Wire Format

### 3.1 Directory and File Paths

Biometric templates are stored in a dedicated system directory, defaulting to `/var/lib/soos/biometrics/`:
```text
/var/lib/soos/biometrics/
├── 1000.cbor.enc    # Mode 0600, owner root:root
├── 1001.cbor.enc    # Mode 0600, owner root:root
└── master.key       # Optional master key file (Mode 0600)
```

### 3.2 Wire Payload Layout

Every encrypted template file on disk follows a strict binary layout:

| Offset | Length | Field | Description |
|---|---|---|---|
| `0..8` | 8 bytes | `MAGIC_HEADER` | ASCII magic constant `b"SOOSBIO1"` |
| `8..20` | 12 bytes | `Nonce` | Unique CSPRNG 96-bit initialization vector |
| `20..N-16` | Variable | `Ciphertext` | AES-256-GCM encrypted CBOR payload |
| `N-16..N` | 16 bytes | `Tag` | Poly1305 / GCM 128-bit authentication tag |

---

## 4. Public API & Usage

### 4.1 Master Key Management

```rust
use soos_biometric_store::MasterKey;

// Generate a fresh 256-bit key from kernel CSPRNG
let key = MasterKey::generate()?;

// Or persist/load from a protected key file (mode 0600)
let key = MasterKey::load_or_create("/var/lib/soos/master.key")?;
```

### 4.2 Biometric Store Operations (CRUD)

```rust
use soos_biometric_store::{BiometricStore, BiometricTemplate, MasterKey};
use zeroize::Zeroizing;

let key = MasterKey::generate()?;
let store = BiometricStore::new("/var/lib/soos/biometrics", key)?;

// 1. Create (Enroll)
let template = BiometricTemplate::new(
    1000,
    "glintr100".to_string(),
    "1.0.0".to_string(),
    1726000000,
    Zeroizing::new(vec![0.05_f32; 512]),
)?;
store.enroll(&template)?;

// 2. Read (Get)
if let Some(loaded) = store.get(1000)? {
    println!("Loaded UID {} with dim {}", loaded.uid, loaded.embedding_dim);
}

// 3. Check Existence
assert!(store.exists(1000)?);

// 4. List Enrolled UIDs
let uids = store.list_enrolled()?; // vec![1000]

// 5. Delete
assert!(store.delete(1000)?);
```

---

## 5. Verification & Security Matrix

| Criterion | Requirement | Test Suite | Status |
|---|---|---|---|
| **B1** | Embeddings encrypted at rest with AES-GCM | `tests/encryption_tests.rs`, `tests/proptest_suite.rs` | ☑ Validated |
| **B2** | File mode `0600`, directory mode `0700`, atomic writes | `tests/permissions_tests.rs` | ☑ Validated |
| **B3** | Model metadata tracked for migration | `tests/metadata_tests.rs` | ☑ Validated |
| **B4** | Full CRUD operations tested | `tests/crud_tests.rs` | ☑ Validated |
