# Walkthrough 24 — `evidence-store` Crate: Anti-Intrusion Snapshots

## Context & Objectives

- **Target Issue**: Backlog Issue #9 / GitHub Issue #16 (`feat/evidence-store`)
- **Mission**: Implement the `evidence-store` crate providing local, authenticated, AES-256-GCM encrypted persistence for anti-intrusion camera snapshots with automatic 7-day retention rotation and per-UID daily capture limits.
- **Acceptance Criteria**: `E1`, `E2`, `E3`, `E4`, `E5` in `AI/VERIFICATION_MATRIX.md`.

---

## 1. Architecture & Design (Phase 1 — Architect)

- **Scaffolded Crate**: `crates/evidence-store/` registered in workspace `Cargo.toml`.
- **Manifest Privacy**: Configured `publish.workspace = true` to inherit root privacy and satisfy `cargo-deny`.
- **Zero-Trust Hardening**:
  - Encrypted snapshot files: `/var/lib/soos/evidence/YYYY-MM-DD/<uuid>.webp.enc` with mode `0600` (`-rw-------`).
  - Date partition directories: `/var/lib/soos/evidence/YYYY-MM-DD/` with mode `0700` (`drwx------`).
  - AES-256-GCM authenticated encryption with 96-bit CSPRNG nonces and `SOOSEVD1` magic header.
  - Strictly opt-in: `EvidenceConfig.enabled` defaults to `false` (E1).
  - 7-day retention rotation using pure Gregorian affine calendar math without third-party dependencies (E2).
  - Per-UID daily cap (default 10 captures) to prevent storage exhaustion attacks (E3).
  - Absolute zero network transmission: no network dependencies in `Cargo.toml` or source code (E5).
  - `#![forbid(unsafe_code)]` declared in `crates/evidence-store/src/lib.rs`.

---

## 2. Plan Evaluation Audit (Phase 1.5 — Plan Evaluator)

- The Plan Evaluator Sub-Agent executed an evaluation of the proposed design across the 6 architectural pillars:
  - Architectural Alignment & Threat Model: **PASS**
  - PAM Real-Time Latency & Concurrency: **PASS**
  - Panic Safety & Fail-Closed Behavior: **PASS**
  - Dependency Isolation & Banned Crates: **PASS**
  - Data Confidentiality & Zeroization: **PASS**
  - Test Integrity & TDD Contracts: **PASS**
- Generated `AI/plan_evaluator_report.md` with **`VALIDATION_VERDICT: APPROVED`**.

---

## 3. Contractual Testing (Phase 2 — Tester)

Authoring unit, property, and invariant test suites prior to production implementation:
- `tests/opt_in_tests.rs`: Validates disabled-by-default behavior (E1) and rejection of snapshot requests when disabled.
- `tests/permissions_tests.rs`: Asserts mode `0600` on snapshot files, `0700` on directories, and 36-character UUID v4 filename convention (E4).
- `tests/encryption_tests.rs`: Asserts `SOOSEVD1` magic header, CSPRNG nonce uniqueness, absence of plaintext in ciphertext, and tamper detection (E4).
- `tests/retention_tests.rs`: Tests multi-day directory pruning, verifying directories older than 7 days are pruned while newer ones are retained (E2).
- `tests/daily_cap_tests.rs`: Tests enforcement of daily capture limits per UID and isolation across UIDs and dates (E3).
- `tests/zero_network_tests.rs`: Audits `Cargo.toml` and source code for zero network dependencies (E5).
- `tests/proptest_suite.rs`: 50 property-based runs verifying arbitrary frame sizes, UUIDs, and roundtrip encryption.
- `tests/invariants/src/lib.rs`: Invariant 9 validating zero networking crates across `crates/evidence-store/`.

All tests initially failed as expected (TDD Red Phase).

---

## 4. Security & Panic Audit (Phase 3 — Auditor)

- Verified absence of `unwrap()` and `expect()` in library production code.
- Verified memory zeroization (`Zeroize` on `MasterKey`, `Zeroizing` on decrypted plaintext buffers).
- Confirmed redaction of master key in `Debug` implementation.
- Confirmed license compatibility of dependencies with `deny.toml`.

---

## 5. Implementation (Phase 4 — Developer)

Implemented production modules in `crates/evidence-store/src/`:
- `config.rs`: `EvidenceConfig` with `enabled: false`, `retention_days: 7`, `daily_cap_per_uid: 10`.
- `crypto.rs`: `MasterKey` management, `encrypt_payload`, `decrypt_payload` using `Aes256Gcm`.
- `snapshot.rs`: `EvidenceRecord` CBOR codec, `generate_uuid_v4`, and Gregorian date arithmetic (`days_since_epoch`, `format_date_from_timestamp`).
- `store.rs`: `EvidenceStore` engine implementing `store_snapshot` (atomic write + POSIX permissions), `load_snapshot`, `rotate_retention`, and `daily_count`.
- All tests passed cleanly (TDD Green Phase).
- Zero warnings under `cargo clippy --all-targets --all-features -- -D warnings`.
- Code formatted with `cargo fmt`.

---

## 6. Candid Review (Phase 5 — Candid Reviewer)

- Independent diff review conducted across the 5 pillars.
- Authored `AI/candid_review_report.md` with **`VERDICT: APPROVED`**.

---

## 7. Traceability & Matrix Synchronization (Phase 6 — Traceability)

- Updated `AI/VERIFICATION_MATRIX.md` marking E1 through E5 as Verified.
- Authored `Docs/EVIDENCE_STORE_CRATE.md`.
- Synchronized issues via `scripts/sync_issue.py`.
