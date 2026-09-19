# Plan Evaluation Report — Issue #30: Symlink Safety and Atomic Operations in Evidence Store

- **Target Issue**: Issue #30 (`fix/evidence-store-safety`, GitHub Issue #69)
- **Evaluator**: Independent Plan Evaluator Sub-Agent
- **Date**: 2026-09-19
- **Status**: Complete

---

## 1. Context Ingestion Audit

| Source Document | Status | Notes |
| :--- | :--- | :--- |
| `AI/ARCHITECTURE.md` | Ingested | Verified §9 Evidence Snapshots, `/var/lib/soos/evidence` invariants, permissions `0700`/`0600` |
| `AI/DECISIONS.md` | Ingested | Verified ADR-001..ADR-012 constraints |
| `AI/BACKLOG.md` | Ingested | Verified Issue #30 sub-issues #30.1, #30.2, and #30.3 |
| `AI/VERIFICATION_MATRIX.md` | Ingested | Verified acceptance criteria E1..E5, symlink & concurrent safety |
| `Docs/SECURITY_AND_QUALITY_GUIDELINES.md` | Ingested | Invariants on panic safety, bounds, zeroization, atomic file creation |
| `AGENTS.md` | Ingested | Strict test integrity, `#![forbid(unsafe_code)]`, and English policy |

---

## 2. Pillar-by-Pillar Compliance Assessment

### Pillar 1: Architectural Alignment & Threat Model
- **Evaluation**: PASS
- **Details**:
  - Respects `/var/lib/soos/evidence/` storage hierarchy with `0700` directories and `0600` encrypted files.
  - Mitigates local unprivileged symlink attacks: pre-creation verification via `symlink_metadata` ensures date directories and root base directories cannot be hijacked via symlinks to escape `/var/lib/soos/evidence/`.
  - Ensures atomic creation of files with mode `0600` from inception (`O_CREAT | O_EXCL`), closing world/group-readable exposure windows.

### Pillar 2: PAM Real-Time Latency & Concurrency
- **Evaluation**: PASS
- **Details**:
  - Evidence store is utilized exclusively by `soos-daemon` during background processing; zero impact on PAM synchronous 200-250ms authentication latency.
  - Serialization of retention rotation using RAII `nix::fcntl::Flock` on the evidence base directory prevents concurrent rotation races and avoids file system corruption across daemon restarts.

### Pillar 3: Panic Safety & Fail-Closed Behavior
- **Evaluation**: PASS
- **Details**:
  - `#![forbid(unsafe_code)]` remains strictly enforced in `crates/evidence-store`.
  - Zero `unwrap()` or `expect()` in production code.
  - Uses `nix::fcntl::Flock`, which provides a 100% safe RAII lock guard over `std::fs::File`.
  - Systematic fail-closed error handling returning typed `EvidenceStoreError` (`InvalidPath`, `InvalidUid`, `Io`).

### Pillar 4: Dependency Isolation & Banned Crates
- **Evaluation**: PASS
- **Details**:
  - Zero banned dependencies (no OpenCV, no nokhwa).
  - Uses existing workspace dependency `nix` with safe `fs` features for RAII directory locking.
  - Zero network dependencies, upholding Criterion E5.

### Pillar 5: Data Confidentiality & Zeroization
- **Evaluation**: PASS
- **Details**:
  - Encrypted snapshots continue using AES-256-GCM with master key protection.
  - Temporary files created with `mode(0o600)` and `create_new(true)`.
  - Master key loading and creation strictly rejects symlinks and wipes temporary memory buffers.
  - Excessively large or negative (sign-bit set) UIDs are rejected before any disk persistence or cap tracking.

### Pillar 6: Test Integrity & TDD Contracts
- **Evaluation**: PASS
- **Details**:
  - Three explicit contractual tests authored in Phase 2:
    - `test_evidence_store_rejects_symlink_date_directory` (#30.1)
    - `test_concurrent_rotation_does_not_corrupt` (#30.2)
    - `test_evidence_store_rejects_path_traversal_uid` (#30.3)
  - Immutable test contracts: tests are immutable acceptance criteria, with zero weakening allowed.

---

## 3. Plan Evaluator Conclusion

The implementation plan satisfies all zero-trust architectural invariants, panic safety constraints, concurrency guarantees, and test integrity requirements.

**VALIDATION_VERDICT: APPROVED**
