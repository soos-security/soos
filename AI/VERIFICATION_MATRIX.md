# Verification Matrix — Acceptance Criteria by Component

This document translates the critical gating criteria from §11 of `ARCHITECTURE.md` into an actionable checklist for each component. A component is deemed **complete** only when ALL its criteria are validated.

---

## Global Security Invariants (Mandatory for every release)

- [x] No code path ever converts an error or failure into `PAM_SUCCESS` (checked by `crates/pam/src/lib.rs` and `crates/protocol`)
- [x] No camera device is opened by the PAM module (PAM strictly delegates via IPC)
- [x] Daemon unavailable = standard password fallback works (`authenticate_returns_pam_ignore`)
- [x] The `.so` never panics across FFI (enforced by `catch_unwind`)
- [ ] Each ONNX model is attested by manifest + SHA-256 checksum
- [ ] All biometric templates and evidence are located outside `$HOME` and inaccessible to non-root accounts
- [ ] Each target distribution integration is validated in a VM with a documented rollback procedure

---

## Component: `protocol`

| # | Criterion | Test Method | Status |
|---|---|---|---|
| P1 | Types `Request`, `Response`, `Verdict` are serializable and deserializable | Round-trip tests | ☑ Validated |
| P2 | Codec enforces maximum 4,096-byte message boundary | Oversized payload rejection test | ☑ Validated |
| P3 | `request_id` is exactly 256 bits (32 bytes) | Serialization test | ☑ Validated |
| P4 | Protocol is versioned (`version` field) | v1 compatibility test | ☑ Validated |
| P5 | Decoder fuzzing: zero panic on arbitrary inputs | Fuzzing / property test | ☐ Pending |
| P6 | `#![forbid(unsafe_code)]` enabled | Invariant test (`test_business_crates_forbid_unsafe_code`) | ☑ Validated |

---

## Component: `policy`

| # | Criterion | Test Method | Status |
|---|---|---|---|
| PO1 | `Allow` decision only if score >= threshold AND PAD positive AND valid UID | Parametric unit tests (`decision_tests::test_decision_allow_nominal`, `prop_decision_allow_invariant`) | ☑ Validated |
| PO2 | Per-UID rate-limiting is enforced | Request burst test (`rate_limit_tests::test_rate_limit_burst_same_uid`) | ☑ Validated |
| PO3 | Zero I/O inside crate | Dependency audit (`crates/policy/Cargo.toml` contains zero filesystem, network, or async deps) | ☑ Validated |
| PO4 | `#![forbid(unsafe_code)]` enabled | Invariant test (`soos-invariants::test_business_crates_forbid_unsafe_code`) | ☑ Validated |

---

## Component: `pam` (cdylib)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| PA1 | Returns `PAM_IGNORE` when daemon is unavailable | Docker `pamtester` without daemon | ☐ Pending |
| PA2 | Returns `PAM_IGNORE` on timeout (> 250ms) | Simulated slow daemon test | ☐ Pending |
| PA3 | `catch_unwind` wraps all FFI entry points | Code review & panic tests | ☑ Validated |
| PA4 | NEVER starts Tokio runtime | Invariant test (`test_pam_crate_has_no_tokio_dependency`) | ☑ Validated |
| PA5 | Zero `unwrap()` or `expect()` in production code | Invariant test (`test_pam_crate_has_no_unwraps_or_expects`) | ☑ Validated |
| PA6 | Neither reads nor transmits passwords | Invariant test & code audit | ☑ Validated |
| PA7 | Correct C ABI (loadable by Linux-PAM) | Docker `pamtester` T1 test | ☐ Pending |
| PA8 | Absent module = PAM authentication remains functional | Docker `pamtester` T3 test | ☐ Pending |

---

## Component: `daemon`

| # | Criterion | Test Method | Status |
|---|---|---|---|
| D1 | Socket created in `/run/soos/` with `0660` permissions | Integration test (`socket_tests::test_socket_created_with_0660_permissions`, `test_socket_recreation_cleans_up_stale_socket`) | ☑ Validated |
| D2 | `SO_PEERCRED` verified on every connection | Spoofed UID test (`peercred_tests::test_peercred_verification_rejects_mismatched_uid`, `dispatcher_tests::test_dispatcher_rejects_spoofed_uid`) | ☑ Validated |
| D3 | Starts with `RestrictAddressFamilies=AF_UNIX` | Systemd service test (`systemd_test::test_systemd_unit_file_sandboxing_directives`) | ☑ Validated |
| D4 | Health check exposes `socket_ready`, `camera_ready`, `models_verified` | Integration test (`health_tests::test_health_component_readiness_reporting`) | ☑ Validated |
| D5 | Zero sensitive information emitted in logs | Log audit (`logging_audit_test::test_daemon_source_code_has_zero_sensitive_data_in_logs`) | ☑ Validated |

---

## Component: `camera-v4l`

| # | Criterion | Test Method | Status |
|---|---|---|---|
| C1 | `mock-camera` feature provides functional `MockCameraManager` | Unit test | ☐ Pending |
| C2 | Fresh frame available in < 5ms via `ArcSwap` | Benchmark | ☐ Pending |
| C3 | Handles `ENODEV`, `EIO`, `EBUSY` without panic | Error simulation tests | ☐ Pending |
| C4 | Hardware selection by `/dev/v4l/by-id/` rather than index | Configuration test | ☐ Pending |
| C5 | Drops first 15–30 frames after startup for auto-exposure | Functional test | ☐ Pending |

---

## Component: `vision`

| # | Criterion | Test Method | Status |
|---|---|---|---|
| V1 | Golden tests: preprocessing matches training pipeline | Fixture tests | ☐ Pending |
| V2 | L2-normalized embeddings (norm ≈ 1.0) | Math unit test | ☐ Pending |
| V3 | Cosine similarity correctness | Known vector distance test | ☐ Pending |
| V4 | Rejects if 0 or > 1 face detected | Unit tests | ☐ Pending |
| V5 | Full pipeline < 150ms p95 on reference hardware | Benchmark | ☐ Pending |
| V6 | `#![forbid(unsafe_code)]` enabled | Invariant test | ☐ Pending |

---

## Component: `biometric-store`

| # | Criterion | Test Method | Status |
|---|---|---|---|
| B1 | Embeddings encrypted at rest | Read/write test | ☐ Pending |
| B2 | Files under `/var/lib/soos/biometrics/<uid>`, mode `0600`, owner `root:root` | Permissions test | ☐ Pending |
| B3 | `model_id` and version stored with each template | Migration test | ☐ Pending |
| B4 | Template deletion and re-enrollment operational | CRUD tests | ☐ Pending |

---

## Component: `evidence-store`

| # | Criterion | Test Method | Status |
|---|---|---|---|
| E1 | Disabled by default (strictly opt-in) | Configuration test | ☐ Pending |
| E2 | Automatic rotation after 7-day retention | Retention test | ☐ Pending |
| E3 | Daily cap per UID enforced | Limit test | ☐ Pending |
| E4 | Encrypted files, mode `0600`, `root:root` | Permissions test | ☐ Pending |
| E5 | NEVER transmitted across network in Phase 1 | Dependency audit | ☐ Pending |
