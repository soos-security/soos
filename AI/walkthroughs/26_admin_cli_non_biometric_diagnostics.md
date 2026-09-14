# Walkthrough 26 — `admin-cli` Crate: Non-Biometric Diagnostics Tool

## Context & Objectives

- **Target Issue**: Backlog Issue #11 / GitHub Issue #18 (`feat/admin-cli`)
- **Mission**: Implement the `soos-admin-cli` binary crate (`crates/admin-cli/`) providing the administrative diagnostic tool (`soos-admin`) with subcommands: `status`, `test-pam`, and `logs`.
- **Acceptance Criteria**: `AD1` through `AD4` in `AI/VERIFICATION_MATRIX.md`.

---

## 1. Architecture & Design (Phase 1 — Architect)

- **Scaffolded Crate**: `crates/admin-cli/` registered in workspace `Cargo.toml`.
- **Manifest Privacy**: Configured `publish.workspace = true` to inherit root workspace privacy and comply with `cargo-deny`.
- **Security & Privilege Boundaries**:
  - Strictly non-biometric diagnostic tool: does NOT touch, store, or manipulate biometric templates in `/var/lib/soos/` or raw camera frames.
  - Sockets: Accesses `/run/soos/daemon.sock` (mode `0660`, group `soos`), permitting diagnostics by root or users in the `soos` group.
  - Offline resilience: when daemon is offline or stopped, `status` gracefully reports offline status and systemd unit state without panics or crashes.
  - Defense-in-depth log sanitization: `RedactionFilter` automatically masks passwords, bearer tokens, private keys, and embedding float vectors.
  - `#![forbid(unsafe_code)]` declared in `src/lib.rs` and `src/main.rs`.

---

## 2. Plan Evaluation Audit (Phase 1.5 — Plan Evaluator)

The Plan Evaluator Sub-Agent audited the implementation plan across all 6 pillars:
- Pillar 1: Architectural Alignment & Threat Model — **PASS**
- Pillar 2: PAM Real-Time Latency & Concurrency — **PASS**
- Pillar 3: Panic Safety & Fail-Closed Behavior — **PASS**
- Pillar 4: Dependency Isolation & Banned Crates — **PASS**
- Pillar 5: Data Confidentiality & Zeroization — **PASS**
- Pillar 6: Test Integrity & TDD Contracts — **PASS**

Authored `AI/plan_evaluator_report.md` with **`VALIDATION_VERDICT: APPROVED`**.

---

## 3. Contractual Testing (Phase 2 — Tester)

Authored comprehensive unit and integration test suites in `crates/admin-cli/tests/` prior to implementation:
- `scaffold_tests.rs`: Tests argument parsing for `status`, `test-pam`, `logs` (options, lines, follow, level, file), and global flags (AD1).
- `status_tests.rs`: Tests daemon status querying against healthy mock daemon, unready component scenarios, offline daemon resilience, and JSON serialization (AD2).
- `test_pam_tests.rs`: Tests simulated PAM authentication cycle, connection latency, response latency, `Allow` verdict, and `Deny`/`Unavailable` -> `PAM_IGNORE` fallback mapping (AD3).
- `redact_tests.rs`: Tests sensitive data redaction filter against passwords, tokens, master keys, float vectors, and benign log preservation (AD4).
- `logs_tests.rs`: Tests log retrieval and stream filtering with file fixtures and line limits (AD4).

All tests compiled and failed initially as expected (TDD Red Phase).

---

## 4. Security & Panic Audit (Phase 3 — Auditor)

- Verified zero `unwrap()` or `expect()` in library production code.
- Enforced saturating arithmetic (`saturating_add`, `saturating_sub`, `saturating_mul`) in string scanning and timestamp math to prevent integer overflows.
- Replaced direct range indexing with safe `.get()` and `.get_mut()` operations.
- Enforced `#![forbid(unsafe_code)]` across all business code.
- Banned dependencies: zero `opencv` or `nokhwa`.

---

## 5. Production Implementation (Phase 4 — Developer)

- Implemented `crates/admin-cli/src/`:
  - `args.rs`: Typed CLI argument structures via `clap`.
  - `error.rs`: Typed errors via `thiserror`.
  - `redact.rs`: Pure string scanning sensitive pattern sanitizer (`RedactionFilter`, `DefaultRedactionFilter`, `default_redact`).
  - `status.rs`: Daemon readiness and systemd unit inspector (`query_status`, `DaemonStatusReport`).
  - `test_pam.rs`: Out-of-band PAM authentication simulator with latency benchmarking (`simulate_pam_auth`, `PamTestReport`).
  - `logs.rs`: Filtered log stream retriever (`fetch_and_filter_logs`).
  - `lib.rs`: Library exports and interface definitions.
  - `main.rs`: CLI binary entry point with formatted terminal reports.
- Extended `soos-protocol` with `RequestKind::Status` and `StatusResponse` struct.
- Integrated `RequestKind::Status` handling in `soos-daemon` dispatcher.
- Integrated `admin-cli` into `tests/invariants/src/lib.rs` under Invariant 1 (`test_business_crates_forbid_unsafe_code`).
- Formatted via `cargo fmt` and validated with `cargo clippy --all-targets --all-features -- -D warnings`.

---

## 6. Candid Review & Validation (Phase 5 — Candid Reviewer)

- Executed `./scripts/candid_subagent.sh`:
  - Layer 1: Deterministic invariant checks passed cleanly (zero unsafe, zero unwrap in PAM, zero banned deps, English policy).
  - Layer 2: AI Sub-Agent reasoning audit approved in `AI/candid_review_report.md`.
- Overall candid review status: **`VERDICT: APPROVED`**.

---

## 7. Verification Evidence

- Total tests passed in `soos-admin-cli`: **22/22**
- Total tests passed across entire workspace: **150+/150+**
- Invariant tests in `tests/invariants`: **9/9 passed**
- License and security audits: `cargo deny check` **passed**
