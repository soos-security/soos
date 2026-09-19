# Walkthrough 41 — Biometric Store: Atomic Key Creation and Secure Deletion

## Overview

This walkthrough documents the resolution of **Issue #25** (`fix(biometric-store): Atomic key creation and secure deletion`, GitHub Issue #64).

The biometric store persistence layer has been hardened across three fundamental Zero-Trust security invariants:
1. **Atomic `0600` Inception**: Master key and template temporary files are created with explicit mode `0600` atomically via `O_CREAT | O_EXCL` in `open()`, closing any race window where secret key material or encrypted templates could be world/group-readable.
2. **Anti-Forensic Secure Erasure**: `BiometricStore::delete()` now performs 3 passes of CSPRNG random byte overwriting (`getrandom::fill`) with hardware fsync (`sync_all`) before unlinking, ensuring deleted template sectors cannot be recovered via block device forensics.
3. **Symlink Traversal Prevention**: `BiometricStore::template_path()`, `BiometricStore::new()`, and `MasterKey::load_or_create()` reject symbolic link paths using `symlink_metadata` and `libc::O_NOFOLLOW`.

---

## Changes Made

### 1. Atomic Key Inception & Symlink Rejection (`crates/biometric-store/src/crypto.rs`)
- Replaced `File::create()` + `set_permissions()` with:
  ```rust
  OpenOptions::new()
      .write(true)
      .create_new(true)
      .mode(0o600)
      .open(&tmp_path)?;
  ```
- Added symlink inspection with `std::fs::symlink_metadata` to reject symlinked master key targets.
- Added `libc::O_NOFOLLOW` when opening existing master key files.

### 2. Symlink Rejection & 3-Pass Secure Erasure (`crates/biometric-store/src/store.rs`)
- Updated `template_path(&self, uid: u32) -> Result<PathBuf, BiometricStoreError>` to inspect `symlink_metadata` and fail closed if a symlink is encountered.
- Enforced `BiometricStore::new` symlink rejection on `base_dir`.
- Updated `enroll` to atomically create template temp files with `create_new(true)` and `mode(0o600)`.
- Implemented in `delete(&self, uid: u32) -> Result<bool, BiometricStoreError>`:
  - 3-pass CSPRNG random buffer overwriting (`4096` bytes per chunk) with `sync_all()` per pass.
  - Safe bounds using `.get_mut(..to_write)` (`#![forbid(unsafe_code)]` compliant).
  - Clean unlinking via `std::fs::remove_file(&path)` upon completion.

### 3. Service Integration (`crates/enrollment-cli/src/service.rs`)
- Updated `delete()` in `EnrollmentService` to delegate directly to `self.store.delete(uid)?`.

---

## Verification & Test Contracts

The following TDD contractual tests were authored and verified:
1. `test_master_key_created_with_0600_from_inception`:
   - Validates that newly generated key files have permissions `0600` from inception.
   - Validates that symlink attacks targeting master key files are rejected.
2. `test_delete_securely_overwrites_before_unlink`:
   - Utilizes hard link probes sharing the template file inode and data blocks.
   - Verifies file sectors are completely overwritten in place with random bytes and no longer contain `SOOSBIO1` headers before unlinking.
3. `test_biometric_store_rejects_symlink_template_path`:
   - Confirms symlinks and broken symlinks pointing outside the store are rejected across `template_path`, `exists`, `get`, `delete`, and `enroll`.

### Verification Suite
- `cargo test -p soos-biometric-store`: 16/16 tests passing.
- `cargo test -p soos-enrollment-cli`: 27/27 tests passing.
- `cargo test --all-targets`: 100% workspace tests passing.
- `cargo clippy --all-targets --all-features -- -D warnings`: 0 warnings.
- `cargo fmt --check`: Clean formatting.
- `./scripts/candid_review.sh`: Passed on all 7 audits.
