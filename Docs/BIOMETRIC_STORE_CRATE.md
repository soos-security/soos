# `soos-biometric-store` Crate Documentation

## 1. Overview & Purpose

The `soos-biometric-store` crate provides secure, hardware-free, AES-256-GCM encrypted persistence for biometric facial templates in the `soos` Linux Biometric PAM monorepo. It serves privileged components (`soos-daemon`, `enrollment-cli`) to persist, retrieve, update, and delete enrolled biometric identity templates while upholding strict security and zero-trust invariants:
- **Authenticated Encryption at Rest**: AES-256-GCM authenticated cipher with 128-bit authentication tags.
- **Unique Nonces**: Unique 96-bit cryptographically secure pseudorandom nonces generated on every write.
- **Strict POSIX Permissions**: Template files written with mode `0600` (`-rw-------`); a store directory created by the crate gets mode `0700` (`drwx------`). An existing directory is validated, never chmod-ed (§3.3).
- **Atomic File Operations**: Safe write-to-temporary (`.tmp`) followed by `fsync`, atomic POSIX rename and a directory `fsync` to prevent incomplete reads.
- **Race-free master key creation** (GitHub #303): `MasterKey::load_or_create` publishes a new key with `link(2)` from an exclusive `0600` temporary file; `EEXIST` means another process (daemon, `soos-enroll`, GUI) published first, and its key is read and returned instead of being replaced. The temporary file is unlinked on every path (`key_publication_race_tests.rs`, row SKE1).
- **Template reads never block** (GitHub #312): a template is opened `O_NOFOLLOW | O_NONBLOCK` and must be a regular file on the open descriptor; a FIFO or directory at the template path fails closed with `CorruptFile` (`template_fifo_tests.rs`, row SKE8).
- **Honest Erasure Model**: Encryption at rest plus master-key destruction is the erasure guarantee; in-place overwrite on `delete` and re-`enroll` is best effort (§3.4).
- **Model Migration Support**: Explicit tracking of `model_id`, `model_version`, `enrollment_timestamp`, and `embedding_dim`.
- **Memory Zeroization**: Plaintext embeddings stored in `zeroize::Zeroizing<Vec<f32>>`, the CBOR plaintext returned by `BiometricTemplate::to_cbor` in a pre-reserved `Zeroizing<Vec<u8>>`, and master keys zeroized on drop.
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

Every template written since GitHub #266 uses the AAD-bound envelope (payload format version 2,
`PAYLOAD_FORMAT_VERSION`):

| Offset | Length | Field | Description |
|---|---|---|---|
| `0..8` | 8 bytes | `MAGIC_HEADER` | ASCII magic constant `b"SOOSBIO1"` (unchanged) |
| `8..12` | 4 bytes | `BOUND_FORMAT_MARKER` | `b"AAD\x02"`: AAD-bound format, version 2 |
| `12..16` | 4 bytes | `uid` | Big-endian UID the ciphertext is bound to (clear, authenticated) |
| `16..28` | 12 bytes | `Nonce` | Unique CSPRNG 96-bit initialization vector |
| `28..N-16` | Variable | `Ciphertext` | AES-256-GCM encrypted CBOR payload |
| `N-16..N` | 16 bytes | `Tag` | GCM 128-bit authentication tag |

Templates written by earlier releases use the legacy unbound envelope (format version 1):
`MAGIC_HEADER (0..8) || Nonce (8..20) || Ciphertext || Tag`, with no associated data. The legacy
codec is still exposed as `encrypt_payload` / `decrypt_payload`; the store never writes it.

### 3.2.1 Associated Data Binding and Migration (GitHub #266, STO-22)

Recorded as ADR 2026-09-30 "AES-GCM Associated Data for Stored Templates and Evidence".

- **Associated Data**: `template_aad(uid)` = `b"soos/biometric-template" || 0x00 || MAGIC_HEADER ||
  BOUND_FORMAT_MARKER || uid (big-endian)`. It binds the file role (template), the payload format
  version and the UID. `encrypt_template_payload` / `decrypt_template_payload` implement the codec.
- **Cross-UID swap**: a bound template copied or renamed to another UID's file authenticates under
  the UID in its own header and is then refused with `CorruptFile` by the codec, before any CBOR
  parsing. Rewriting the clear UID breaks the tag (`Crypto`). The plaintext UID check of `get` /
  `get_metadata` is kept as a second layer.
- **Legacy templates are never lost**: a file without the marker is decrypted with the legacy
  codec (`PayloadFormat::LegacyV1`) and the plaintext UID check applies as before. A legacy nonce
  that happens to start with the marker (probability 2^-32) is still read, because decoding falls
  back to the legacy layout when the bound layout does not authenticate.
- **Re-encryption**: the next `enroll` of the UID writes the bound format (atomic replace plus the
  best-effort overwrite of the legacy inode). `BiometricStore::template_format(uid)` reports the
  format, and `BiometricStore::migrate_legacy_template(uid)` re-encrypts a legacy template in place
  without changing its content (it refuses a legacy file whose embedded UID does not match and
  leaves it untouched; it is serialized with `enroll` and `delete` by the store lock below).
- **Bulk migration (GitHub #287, owner decision 2026-10-01)**: `BiometricStore::migrate_legacy_templates(dry_run)`
  runs `migrate_template(uid, dry_run)` (outcome `TemplateMigration::{Missing, AlreadyCurrent,
  Migrated, WouldMigrate}`) for every UID of `list_enrolled` and returns a
  `TemplateMigrationReport` (`migrated`, `already_current`, `failed` with UID and error message).
  Every file is size-bounded, authenticated and fully validated (CBOR template, UID check) in both
  modes; a legacy template is rewritten only through `enroll` (temporary file `0600` with
  `O_NOFOLLOW`, `fsync`, atomic rename, directory `fsync`, best-effort overwrite of the legacy
  inode), a bound template is never rewritten, and a refused file (tampered, foreign UID, symlink,
  I/O error) is recorded and left untouched while the other templates are still processed. A
  second run reports every template as already current. The report never carries embedding values
  or key material. The operator entry point is `soos-enroll migrate [--dry-run]`
  (`Docs/ENROLLMENT_CLI.md`); v1 stays readable without it (no cut-off). `MasterKey::load_existing(path)`
  loads an existing master key with the validation of `load_or_create` and never creates a key or a
  directory; `migrate` uses it so that a host without a key is never given one.
- **Store lock and migration identity check (GitHub #289, ADR 2026-10-01 "Biometric Store
  Lock and Migration Identity Check")**: `enroll`, `delete` (hence `soos-enroll enroll`,
  `import` and `delete`, and the GUI) and a real `migrate_template` take an exclusive advisory
  `flock` on the store directory itself (opened `O_RDONLY | O_DIRECTORY | O_NOFOLLOW`; no lock
  file is created). The lock is held only around the file mutation, never across a camera
  capture; reads (`get`, `get_metadata`, `template_format`, `list_enrolled`) and dry runs never
  take it. A contended lock is retried every 10 ms up to `STORE_LOCK_TIMEOUT` (5 s, configurable
  with `BiometricStore::with_lock_timeout`), then the call fails with
  `BiometricStoreError::LockTimeout` ("... is locked by another enroll, delete, import or
  migrate operation; gave up after N ms") and changes nothing. A migration holds the lock from
  its read to its rewrite, so a concurrent `delete` never sees its template resurrected and a
  concurrent re-enrollment is never overwritten. As a second layer against writers that bypass
  the lock, the device, inode, size and modification time of the file read are recorded; the
  rewrite is refused with `BiometricStoreError::ChangedConcurrently` ("template of UID N changed
  concurrently during migration; left as found") when the write handle or, just before the
  rename, the path no longer matches; the temporary file is removed and the file is left as found
  (a per-file failure of `migrate_legacy_templates`).
- **Local filesystem required for the store lock (GitHub #291)**: the lock is a `flock(2)` on a
  directory descriptor opened `O_RDONLY`. On NFS, Linux emulates `flock` with POSIX byte-range
  locks, and an exclusive byte-range lock needs a descriptor opened for writing, which a
  directory can never be: every `flock(LOCK_EX)` there fails with `EBADF`. `lock_store` maps any
  errno other than `EAGAIN` / `EINTR` to `BiometricStoreError::Io`, so on NFS every `enroll`,
  `delete`, `enroll_if_absent` and real migration fails closed with an I/O error and changes
  nothing (reads still work). The store directory (`/var/lib/soos/biometrics` and any
  `--biometrics-dir` / `--dev-store`) must therefore live on a local filesystem (ext4, xfs,
  btrfs, tmpfs, ...); network filesystems are not supported.
- **Atomic enroll-if-absent (GitHub #291)**: `BiometricStore::enroll_if_absent(&template)`
  takes the store lock, then refuses with `BiometricStoreError::AlreadyEnrolled(uid)` when any
  entry exists at the template path (nothing is written), otherwise writes exactly like
  `enroll`. The check and the write are under the same lock, so a template enrolled by another
  `enroll`, `import` or GUI save after an earlier check of the caller is never replaced.
  `soos-enroll import` without `--yes` writes through it.
- **Orphaned temporary file sweep (GitHub #291)**: `BiometricStore::sweep_orphaned_temp_files()`
  removes the temporary files an interrupted `enroll` left (process killed between the
  `create_new` of `<uid>.tmp.<pid>.<suffix>` and the rename). Candidates are only names of that
  exact form with canonical decimal numbers (`u32`, `u32`, `u64`); a candidate is removed only
  if it is a regular file with one link, owned by root or the effective UID and modified at
  least `TEMP_SWEEP_MIN_AGE` (60 s) ago (a future time is never old). `fstatat` and `unlinkat`
  run relative to the store directory descriptor (`O_DIRECTORY | O_NOFOLLOW`, identity checked
  against the path), with `AT_SYMLINK_NOFOLLOW`; nothing is followed. At most
  `MAX_TEMP_SWEEP_REMOVALS` (256) files are removed and 65 536 entries examined per call
  (`TempSweepReport::limit_reached`). The store lock is tried once and never waited for; while
  another operation holds it, nothing is examined (`TempSweepReport::lock_busy`). `soos-daemon`
  runs it once at startup (`pipeline::sweep_orphaned_store_temp_files`). Master-key temporary
  files (`master.key.tmp.*`, outside the store directory) are not swept.
- **Never-create constructor (GitHub #289)**: `BiometricStore::open_existing(dir, key)` validates
  an existing directory exactly like `new` and reports a missing one as `Io(NotFound)` without
  creating it or any parent. `soos-enroll migrate` opens the template store with it.
- **What is detected**: moving a ciphertext between UIDs, editing the clear header, any bit flip,
  a wrong key.
- **What is not detected (rollback)**: restoring an older, still-valid bound template of the
  **same** UID (same associated data), restoring a legacy template of the same UID, deleting a
  template, or restoring a whole older copy of the directory. Detecting rollback would need a
  monotonic counter kept outside the attacker's reach (TPM NV index or similar); a counter stored
  next to the templates is rolled back with them. Only root can write `/var/lib/soos/biometrics`,
  and root attackers are out of scope (`AI/ARCHITECTURE.md` threat model), so no counter is
  added: the binding is defense in depth.

### 3.3 Store Directory Validation (GitHub #178, STO-04)

`BiometricStore::new(base_dir, key)` never changes the permissions of a directory it did not create:

| Situation | Behavior |
|---|---|
| `base_dir` missing | Missing parents are created with default permissions; the final component is created with `DirBuilder::mode(0o700)` and then set to exactly `0700` (`STORE_DIR_MODE`). |
| `base_dir` exists | Validated with `symlink_metadata`, **never chmod-ed**. Refused with `BiometricStoreError::InvalidPath` if it is a symlink, not a directory, owned by a UID other than `0` or the process effective UID, or has any bit of `FORBIDDEN_STORE_DIR_BITS = 0o022` (group- or world-writable). |
| Accepted existing directory | Keeps its exact mode, including setgid/sticky bits and read/search bits chosen by the administrator. |

Consequence: `soos-enroll --biometrics-dir /tmp ...` fails with an explicit error (mode `1777` is world-writable) instead of silently rewriting `/tmp` to `0700`. The packaged directory `/var/lib/soos/biometrics` (`0700 root:root`) is unaffected.

### 3.4 Erasure Model (GitHub #179, STO-06)

Recorded as ADR 2026-09-30 "Biometric Template Erasure Model" in `AI/DECISIONS.md`.

- **What is guaranteed**: every template on disk is AES-256-GCM ciphertext under the host master key (`/var/lib/soos/master.key`). Any residual copy of a discarded template (stale blocks, journal, snapshot, backup, SSD spare area) is undecryptable without that key. Destroying the master key is the complete erasure of every template ever written under it (crypto-erasure); it is only as strong as the destruction of the 32-byte key file, which is subject to the same filesystem limitations, so hosts that need a hard guarantee must keep `/var/lib/soos` on full-disk encryption (LUKS).
- **What is done in addition (best effort)**: before a template inode is released, its content is overwritten in place with CSPRNG bytes (3 passes, `fsync` after each):
  - `delete(uid)`: overwrite, unlink, then `fsync` of the directory.
  - `enroll(template)` replacing an existing template: a handle on the old inode is opened (regular file only, `O_NOFOLLOW | O_NONBLOCK`, inode re-checked after `open`) **before** the atomic rename; after the rename and the directory `fsync` commit the new template, the old inode is overwritten through that handle. The replace stays atomic: a crash never leaves the user without a template. An overwrite failure is reported as an error even though the new template is already committed.
- **What cannot be guaranteed**: in-place overwrite does not reach the physical blocks on copy-on-write filesystems (btrfs, ZFS), with `data=journal`, LVM/filesystem snapshots or backups, or on flash media with wear levelling (SSD, eMMC, NVMe). This is the same limitation documented for GNU `shred`. User-facing messages therefore say "overwritten (best effort)", never "securely shredded" or "unrecoverable".
- **Rejected alternative**: a per-template data key stored inside the template file adds nothing (deleting the file and deleting the wrapped key are the same operation on the same blocks); a separate per-template key store would move the problem to another file. The master key remains the single crypto-erasure point.

### 3.5 Bounded Reads and Metadata-Only Access (GitHub #235, STO-19)

Recorded as ADR 2026-09-30 "Bounded Template Reads and Metadata-Only Listing".

- `MAX_TEMPLATE_FILE_BYTES` = 64 KiB bounds every template file. `get` and `get_metadata` refuse a larger file with `BiometricStoreError::CorruptFile` from its `fstat` length, before any read, and read through `take(MAX + 1)` so a file that grows after the check is refused too. `enroll` refuses (`InvalidMetadata`) a template whose ciphertext would exceed the bound.
- `get_metadata(uid) -> Option<TemplateMetadata>` authenticates and decrypts like `get`, then deserializes `TemplateMetadata` (`uid`, `model_id`, `model_version`, `enrollment_timestamp`, `embedding_dim`): the embedding array is walked only to count its elements and check they are finite, never stored. Listing callers (`soos-enroll list`, the GUI profile refresh) use it.

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

Key file validation (GitHub #230): an existing key is opened with `O_NOFOLLOW | O_NONBLOCK`
and accepted only when the open descriptor is a regular file owned by root or by the effective
UID, with no group or world permission bit (`mode & 0o077 == 0`), and exactly
`MASTER_KEY_LEN` (32) bytes; at most 33 bytes are read. Anything else is
`BiometricStoreError::KeyError` (a symlink is `InvalidPath` or `Io`) and the file is not
modified: a key leaked to `0644` stops the daemon instead of being used silently. When the key
is created, missing parent directories are created with `KEY_PARENT_DIR_MODE` (`0755`, the
`/var/lib/soos` contract that keeps `/var/lib/soos/models` traversable); an existing parent is
never chmod-ed. The key file itself is created `0600` with `O_EXCL` and renamed into place.

### 4.2 Biometric Store Operations (CRUD)

```rust
use soos_biometric_store::{BiometricStore, BiometricTemplate, MasterKey};
use zeroize::Zeroizing;

let key = MasterKey::generate()?;
let store = BiometricStore::new("/var/lib/soos/biometrics", key)?;

// 1. Create (Enroll)
let template = BiometricTemplate::new(
    1000,
    "sface_2021dec".to_string(), // SHIPPED_EMBEDDING_MODEL.model_id
    "2.0.0".to_string(),
    1726000000,
    Zeroizing::new(vec![0.05_f32; 128]), // SHIPPED_EMBEDDING_MODEL.dimension
)?;
store.enroll(&template)?;

// 2. Read (Get)
if let Some(loaded) = store.get(1000)? {
    println!("Loaded UID {} with dim {}", loaded.uid, loaded.embedding_dim);
}

// 2b. Read metadata only (no embedding allocation; used for listing)
if let Some(meta) = store.get_metadata(1000)? {
    println!("UID {} enrolled with {} v{}", meta.uid, meta.model_id, meta.model_version);
}

// 3. Check Existence
assert!(store.exists(1000)?);

// 4. List Enrolled UIDs
let uids = store.list_enrolled()?; // vec![1000]

// 5. Delete
assert!(store.delete(1000)?);

// 6. Enroll only if absent (check and write under one store lock, GitHub #291)
store.enroll_if_absent(&template)?; // Err(AlreadyEnrolled(1000)) if enrolled meanwhile

// 7. Startup housekeeping: remove orphaned `<uid>.tmp.<pid>.<suffix>` files (GitHub #291)
let report = store.sweep_orphaned_temp_files()?;
println!("removed {} orphaned temporary files", report.removed);
```

---

## 5. Verification & Security Matrix

| Criterion | Requirement | Test Suite | Status |
|---|---|---|---|
| **B1** | Embeddings encrypted at rest with AES-GCM | `tests/encryption_tests.rs`, `tests/proptest_suite.rs` | ☑ Validated |
| **B2** | File mode `0600`, directory mode `0700`, atomic writes | `tests/permissions_tests.rs` | ☑ Validated |
| **B3** | Model metadata tracked for migration | `tests/metadata_tests.rs` | ☑ Validated |
| **B4** | Full CRUD operations tested | `tests/crud_tests.rs` | ☑ Validated |
| **B8** | Existing store directory validated, never chmod-ed; created directory `0700` | `tests/directory_validation_tests.rs`, `store::tests::test_validate_store_dir_*` | ✅ Verified |
| **B9** | Re-enrollment overwrites the replaced template inode (best effort) after an atomic commit | `tests/erasure_tests.rs` | ✅ Verified |
| **B10** | CBOR plaintext returned in a zeroizing buffer | `tests/cbor_zeroize_tests.rs` | ✅ Verified |
| **SRK2** | Existing master key validated on the open descriptor (regular, trusted owner, no group/world bit, 32 bytes, bounded read, `O_NOFOLLOW`) | `tests/master_key_hardening_tests.rs`, `crypto::key_open_tests::test_230_key_open_does_not_follow_symlinks` | ✅ Verified |
| **SRK3** | Missing key parents created `0755`, existing parents never chmod-ed | `tests/master_key_hardening_tests.rs` | ✅ Verified |
| **SGU3** | `enroll_if_absent` checks and writes under one store lock; a concurrent enrollment is never replaced (GitHub #291) | `tests/enroll_if_absent_tests.rs` | ✅ Verified |
| **SGU4** | Orphaned `<uid>.tmp.<pid>.<suffix>` files swept: exact names, regular single-link files of root/euid, older than 60 s, capped, lock tried once, decoys kept (GitHub #291) | `tests/temp_sweep_tests.rs` | ✅ Verified |
