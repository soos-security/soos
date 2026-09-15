# Walkthrough 27 — `soos-daemon`: Full Pipeline Integration

## Context & Objectives

- **Target Issue**: Backlog Issue #12 / GitHub Issue #19 (`feat/daemon-pipeline`)
- **Mission**: Wire together all subsystems (`soos-camera-v4l`, `soos-vision`, `soos-inference-ort`, `soos-biometric-store`, `soos-evidence-store`, `soos-policy`) into `soos-daemon`, providing end-to-end facial verification, deadline enforcement, rate limiting, and intrusion evidence capture.
- **Acceptance Criteria**: Criteria `D6` through `D11` in `AI/VERIFICATION_MATRIX.md`.

---

## 1. Architecture & Design (Phase 1 — Architect)

- **Scaffolded Integration**:
  - `crates/daemon/src/pipeline.rs`: Declares `PipelineComponents` encapsulating `Arc<dyn CameraManager>`, `Arc<VisionPipeline>`, `Arc<BiometricStore>`, `Arc<EvidenceStore>`, and `Arc<Mutex<AuthorizationEngine>>`.
  - Configured constants: `MAX_FRAME_AGE_NS = 150_000_000` (150ms frame freshness limit) and `DECISION_BUDGET_MS = 150` (150ms cumulative decision budget).
  - Monotonic clock utility `current_monotonic_nanos()` using safe POSIX bindings (`nix::time::clock_gettime(ClockId::CLOCK_MONOTONIC)`).
  - `#![forbid(unsafe_code)]` maintained in `crates/daemon/src/lib.rs` and `src/main.rs`.
- **Dispatcher Request & Event Routing**:
  - Wired full request dispatching in `ConnectionDispatcher::handle_request`.
  - Wire schema disambiguation for incoming frames between `Request` and `Event` without unconsumed trailing bytes.
  - On `EventKind::PasswordFailed`: best-effort intrusion snapshot captured to `EvidenceStore` if enabled.

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

Authored comprehensive integration tests in `crates/daemon/tests/pipeline_integration_tests.rs`:
- `test_12_1_camera_startup_reports_readiness_to_health`: Verifies camera startup and readiness propagation to `HealthState` (D6).
- `test_12_2_deadline_exceeded_returns_unavailable_timeout`: Verifies monotonic deadline propagation returning `Verdict::Unavailable` with `ReasonClass::Timeout` (D7).
- `test_12_3_missing_enrollment_returns_unavailable`: Verifies unenrolled UID returns `Verdict::Unavailable` with `ReasonClass::InternalError` (D8).
- `test_12_4_password_failed_event_captures_evidence_snapshot`: Verifies `PasswordFailed` telemetry event captures encrypted intrusion frame in `EvidenceStore` (D9).
- `test_12_5_rate_limit_exceeded_returns_protocol_error_rate_limited`: Verifies per-UID rate limit exhaustion returns `Verdict::ProtocolError` with `ReasonClass::RateLimited` (D10).
- `test_12_6_all_four_verdict_paths`: Verifies end-to-end all 4 verdict paths (`Allow`, `Deny`/`ProtocolError`, `Unavailable`, and diagnostic status query) (D11).

All tests failed initially (TDD Red Phase).

---

## 4. Security & Panic Audit (Phase 3 — Auditor)

- Validated absence of `unwrap()` or `expect()` in daemon production code.
- Enforced safe monotonic clock retrieval via `nix::time::clock_gettime(ClockId::CLOCK_MONOTONIC)`.
- Verified logging audit compliance: zero sensitive keywords (`password`, `secret`, `credential`, `embedding`, `frame`, `image`, `raw_payload`) in logging macros.
- Preserved strict `#![forbid(unsafe_code)]` in `crates/daemon/src/lib.rs` and `crates/daemon/src/main.rs`.

---

## 5. Production Implementation (Phase 4 — Developer)

- Implemented `PipelineComponents` in `crates/daemon/src/pipeline.rs`.
- Extended `crates/daemon/src/config.rs` with `PipelineConfig`.
- Extended `crates/daemon/src/error.rs` with `From` conversions for camera, biometric store, evidence store, and vision errors.
- Enhanced `crates/daemon/src/dispatcher.rs` with:
  - Disambiguation logic between `Request` and `Event` wire schemas.
  - Freshness checking of camera frames against `MAX_FRAME_AGE_NS`.
  - Monotonic deadline checks at each stage of the authentication pipeline.
  - Retrieval of biometric templates from encrypted `BiometricStore`.
  - Preprocessing, face detection, landmark detection, and embedding matching via `VisionPipeline`.
  - Rate-limited evaluation using `AuthorizationEngine`.
  - Telemetry event handling capturing intrusion evidence via `EvidenceStore`.
- Synchronized camera timestamps in `crates/camera-v4l/src/mock.rs` and `v4l_impl.rs` to true monotonic clock.

---

## 6. Candid Review Audit (Phase 5 — Candid Reviewer)

The Candid Reviewer Sub-Agent audited the raw diff across 5 core pillars:
- Pillar 1: Logic & Architectural Soundness — **PASS**
- Pillar 2: PAM Concurrency & Real-Time Deadlines — **PASS**
- Pillar 3: Panic Safety & Fail-Closed Behavior — **PASS**
- Pillar 4: Strict Test Integrity (Zero Weakening) — **PASS**
- Pillar 5: Memory Safety, Bounds & Secrets — **PASS**

Authored `AI/candid_review_report.md` with **`VERDICT: APPROVED`**.

---

## 7. Traceability & Verification (Phase 6 — Traceability)

- Synchronized `AI/BACKLOG.md` and GitHub Issue #19 (`scripts/sync_issue.py --auto`).
- Updated `AI/VERIFICATION_MATRIX.md` marking criteria `D6` through `D11` as `✅ Verified`.
- Full workspace test suite passes cleanly: 100+ tests passed across all 12 crates.
- Formatted with `cargo fmt` and audited with `cargo clippy --all-targets --all-features -- -D warnings`.
