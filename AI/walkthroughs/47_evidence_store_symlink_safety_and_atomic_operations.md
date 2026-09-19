# Walkthrough 47 — Evidence Store: Symlink Safety, Atomic Operations, and File Locking

## Overview

This walkthrough documents the resolution of **Issue #30** (`fix(evidence-store): Symlink safety and atomic operations`, GitHub Issue #69).

The evidence store persistence layer (`crates/evidence-store`) has been hardened across three critical Zero-Trust security invariants:
1. **Symlink Traversal Prevention**: Pre-creation and pre-write verification on date directories and the root base directory using `symlink_metadata` (`lstat`), preventing directory escapes and symlink redirection attacks (#30.1).
2. **Concurrent Rotation File Locking**: Retention rotation now synchronizes concurrent processes and restarts using an exclusive RAII file lock (`nix::fcntl::Flock`) on the evidence base directory, preventing data corruption and race conditions during automated pruning (#30.2).
3. **POSIX UID Bounds Validation**: Validates user identifiers in `store_snapshot()`, ensuring `uid <= MAX_VALID_UID` (where `MAX_VALID_UID = 2_147_483_647` / `2^31 - 1`), systematically rejecting negative values cast to unsigned (sign-bit set) and invalid sentinel identifiers (`(uid_t)-1`) (#30.3).

---

## Changes Made

### 1. Error Definitions & Types (`crates/evidence-store/src/error.rs`)
- Added `EvidenceStoreError::InvalidUid(u32)` for rejected out-of-bounds user identifiers.
- Added `EvidenceStoreError::InvalidPath(String)` for rejected symlink paths, non-directories, and traversal attempts.

### 2. Atomic Inception & Symlink Defense (`crates/evidence-store/src/crypto.rs`)
- Hardened `MasterKey::load_or_create` with `symlink_metadata` to reject symlinks targeting master key paths.
- Replaced non-atomic file creation with `OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp_path)` to ensure mode `0600` from inception.
- Added CSPRNG salt to temporary file names to prevent symlink collision attacks.

### 3. Store Engine Hardening (`crates/evidence-store/src/store.rs`)
- Defined `pub const MAX_VALID_UID: u32 = 2_147_483_647;` (`i32::MAX as u32`).
- In `store_snapshot()`:
  - Enforced `uid <= MAX_VALID_UID`.
  - Inspected `base_dir` and `target_dir` with `symlink_metadata`, returning `EvidenceStoreError::InvalidPath` upon detecting symbolic links.
  - Replaced temporary file creation with atomic `create_new(true).mode(0o600)` with CSPRNG nonce in temporary filename.
  - Verified `final_path` is not a symlink prior to atomic rename.
- In `list_snapshots_for_date()`:
  - Rejected symlinked date directories and skipped symlinks during snapshot file enumeration.
- In `rotate_retention()`:
  - Acquired exclusive RAII file lock on `self.config.base_dir` via `nix::fcntl::Flock::lock(dir_file, FlockArg::LockExclusive)`.
  - Ignored symlinks in `base_dir` during traversal to prevent following external directories.
  - Gracefully handled concurrent directory deletions (`ErrorKind::NotFound`).

### 4. Workspace & Dependencies (`crates/evidence-store/Cargo.toml`)
- Added `nix = { workspace = true }` dependency.

---

## Verification & Test Contracts

### 1. Contractual Integration Tests (`crates/evidence-store/tests/safety_hardening_tests.rs`)
The following TDD contractual tests were authored and verified:
1. `test_evidence_store_rejects_symlink_date_directory` (#30.1):
   - Created a symlink in `base_dir` pointing to an external directory.
   - Asserted that `store_snapshot` returns `EvidenceStoreError::InvalidPath` containing `"symlink"`.
   - Verified zero files written into the symlink target.
2. `test_concurrent_rotation_does_not_corrupt` (#30.2):
   - Seeded multiple date partitions older and newer than the 7-day retention threshold.
   - Spawned 8 concurrent threads executing `rotate_retention("2026-09-20")`.
   - Verified all threads completed without crash or corruption, older directories were cleanly pruned, and retained directories remained intact.
3. `test_evidence_store_rejects_path_traversal_uid` (#30.3):
   - Verified rejection of `u32::MAX` (`(uid_t)-1`).
   - Verified rejection of negative values cast to unsigned (`(-1000i32) as u32`).
   - Verified rejection of `MAX_VALID_UID + 1`.
   - Confirmed nominal acceptance of valid POSIX UIDs: `0` (root), `1000` (user), `65534` (nobody), and `MAX_VALID_UID`.

### 2. Dual-Layer Candid Review
- `scripts/candid_review.sh`: 7/7 deterministic audits passed (zero unsafe, zero unwraps/panics in PAM, zero tokio in PAM, zero opencv/nokhwa, shell syntax valid, zero prints, English-only policy).
- `scripts/candid_subagent.sh`: Deep reasoning audit authored in `AI/candid_review_report.md` with `VERDICT: APPROVED`.

### 3. Verification Suite
- `cargo test -p soos-evidence-store`: 20/20 passed.
- `cargo test --workspace`: 100% monorepo tests passed.
- `cargo test -p soos-invariants`: 19/19 passed.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: 0 warnings.
- `cargo fmt --check`: Clean formatting.
