# Candid Review Report

- **Date**: 2026-09-19
- **Target Branch / Commit**: `fix/evidence-store-safety`
- **Base Reference**: `origin/main`
- **Audited Files**:
  - `crates/evidence-store/Cargo.toml`
  - `crates/evidence-store/src/lib.rs`
  - `crates/evidence-store/src/error.rs`
  - `crates/evidence-store/src/crypto.rs`
  - `crates/evidence-store/src/store.rs`
  - `crates/evidence-store/tests/safety_hardening_tests.rs`
  - `crates/evidence-store/tests/proptest_suite.rs`
  - `scripts/sync_issue.py`

## 1. Executive Summary

This cold code review examines the security hardening and atomic operations in `crates/evidence-store` for Issue #30 (GitHub #69). The changes prevent symbolic link path traversal across root and date partition directories, implement atomic file creation with mode `0600` from inception (`O_CREAT | O_EXCL`), synchronize retention rotation across concurrent threads and daemon instances using safe RAII `nix::fcntl::Flock`, and enforce strict POSIX UID boundary validation (`0 <= uid <= MAX_VALID_UID`). The modifications preserve `#![forbid(unsafe_code)]` and pass all deterministic invariant checks.

## 2. Deep Reasoning Audit

### Logic & Architecture
- [PASS]: Date directories and root base directory are verified with `symlink_metadata` (`lstat`) before access or creation, preventing symlink traversal attacks.
- [PASS]: Temporary files for snapshots and master keys are created with explicit mode `0600` via `create_new(true)` atomically, closing exposure windows.
- [PASS]: Retention rotation acquires an exclusive blocking file lock (`nix::fcntl::Flock`) on the evidence base directory, preventing concurrent daemon race conditions and file system corruption during pruning.
- [PASS]: POSIX UID validation strictly caps input user identifiers at `MAX_VALID_UID = 2_147_483_647` (`i32::MAX as u32`), rejecting negative numbers cast to unsigned (high sign bit set) and sentinel invalid UIDs (`(uid_t)-1`).

### PAM Concurrency & Deadlines
- [PASS]: `evidence-store` is utilized exclusively by daemon background task dispatchers; zero Tokio or asynchronous runtimes are introduced into PAM modules.
- [PASS]: Zero stdout/stderr stream pollution (`println!`, `eprintln!`, `dbg!`).

### Panic Safety & Fallback
- [PASS]: Zero `unwrap()`, `expect()`, `panic!()`, or unhandled stubs in `crates/evidence-store/src/`.
- [PASS]: All error paths fail closed and return typed `EvidenceStoreError` (`InvalidPath`, `InvalidUid`, `Io`).
- [PASS]: Directory lock drops cleanly via RAII on function return.

### Test Integrity & Anti-Weakening
- [PASS]: Contractual tests `test_evidence_store_rejects_symlink_date_directory`, `test_concurrent_rotation_does_not_corrupt`, and `test_evidence_store_rejects_path_traversal_uid` comprehensively cover acceptance criteria E1..E5 and sub-issues #30.1, #30.2, and #30.3.
- [PASS]: Existing tests and property tests remain intact and verified.

### Memory & Secret Bounds
- [PASS]: `#![forbid(unsafe_code)]` is strictly preserved.
- [PASS]: Sensitive key material in `load_or_create` is zeroized before return.
- [PASS]: Zero plaintext credentials, passwords, or raw embeddings exposed.

## 3. Detailed Findings & Action Items
- None. All architectural invariants, bounds, and test contracts pass without findings.

## 4. Final Verdict
**VERDICT: APPROVED**
