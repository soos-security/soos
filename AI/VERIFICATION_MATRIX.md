# Verification Matrix — Acceptance Criteria by Component

This document translates the critical gating criteria from §11 of `ARCHITECTURE.md` into an actionable checklist for each component. A component is deemed **complete** only when ALL its criteria are validated.

---

## Global Security Invariants (Mandatory for every release)

- [x] No code path ever converts an error or failure into `PAM_SUCCESS` (checked by `crates/pam/src/lib.rs` and `crates/protocol`)
- [x] No camera device is opened by the PAM module (PAM strictly delegates via IPC)
- [x] Daemon unavailable = standard password fallback works (`authenticate_returns_pam_ignore`)
- [x] The `.so` never panics across FFI (enforced by `catch_unwind`)
- [x] Each ONNX model is attested by manifest + SHA-256 checksum (`manifest_tests::test_parse_workspace_manifest_file`, `manifest_tests::test_verify_model_checksum_success_and_tamper_detection`, `registry_tests::test_registry_verify_integrity_missing_files_fails_closed`)
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
| P5 | Decoder fuzzing: zero panic on arbitrary inputs | Property tests and libFuzzer harness (`prop_decode_request_never_panics`, `prop_decode_response_never_panics`, `cargo-fuzz`) | ☑ Validated |
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
| PA1 | Returns `PAM_IGNORE` when daemon is unavailable | Integration test (`test_ipc_offline_daemon_returns_ignore`), Docker `pamtester` | ☑ Validated |
| PA2 | Returns `PAM_IGNORE` on timeout (> 250ms) | Simulated slow daemon test (`test_ipc_slow_daemon_timeout`) | ☑ Validated |
| PA3 | `catch_unwind` wraps all FFI entry points | Code review & panic tests | ☑ Validated |
| PA4 | NEVER starts Tokio runtime | Invariant test (`test_pam_crate_has_no_tokio_dependency`) | ☑ Validated |
| PA5 | Zero `unwrap()` or `expect()` in production code | Invariant test (`test_pam_crate_has_no_unwraps_or_expects`) | ☑ Validated |
| PA6 | Neither reads nor transmits passwords | Invariant test & code audit | ☑ Validated |
| PA7 | Correct C ABI (loadable by Linux-PAM) | Docker `pamtester` T1 test & ABI symbol verification | ☑ Validated |
| PA8 | Absent module = PAM authentication remains functional | Docker `pamtester` T3 test | ☑ Validated |

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
| C1 | `mock-camera` feature provides functional `MockCameraManager` | Unit test (`mock_camera_tests::test_mock_camera_generates_frames_and_readiness`) | ☑ Validated |
| C2 | Fresh frame available in < 5ms via `ArcSwap` | Benchmark (`bench_latency_tests::test_arcswap_frame_retrieval_latency_under_5ms`) | ☑ Validated |
| C3 | Handles `ENODEV`, `EIO`, `EBUSY` without panic | Error simulation tests (`error_recovery_tests::test_error_recovery_enodev_without_panic`) | ☑ Validated |
| C4 | Hardware selection by `/dev/v4l/by-id/` rather than index | Configuration test (`config_hardware_tests::test_config_by_id_path_selection`) | ☑ Validated |
| C5 | Drops first 15–30 frames after startup for auto-exposure | Functional test (`warmup_tests::test_warmup_frames_discard_before_ready`) | ☑ Validated |

---

## Component: `vision`

| # | Criterion | Test Method | Status |
|---|---|---|---|
| V1 | Golden tests: preprocessing matches training pipeline | Fixture tests (`align_tests::test_canonical_identity_alignment_matches_reference`, `align_tests::test_translated_face_alignment_recenters`, `align_tests::test_rotated_face_alignment_levels_eyes`) | ☑ Validated |
| V2 | L2-normalized embeddings (norm ≈ 1.0) | Unit & property tests (`embedding_tests::test_l2_norm_and_normalization_criterion_v2`, `proptest_suite::prop_embedding_normalization_criterion_v2`) | ☑ Validated |
| V3 | Cosine similarity correctness | Known vector distance test (`matcher_tests::test_cosine_similarity_identical_vectors`, `matcher_tests::test_cosine_similarity_orthogonal_vectors`, `matcher_tests::test_cosine_similarity_known_precomputed_vectors`) | ☑ Validated |
| V4 | Rejects if 0 or > 1 face detected | Unit tests (`pipeline_tests::test_pipeline_rejects_zero_faces`, `pipeline_tests::test_pipeline_rejects_two_faces`, `pipeline_tests::test_pipeline_rejects_three_faces`) | ☑ Validated |
| V5 | Full pipeline < 150ms p95 on reference hardware | Benchmark (`bench_tests::test_pipeline_latency_budget_under_150ms_p95` — achieved 28.02ms p95) | ☑ Validated |
| V6 | `#![forbid(unsafe_code)]` enabled | Invariant test (`soos-invariants::test_business_crates_forbid_unsafe_code`) | ☑ Validated |

---

## Component: `biometric-store`

| # | Criterion | Test Method | Status |
|---|---|---|---|
| B1 | Embeddings encrypted at rest | `encryption_tests::test_b1_*`, `proptest_suite::prop_encrypt_decrypt_roundtrip` | ☑ Validated |
| B2 | Files under `/var/lib/soos/biometrics/<uid>`, mode `0600`, owner `root:root` | `permissions_tests::test_b2_permissions_and_atomic_writes`, `test_b2_master_key_file_permissions` | ☑ Validated |
| B3 | `model_id` and version stored with each template | `metadata_tests::test_b3_metadata_tracking_and_model_migration_support` | ☑ Validated |
| B4 | Template deletion and re-enrollment operational | `crud_tests::test_b4_full_crud_lifecycle` | ☑ Validated |

---

## Component: `evidence-store`

| # | Criterion | Test Method | Status |
|---|---|---|---|
| E1 | Disabled by default (strictly opt-in) | Configuration test | ✅ Verified (`test_evidence_config_disabled_by_default`, `test_store_snapshot_rejected_when_disabled`) |
| E2 | Automatic rotation after 7-day retention | Retention test | ✅ Verified (`test_retention_rotation_7_days`, `test_retention_rotation_custom_days`) |
| E3 | Daily cap per UID enforced | Limit test | ✅ Verified (`test_daily_cap_per_uid_enforced`, `test_custom_daily_cap`) |
| E4 | Encrypted files, mode `0600`, `root:root` | Permissions test | ✅ Verified (`test_evidence_file_and_directory_permissions`, `test_encryption_roundtrip_and_structure`) |
| E5 | NEVER transmitted across network in Phase 1 | Dependency audit | ✅ Verified (`test_evidence_store_has_zero_network_dependencies`, `test_evidence_store_has_no_network_dependencies`) |

---

## Component: `enrollment-cli`

| # | Criterion | Test Method | Status |
|---|---|---|---|
| EN1 | Scaffolding and CLI argument parsing (subcommands enroll, verify, delete, list) | Unit tests | ✅ Verified (`test_cli_parse_enroll_subcommand_*`, `test_cli_parse_verify_subcommand`, `test_cli_parse_delete_subcommand`, `test_cli_parse_list_subcommand_*`, `test_cli_global_options`) |
| EN2 | Root privilege enforcement (EUID 0) | System call verification | ✅ Verified (`test_root_check_enforced_against_euid`, `test_root_check_bypassed_when_flag_disabled`, `test_root_required_error_message`) |
| EN3 | Multi-frame quality selection & single-face invariant | Quality selection test | ✅ Verified (`test_quality_selects_highest_scoring_single_face`, `test_quality_ignores_multi_face_or_zero_face_frames`, `test_quality_fails_when_all_frames_below_confidence`, `test_enroll_nominal_with_auto_confirm`, `test_enroll_interactive_confirmation_rejected`, `test_enroll_already_enrolled_overwrite_rejected`) |
| EN4 | Anti-forensic secure erasure on template deletion | Destruction test | ✅ Verified (`test_shred_overwrites_and_removes_file`, `test_shred_empty_file_removes_cleanly`, `test_delete_existing_template_with_auto_confirm`, `test_delete_interactive_prompt_cancelled`, `test_delete_non_existent_uid_fails`) |
| EN5 | Diagnostic one-shot verification with latency breakdown and PAD | Diagnostic test | ✅ Verified (`test_verify_matching_user_reports_allow_and_metrics`, `test_verify_non_matching_user_reports_deny`, `test_verify_unenrolled_uid_fails`) |
| EN6 | Enumeration of enrolled UIDs and metadata resolution | Listing test | ✅ Verified (`test_list_empty_store_returns_empty_vec`, `test_list_multiple_enrolled_users_returns_sorted_summaries`) |

