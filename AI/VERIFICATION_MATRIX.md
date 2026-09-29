# Verification Matrix — Acceptance Criteria by Component

This document translates the critical gating criteria from §11 of `ARCHITECTURE.md` into an actionable checklist for each component. A component is deemed **complete** only when ALL its criteria are validated.

---

## Global Security Invariants (Mandatory for every release)

- [x] No code path ever converts an error or failure into `PAM_SUCCESS` (checked by `crates/pam/src/lib.rs` and `crates/protocol`)
- [x] No camera device is opened by the PAM module (PAM strictly delegates via IPC)
- [x] Daemon unavailable = standard password fallback works (`authenticate_returns_pam_ignore`)
- [x] The `.so` never panics across FFI (enforced by `catch_unwind`)
- [x] Each ONNX model is attested by manifest + SHA-256 checksum (`manifest_tests::test_parse_workspace_manifest_file`, `manifest_tests::test_verify_model_checksum_success_and_tamper_detection`, `registry_tests::test_registry_verify_integrity_missing_files_fails_closed`)
- [x] All biometric templates and evidence are located outside `$HOME` and inaccessible to non-root accounts (`test_install_script_creates_required_directories`, mode `0700` `root:root`)
- [x] Each target distribution integration is validated with a documented rollback procedure (`test_pam_config_ordering_matches_spec`, `test_uninstall_restores_pam_config`)

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
| POL1 | Decision engine explicitly rejects `f32::INFINITY` scores | Unit tests (`decision_tests::test_decision_deny_score_infinity`, `decision_tests::test_decision_deny_score_neg_infinity`) | ✅ Verified |
| POL2 | RateLimiter evicts stale UIDs and maintains capacity bounds | Unit tests (`rate_limit_tests::test_rate_limit_capacity_bounds_and_lru_eviction`, `rate_limit_tests::test_rate_limit_stale_uid_eviction_on_capacity`, `rate_limit_tests::test_rate_limit_default_capacity`, `rate_limit_tests::test_rate_limit_zero_capacity_fails_closed`) | ✅ Verified |

---

## Component: `pam` (cdylib)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| PA1 | Returns `PAM_IGNORE` when daemon is unavailable | Integration test (`test_ipc_offline_daemon_returns_ignore`), Docker `pamtester` | ☑ Validated |
| PA2 | Returns `PAM_IGNORE` on timeout (> 250ms) | Simulated slow daemon test (`test_ipc_slow_daemon_timeout`), Docker T2/T3 | ✅ Verified |
| PA3 | `catch_unwind` wraps all FFI entry points and argument parsing (`pam_sm_*`, `parse_argv`, `parse_cstrs`) | Unit & integration tests (`panic_safety_returns_pam_ignore`, `authenticate_catches_parse_argv_panics`, `test_c_abi_all_entry_points_panic_safe`) | ✅ Verified |
| PA4 | NEVER starts Tokio runtime | Invariant test (`test_pam_crate_has_no_tokio_dependency`) | ☑ Validated |
| PA5 | Zero `unwrap()` or `expect()` in production code | Invariant test (`test_pam_crate_has_no_unwraps_or_expects`) | ☑ Validated |
| PA6 | Neither reads nor transmits passwords | Invariant test & code audit | ☑ Validated |
| PA7 | Correct C ABI (loadable by Linux-PAM) | Docker `pamtester` & `pam_test_runner` T1 test & ABI symbol verification | ✅ Verified |
| PA8 | Absent module = PAM authentication remains functional | Docker T8 test | ✅ Verified |
| PA9 | Returns `PAM_IGNORE` on daemon crash mid-request | Unit & integration tests (`test_ipc_daemon_crash_immediate_disconnect_returns_ignore`, `test_ipc_daemon_crash_partial_header_returns_ignore`, `test_ipc_daemon_crash_truncated_body_returns_ignore`), Docker T4/T5 | ✅ Verified |
| PA10 | Multi-distribution PAM stack integration across Debian/Ubuntu, RHEL/Fedora, and Arch Linux | Invariant test (`test_pam_docker_matrix_files_and_distro_configs_exist`), Docker matrix runner (`tests/docker/run_matrix.sh`) | ✅ Verified |
| PA11 | `pam-bindings` 0.3.0 `PamHooks` trait implementation with unhandled hook defaults | Unit & integration tests (`test_pam_hooks_unhandled_hooks_return_ignore`, `test_pam_hooks_authenticate_offline_daemon_returns_ignore`, `test_pam_crate_uses_pam_bindings_and_implements_pam_hooks`) | ✅ Verified |
| PA12 | Syslog panic logging on caught panics without secret leakage | Unit & integration tests (`test_syslog_panic_message_formatting`, `test_syslog_panic_message_sanitization`, `test_syslog_log_panic_execution`, `test_pam_crate_has_syslog_panic_logging_without_secrets`) | ✅ Verified |
| PA13 | Dynamic buffer growth for POSIX `getpwnam_r` resolving users with LDAP/AD backends | Unit & retry tests (`uid_resolution_tests::test_getpwnam_r_handles_erange_retry`, `uid_resolution_tests::test_getpwnam_r_caps_at_max_buffer_size`, `uid_resolution_tests::test_getpwnam_r_nominal_resolution`, `uid_resolution_tests::test_getpwnam_r_nonexistent_user_returns_none`) | ✅ Verified |
| PA14 | Telemetry `PasswordFailed` event payload includes target `uid` for evidence snapshot attribution | Unit & integration tests (`ipc_tests::test_password_failed_event_includes_uid`, `pipeline_integration_tests::test_12_4_password_failed_event_captures_evidence_snapshot`) | ✅ Verified |
| PA15 | Non-blocking socket connect with strict timeout budget: saturated listen backlog or frozen daemon times out within budget (< 250ms) and fails closed to `PAM_IGNORE` | Integration test (`ipc_tests::test_ipc_connect_timeout_frozen_daemon`) | ✅ Verified |
| PA16 | Memory zeroization of IPC requests, nonces, and buffers: `Request` and `Event` implement `Zeroize` on `Drop`, and PAM IPC buffers are zeroized upon deallocation | Unit & integration tests (`ipc_tests::test_request_and_event_zeroize_on_drop`) | ✅ Verified |

---

## Component: `daemon`

| # | Criterion | Test Method | Status |
|---|---|---|---|
| D1 | Socket created in `/run/soos/` with `0660` permissions | Integration test (`socket_tests::test_socket_created_with_0660_permissions`, `test_socket_recreation_cleans_up_stale_socket`) | ☑ Validated |
| D2 | `SO_PEERCRED` verified on every connection | Spoofed UID test (`peercred_tests::test_peercred_verification_rejects_mismatched_uid`, `dispatcher_tests::test_dispatcher_rejects_spoofed_uid`) | ☑ Validated |
| D3 | Starts with `RestrictAddressFamilies=AF_UNIX` | Systemd service test (`systemd_test::test_systemd_unit_file_sandboxing_directives`) | ☑ Validated |
| D4 | Health check exposes `socket_ready`, `camera_ready`, `models_verified` | Integration test (`health_tests::test_health_component_readiness_reporting`) | ☑ Validated |
| D5 | Zero sensitive information emitted in logs | Log audit (`logging_audit_test::test_daemon_source_code_has_zero_sensitive_data_in_logs`) | ☑ Validated |
| D6 | Camera readiness integrated with HealthState and auth checks | Integration test (`pipeline_integration_tests::test_12_1_camera_startup_reports_readiness_to_health`) | ✅ Verified |
| D7 | Monotonic deadline propagation and 150ms decision budget enforced | Integration test (`pipeline_integration_tests::test_12_2_deadline_exceeded_returns_unavailable_timeout`) | ✅ Verified |
| D8 | BiometricStore template retrieval with missing enrollment fallback | Integration test (`pipeline_integration_tests::test_12_3_missing_enrollment_returns_unavailable`) | ✅ Verified |
| D9 | EvidenceStore intrusion snapshot on PasswordFailed event | Integration test (`pipeline_integration_tests::test_12_4_password_failed_event_captures_evidence_snapshot`) | ✅ Verified |
| D10 | Policy rate limiting integrated per UID | Integration test (`pipeline_integration_tests::test_12_5_rate_limit_exceeded_returns_protocol_error_rate_limited`) | ✅ Verified |
| D11 | Full pipeline end-to-end all 4 verdict paths (Allow, Deny, Unavailable, ProtocolError) | Integration test (`pipeline_integration_tests::test_12_6_all_four_verdict_paths`) | ✅ Verified |
| D12 | Fail-closed dispatcher fallback (returns `Unavailable`/`InternalError`, never `Allow` without pipeline) | Integration test (`dispatcher_tests::test_dispatcher_no_pipeline_returns_unavailable_not_allow`) | ✅ Verified |
| D13 | Full pipeline initialization and TOML configuration (`--config`, `--mock-camera`, missing models fail closed) | Integration & unit tests (`config_tests::test_config_file_parsing_complete`, `config_tests::test_config_defaults_when_file_absent`, `config_tests::test_config_file_invalid_syntax_fails_closed`, `pipeline_init_tests::test_daemon_startup_initializes_all_pipeline_components`, `pipeline_init_tests::test_mock_camera_flag_uses_mock_manager`, `pipeline_init_tests::test_pipeline_init_missing_models_fails_closed`) | ✅ Verified |
| D14 | ONNX model download, SHA-256 verification, and fail-fast startup attestation | Integration & script tests (`model_deployment_tests::test_download_script_verifies_checksums`, `model_deployment_tests::test_models_readme_complete_and_accurate`, `model_deployment_tests::test_daemon_refuses_start_with_missing_models`, `model_deployment_tests::test_daemon_refuses_start_with_tampered_models`) | ✅ Verified |
| D15 | Configuration file parsed from TOML; defaults used when absent | Unit test (`config_tests::test_config_file_parsing_complete`, `config_tests::test_config_defaults_when_file_absent`, `config_tests::test_config_file_invalid_syntax_fails_closed`) | ✅ Verified |
| D16 | Socket binding is TOCTOU-safe with symlink protection and root:soos ownership | Security & integration tests (`socket_tests::test_socket_binding_resists_symlink_race`, `socket_tests::test_socket_ownership_root_soos`, `dispatcher_tests::test_dispatcher_rejects_invalid_protocol_version`, `dispatcher_tests::test_dispatcher_rejects_oversized_service_name`) | ✅ Verified |
| D17 | Policy engine concurrency with `RwLock` and `check_allowed` without lock starvation | Unit & integration tests (`policy_concurrency_tests::test_concurrent_auth_requests_no_lock_starvation`) | ✅ Verified |
| D18 | Safe monotonic clock fallback with fail-closed `Unavailable` verdict | Unit & integration tests (`policy_concurrency_tests::test_monotonic_clock_invalid_clock_id_returns_error`, `policy_concurrency_tests::test_monotonic_clock_failure_returns_unavailable`) | ✅ Verified |
| D19 | `systemd-logind` session validation cross-referencing `/run/systemd/sessions/` for active local sessions | Unit & integration tests (`policy_concurrency_tests::test_session_validator_parses_active_session`, `policy_concurrency_tests::test_auth_rejected_for_uid_without_active_session`) | ✅ Verified |

---

## Component: `camera-v4l`

| # | Criterion | Test Method | Status |
|---|---|---|---|
| C1 | `mock-camera` feature provides functional `MockCameraManager` | Unit test (`mock_camera_tests::test_mock_camera_generates_frames_and_readiness`) | ☑ Validated |
| C2 | Fresh frame available in < 5ms via `ArcSwap` | Benchmark (`bench_latency_tests::test_arcswap_frame_retrieval_latency_under_5ms`) | ☑ Validated |
| C3 | Handles `ENODEV`, `EIO`, `EBUSY` without panic | Error simulation tests (`error_recovery_tests::test_error_recovery_enodev_without_panic`) | ☑ Validated |
| C4 | Hardware selection by `/dev/v4l/by-id/` rather than index | Configuration test (`config_hardware_tests::test_config_by_id_path_selection`) | ☑ Validated |
| C5 | Drops first 15–30 frames after startup for auto-exposure | Functional test (`warmup_tests::test_warmup_frames_discard_before_ready`) | ☑ Validated |
| C6 | Priority format negotiation (`RGB24 -> YUYV -> NV12 -> MJPEG -> Grey`) and fallback | Unit tests (`format_negotiation_tests::test_format_negotiation_prefers_rgb24`, `format_negotiation_tests::test_format_fallback_on_unsupported`, `format_negotiation_tests::test_format_negotiation_all_priority_order`) | ✅ Verified |
| C7 | Graceful hot-unplug recovery on `ENODEV` with automatic reconnection | Integration test (`hotunplug_tests::test_camera_hotunplug_recovery`) | ✅ Verified |
| C8 | Multi-sensor device classification distinguishing RGB vs IR sensors | Unit tests (`dual_sensor_tests::test_dual_sensor_prefers_rgb`, `dual_sensor_tests::test_dual_sensor_override_prefers_ir`, `dual_sensor_tests::test_sensor_classification_by_card_name`, `dual_sensor_tests::test_sensor_classification_by_formats`) | ✅ Verified |
| C9 | Capture thread shutdown completes within 500ms even if camera is idle | Unit & integration tests (`shutdown_tests::test_camera_drop_completes_within_timeout`, `shutdown_tests::test_camera_stop_signals_graceful_shutdown`, `shutdown_tests::test_v4l_camera_drop_completes_within_timeout`, `shutdown_tests::test_is_ready_memory_visibility_acquire_release`) | ✅ Verified |

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
| V7 | NV12 YUV 4:2:0 bi-planar decoding to RGB24 with dimension and size validation | Unit tests (`color_tests::test_nv12_to_rgb_conversion`, `color_tests::test_nv12_known_reference_image`, `color_tests::test_nv12_invalid_size_fails_closed`, `color_tests::test_nv12_odd_dimensions_rejected`) | ✅ Verified |

---

## Component: `biometric-store`

| # | Criterion | Test Method | Status |
|---|---|---|---|
| B1 | Embeddings encrypted at rest | `encryption_tests::test_b1_*`, `proptest_suite::prop_encrypt_decrypt_roundtrip` | ☑ Validated |
| B2 | Files under `/var/lib/soos/biometrics/<uid>`, mode `0600`, owner `root:root` | `permissions_tests::test_b2_permissions_and_atomic_writes`, `test_b2_master_key_file_permissions` | ☑ Validated |
| B3 | `model_id` and version stored with each template | `metadata_tests::test_b3_metadata_tracking_and_model_migration_support` | ☑ Validated |
| B4 | Template deletion and re-enrollment operational | `crud_tests::test_b4_full_crud_lifecycle` | ☑ Validated |
| B5 | Master key created atomically with mode `0600` from inception (`O_CREAT \| O_EXCL`) | `permissions_tests::test_master_key_created_with_0600_from_inception` | ✅ Verified |
| B6 | Anti-forensic secure erasure on template deletion (3-pass CSPRNG overwrite before unlinking) | `crud_tests::test_delete_securely_overwrites_before_unlink` | ✅ Verified |
| B7 | Symlink traversal prevention on template and key paths | `crud_tests::test_biometric_store_rejects_symlink_template_path`, `permissions_tests::test_master_key_created_with_0600_from_inception` | ✅ Verified |

---

## Component: `evidence-store`

| # | Criterion | Test Method | Status |
|---|---|---|---|
| E1 | Disabled by default (strictly opt-in) | Configuration test | ✅ Verified (`test_evidence_config_disabled_by_default`, `test_store_snapshot_rejected_when_disabled`) |
| E2 | Automatic rotation after 7-day retention | Retention test | ✅ Verified (`test_retention_rotation_7_days`, `test_retention_rotation_custom_days`) |
| E3 | Daily cap per UID enforced | Limit test | ✅ Verified (`test_daily_cap_per_uid_enforced`, `test_custom_daily_cap`) |
| E4 | Encrypted files, mode `0600`, `root:root` | Permissions test | ✅ Verified (`test_evidence_file_and_directory_permissions`, `test_encryption_roundtrip_and_structure`) |
| E5 | NEVER transmitted across network in Phase 1 | Dependency audit | ✅ Verified (`test_evidence_store_has_zero_network_dependencies`, `test_evidence_store_has_no_network_dependencies`) |
| E6 | Symlink traversal prevention on date and base directories | `safety_hardening_tests::test_evidence_store_rejects_symlink_date_directory` | ✅ Verified |
| E7 | Concurrent retention rotation serialized via `flock()` | `safety_hardening_tests::test_concurrent_rotation_does_not_corrupt` | ✅ Verified |
| E8 | Strict POSIX UID validation & path traversal prevention | `safety_hardening_tests::test_evidence_store_rejects_path_traversal_uid` | ✅ Verified |

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
| EN7 | Model registry ID attestation matching `models/manifest.toml` v2.0.0 (`scrfd_500m_kps`, `minifasnet_v2_pad`, `arcface_w600k_mbf`) | Model ID attestation tests | ✅ Verified (`test_enrollment_cli_model_ids_match_manifest`, `test_enrollment_cli_legacy_model_ids_absent`) |
| EN8 | Lazy initialization: non-biometric commands (`list`, `delete`) execute store-only without camera or neural models | Lazy init test | ✅ Verified (`test_list_command_works_without_camera_or_models`, `test_delete_command_works_with_store_only`) |
| EN9 | Deterministic hardware camera addressing defaulting to `/dev/v4l/by-id/` (Criterion C4) | Hardware path resolution test | ✅ Verified (`test_camera_device_path_uses_stable_by_id`) |
| EN10 | Strict path validation and sanitization: blocks parent directory traversal (`..`), mandates absolute paths, enforces standard FHS prefixes, and restricts camera devices to `/dev/` | Path validation tests | ✅ Verified (`test_sanitize_path_blocks_parent_dir_traversal`, `test_sanitize_path_blocks_relative_paths`, `test_validate_fhs_path_allows_valid_system_directories`, `test_validate_fhs_path_rejects_non_fhs_locations`, `test_validate_camera_device_path_requires_dev`, `test_build_store_only_rejects_traversal_paths`, `test_build_full_service_rejects_non_dev_camera`) |
| EN11 | Subcommand root privilege enforcement & CLI bypass elimination: removal of `--skip-root-check` and systematic EUID 0 validation across `enroll`, `verify`, `delete`, and `list` | Privilege enforcement tests | ✅ Verified (`test_verify_subcommand_enforces_root_privileges`, `test_list_subcommand_enforces_root_privileges`, `test_delete_subcommand_enforces_root_privileges`, `test_cli_rejects_skip_root_check_argument`) |
| EN12 | `debug-vision` report file is created atomically (`O_CREAT \| O_EXCL \| O_NOFOLLOW`, mode `0600`); a pre-existing file or symbolic link at the output path, or a symlinked parent directory, is refused and left untouched (GitHub #149 / STO-02) | Filesystem safety tests (`debug_vision_tests.rs`) | ✅ Verified (`test_debug_vision_creates_report_with_mode_0600`, `test_debug_vision_refuses_symlink_output_and_leaves_target_untouched`, `test_debug_vision_refuses_existing_file_without_truncation`, `test_debug_vision_refuses_symlinked_parent_directory`) |
| EN13 | `debug-vision` embeds the raw camera frame (biometric data) only with the explicit `--embed-frame` flag; the default report carries detection geometry only and the embedded variant shows a privacy warning | Opt-in embedding tests (`debug_vision_tests.rs`) | ✅ Verified (`test_debug_vision_omits_frame_without_embed_flag`, `test_debug_vision_embeds_frame_only_with_explicit_flag`, `test_cli_parse_debug_vision_subcommand_flags`) |
| EN14 | `debug-vision` defaults to the root-only `DEFAULT_DEBUG_REPORT_DIR` (`/var/lib/soos/debug`, created `0700`, existing directory never `chmod`-ed, symlink refused), validates an explicit `--output` through `validate_fhs_path`, and propagates detector errors instead of reporting zero faces | Path resolution and error propagation tests (`debug_vision_tests.rs`) | ✅ Verified (`test_ensure_debug_report_dir_creates_0700_and_refuses_symlink`, `test_default_debug_report_dir_is_root_only_location`, `test_debug_vision_rejects_traversal_output_path`, `test_debug_vision_propagates_detector_error`) |

---

## Component: `admin-cli`

| # | Criterion | Test Method | Status |
|---|---|---|---|
| AD1 | Scaffolding and CLI argument parsing (`status`, `test-pam`, `logs`, global options) | Unit tests (`test_cli_parse_status_*`, `test_cli_parse_test_pam_*`, `test_cli_parse_logs_*`, `test_cli_constants_conform_to_spec`) | ✅ Verified |
| AD2 | Daemon status inspection (component readiness, PID, uptime, systemd unit state, offline reporting) | Unit tests (`test_status_query_mock_daemon_healthy`, `test_status_query_mock_daemon_component_unready`, `test_status_query_offline_daemon_does_not_panic`, `test_status_report_json_serialization`) | ✅ Verified |
| AD3 | Simulated PAM authentication cycle with latency breakdown and verdict evaluation | Unit tests (`test_simulate_pam_auth_allow`, `test_simulate_pam_auth_deny_yields_pam_ignore`, `test_simulate_pam_auth_offline_socket_fails_closed`) | ✅ Verified |
| AD4 | Log stream filtering with automatic redaction of sensitive patterns (passwords, tokens, keys, embeddings) | Unit tests (`test_redact_preserves_benign_logs`, `test_redact_masks_password_fields`, `test_redact_masks_tokens_and_secrets`, `test_redact_masks_master_key_and_hex_keys`, `test_redact_masks_embedding_vector_arrays`, `test_fetch_and_filter_logs_from_file_with_redaction`, `test_fetch_and_filter_logs_limits_line_count`) | ✅ Verified |
| AD5 | User group provisioning (`soos-admin add-user <username>`) adding user to `soos` group | Unit tests (`test_add_user_to_soos_group`) | ✅ Verified |

---

## Component: `vision-pad` (Presentation Attack Detection)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| PAD1 | **MiniFASNetV2** anti-spoofing model (`minifasnet_v2_pad`) attested in `models/manifest.toml` v2.0.0 with SHA-256 checksum, 80×80 BGR input shape, and `[1, 3]` output shape; live class index 1 (`DEFAULT_MINIFASNET_LIVE_CLASS_INDEX`) for `[PrintPhoto, Live, ScreenReplay]` class ordering (see ASG1, PLC1–PLC3) | Manifest parsing test (`manifest_tests::test_parse_workspace_manifest_file`, `manifest_tests::test_manifest_v2_model_count_and_checksum_attestation`) | ✅ Verified |
| PAD2 | `PadDetector` trait, `OrtPadDetector`, and `MockPadDetector` with numerically stable softmax and fault injection | Unit tests (`pad_tests::test_mock_pad_detector_nominal_live`, `test_mock_pad_detector_spoof_*`, `test_softmax_numerical_stability`) | ✅ Verified |
| PAD3 | Vision pipeline short-circuits on spoof detection, completely skipping embedding extraction | Pipeline unit tests (`pad_tests::test_pipeline_rejects_printed_photo_spoof`, `test_pipeline_rejects_screen_replay_spoof`) | ✅ Verified |
| PAD4 | Genuine live face candidates pass PAD and extract biometric embeddings | Pipeline unit tests (`pad_tests::test_pipeline_accepts_live_face`, `test_pad_threshold_calibration`) | ✅ Verified |
| PAD5 | FAR/FRR benchmark on test fixtures population confirms 0.0% False Accept Rate and 0.0% False Reject Rate | Benchmark test (`pad_tests::test_pad_far_frr_benchmark`) | ✅ Verified |
| PAD6 | PAD verification execution latency remains well within the 30ms budget allocated in updated `ARCHITECTURE.md` §7 for MiniFASNetV2 | Benchmark test (`pad_tests::test_pad_latency_budget_compliance`) | ✅ Verified |
| PAD7 | Daemon integration: PAD presentation attack yields `Verdict::Deny` with `ReasonClass::PadFailed` | Integration test (`pipeline_integration_tests::test_15_pad_presentation_attack_spoof_returns_deny_pad_failed`) | ✅ Verified |

---

## Component: `production-hardening` (Issue #16 / GitHub #23)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| H1 | Memory zeroization: `BiometricEmbedding`, `Frame`, and `PipelineOutput` wipe sensitive vectors and camera pixel buffers on drop | Unit tests (`zeroize_tests::test_biometric_embedding_zeroize_trait`, `frame_zeroize_tests::test_frame_zeroize_trait`) | ✅ Verified |
| H2 | Swap protection: `mlock(2)` and `mlockall(2)` page locking prevents sensitive keys and embedding vectors from being paged to disk/swap | Integration tests (`hardening_tests::test_mlock_slice_and_munlock_slice_lifecycle`, `test_locked_buffer_raii_wrapper`, `test_mlock_process_address_space_call`) | ✅ Verified |
| H3 | Systemd sandboxing validation: verifies all security directives in `soos-daemon.service` (`MemoryDenyWriteExecute`, `RestrictSUIDSGID`, `SystemCallArchitectures=native`, `Group=soos`, `StateDirectory=soos`) | Unit & integration tests (`systemd_test::test_systemd_unit_file_sandboxing_directives`, `hardening_tests::test_systemd_hardening_directives_complete`) | ✅ Verified |
| H4 | Supply chain audit: `cargo-deny` enforces license compliance, zero vulnerabilities, and blocks duplicate dependencies (`multiple-versions = "deny"`) | Integration tests (`hardening_tests::test_cargo_deny_bans_duplicate_versions`, `cargo deny check`) | ✅ Verified |

---

## Component: `async-cancel-safety` (Issue #21 / GitHub #60)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| ACS1 | Async cancellation safety on daemon socket writes: response buffer is fully serialized before transmission; request processing timeout cannot cancel mid-write or corrupt response stream | Integration test (`dispatcher_tests::test_timeout_during_write_does_not_corrupt_response`) | ✅ Verified |
| ACS2 | Response completeness validation in PAM IPC client: client tracks exact received byte counts, detects truncated length prefix or body payload, and returns `IpcError::TruncatedResponse` with fail-closed fallback to `PAM_IGNORE` | Integration test (`ipc_tests::test_pam_ipc_detects_truncated_response`) | ✅ Verified |

---

## Component: `vision-zeroize-frames` (Issue #24 / GitHub #63)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| VZF1 | Memory zeroization of RGB intermediate frame buffers: `VisionPipeline::process_frame` wraps RGB conversion from `convert_to_rgb` in `Zeroizing<Vec<u8>>` and guards intermediate aligned crops via `AlignedCropGuard`, guaranteeing heap face pixel wiping upon pipeline completion and early error exits | Unit tests (`zeroize_tests::test_rgb_buffer_zeroized_after_pipeline`, `test_pipeline_zeroizes_intermediate_buffers_on_error`) | ✅ Verified |
| VZF2 | `Zeroize` and `Drop` implementation on `VerificationOutcome`: `VerificationOutcome` implements `zeroize::Zeroize` and `Drop`, delegating to `self.output.zeroize()`, ensuring both primary and cloned outcomes deterministically clear the underlying embedding and crop | Unit test (`zeroize_tests::test_verification_outcome_zeroize_on_drop`) | ✅ Verified |
| VZF3 | Memory zeroization of neural inference input tensors: `OrtFaceDetector`, `OrtEmbeddingExtractor`, `OrtLandmarkDetector`, and `OrtPadDetector` prepare inputs in `Zeroizing<Vec<f32>>` buffers, pass zero-copy slice views (`TensorRef`) to ONNX Runtime, and deterministically zeroize all normalized face pixels post-inference and on drop | Unit test (`zeroize_tests::test_inference_input_buffers_zeroized`) | ✅ Verified |

---

## Component: `packaging` (Issues #26, #27 / GitHub #65, #66)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| PK1 | `scripts/install.sh` provisions `/var/lib/soos/{biometrics,models,evidence}` with mode `0700`/`0755` (`root:root`), `/run/soos` with mode `0750` (`root:soos`), installs binaries and the `provision-master-key` helper, generates the 32-byte `master.key` (mode `0600`) on a live install only (never under `--destdir`, see PMK1), and verifies models | Invariant tests (`test_install_script_creates_required_directories`, `test_install_script_destdir_stages_no_key_material`) | ✅ Verified |
| PK2 | PAM configuration templates for Debian (`pam-auth-update`), Fedora (`authselect`), and Arch Linux (`system-auth`) conform strictly to universal PAM stack ordering in `ARCHITECTURE.md` §5 (`pam_soos.so` before `pam_unix`, `event=password-failed` after `pam_unix`) | Invariant test (`test_pam_config_ordering_matches_spec`) | ✅ Verified |
| PK3 | `scripts/uninstall.sh` executes safe rollback, restoring PAM configuration backups, disabling systemd units, removing binaries, and preserving biometric data by default under `--keep-data` | Invariant test (`test_uninstall_restores_pam_config`) | ✅ Verified |
| PK4 | `soos-admin add-user <username>` validates POSIX username conventions and adds user to `soos` system group via `usermod -aG soos <username>` | Unit & integration tests (`test_add_user_to_soos_group`) | ✅ Verified |
| PK5 | Debian `.deb` package specification (`packaging/debian/control`, `rules`, `postinst`, `prerm`, `postrm`, `scripts/build_deb.sh`) installs complete system with group provisioning, invariant permissions, and PAM integration | Invariant & package build tests (`test_debian_packaging_specification`, `scripts/build_deb.sh`, `tests/docker/test_packages.sh`) | ✅ Verified |
| PK6 | RPM `.spec` file (`packaging/rpm/soos.spec`, `scripts/build_rpm.sh`) with `%pre`, `%post`, `%preun`, `%postun`, `%files` strict attributes (`0700` biometrics/evidence, `0600` master.key), and Fedora `authselect` custom profile template | Invariant & package build tests (`test_rpm_packaging_specification`, `scripts/build_rpm.sh`, `tests/docker/test_packages.sh`) | ✅ Verified |
| PK7 | Arch Linux `PKGBUILD` and `soos.install` (`packaging/arch/PKGBUILD`, `packaging/arch/soos.install`, `scripts/build_arch.sh`) installs systemd unit, binaries, PAM shared module, and provisions `soos` group and invariant directories | Invariant & package build tests (`test_arch_packaging_specification`, `scripts/build_arch.sh`, `tests/docker/test_packages.sh`) | ✅ Verified |

---

## Component: `physical-hardware-validation` (Issue #31 / GitHub #70)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| PH1 | Full enrollment lifecycle on physical hardware (clean slate -> enroll -> list -> verify -> delete -> clean slate) with filesystem permissions mode `0600` on template | Functional & invariant test (`tests/physical/enrollment_test.sh`, `test_physical_hardware_validation_suite_spec`) | ✅ Verified |
| PH2 | PAM stack integration and daemon coordination: genuine face returns `PAM_SUCCESS` without password prompt; occluded/wrong face returns `PAM_IGNORE` falling back to password; stopped daemon falls back to password; wrong password rejected | Functional & invariant test (`tests/physical/pam_integration_test.sh`, `test_physical_hardware_validation_suite_spec`) | ✅ Verified |
| PH3 | Multi-user identity isolation and cross-user rejection: distinct templates for User A and User B; positive auth for each identity; cross-user auth attempts strictly rejected | Functional & invariant test (`tests/physical/multi_user_test.sh`, `test_physical_hardware_validation_suite_spec`) | ✅ Verified |
| PH4 | Screen locker, display manager, and console operational validation manual covering `swaylock`, `hyprlock`, `gdm`, `login` TTY, `sudo`, latency targets, and rollback procedures | Documentation & invariant test (`tests/physical/screensaver_test.md`, `test_physical_hardware_validation_suite_spec`) | ✅ Verified |
| PH5 | Presentation Attack Detection (PAD) adversarial evaluation suite testing printed photographs, smartphone screens, video replay, and measuring APCER / BPCER error rates | Adversarial & invariant test (`tests/physical/adversarial_test.sh`, `pad_tests`, `test_physical_hardware_validation_suite_spec`) | ✅ Verified |
| PH6 | Invariant test enforcing file existence, executable permissions (`0755`), strict bash options (`set -euo pipefail`), and `--help` CLI functionality | Architectural invariant test (`test_physical_hardware_validation_suite_spec`) | ✅ Verified |

---

## Component: `distro-validation` (Issue #32 / GitHub #71)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| DV1 | Debian 12 / Ubuntu 24.04 end-to-end deployment verification: `.deb` package and `scripts/install.sh` deployment, directory permissions (`0700` biometrics/evidence, `0600` master.key), `pam-auth-update` configuration, user enrollment, nominal facial auth (`PAM_SUCCESS`), password fallback, and automated rollback | Functional & invariant test (`tests/distro/debian_ubuntu_test.sh`, `test_distro_validation_suite_spec`) | ✅ Verified |
| DV2 | Fedora 40 / RHEL 9 deployment verification with `authselect`: custom profile activation, strict preservation of `pam_faillock` (preauth and authfail hooks), `sudo` and `gdm` PAM service stack verification, and clean profile rollback | Functional & invariant test (`tests/distro/fedora_rhel_test.sh`, `test_distro_validation_suite_spec`) | ✅ Verified |
| DV3 | Arch Linux deployment verification: `PKGBUILD` packaging, `system-auth` snippet integration, Wayland screen lockers (`swaylock`, `hyprlock`) integration, password fallback, and pacman rollback | Functional & invariant test (`tests/distro/arch_linux_test.sh`, `test_distro_validation_suite_spec`) | ✅ Verified |
| DV4 | Multi-distribution orchestration runner supporting automated environment detection, direct local invocation, and Dockerized matrix execution across distributions | Orchestrator & invariant test (`tests/distro/run_distro_validation.sh`, `test_distro_validation_suite_spec`) | ✅ Verified |
| DV5 | Comprehensive distribution deployment, operational verification, and emergency recovery manual covering Debian/Ubuntu, Fedora/RHEL, and Arch Linux | Documentation & invariant test (`Docs/DISTRIBUTION_DEPLOYMENT.md`, `test_distro_validation_suite_spec`) | ✅ Verified |
| DV6 | Automated architectural security invariant test asserting distribution validation suite presence, executable permissions (`0755`), strict bash options (`set -euo pipefail`), CLI `--help` functionality, and acceptance criteria coverage | Architectural invariant test (`test_distro_validation_suite_spec`) | ✅ Verified |

---

## Component: `pam-ffi-timeout` (Issue #34 / GitHub #73)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| PFT1 | FFI Panic Safety: All exported PAM entry points (`pam_sm_authenticate`, `pam_sm_setcred`, `pam_sm_acct_mgmt`, `pam_sm_chauthtok`, `pam_sm_open_session`, `pam_sm_close_session`) and argument parsing (`parse_argv`, `parse_cstrs`) are wrapped in `catch_unwind` and systematically return `PAM_IGNORE` on panic | Unit & integration tests (`test_c_abi_all_entry_points_panic_safe`, `authenticate_catches_parse_argv_panics`, `panic_safety_returns_pam_ignore`) | ✅ Verified |
| PFT2 | Non-blocking IPC connect timeout: `connect_with_timeout` uses POSIX `poll` to enforce configured latency budget (`timeout_ms`), preventing unbounded blocking on frozen daemon sockets | Integration test (`ipc_tests::test_ipc_connect_timeout_frozen_daemon`) | ✅ Verified |
| PFT3 | Memory Zeroization: `Request` and `Event` implement `zeroize::Zeroize` and `Drop`, and temporary IPC buffers (`request_id`, `encoded`, `full_buf`) are scrubbed on drop | Unit & integration test (`ipc_tests::test_request_and_event_zeroize_on_drop`) | ✅ Verified |

---

## Component: `nextgen-models-manifest` (Issue #36 / GitHub #102)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| NGM1 | `models/manifest.toml` v2.0.0 contains exactly 3 model entries (`scrfd_500m_kps`, `arcface_w600k_mbf`, `minifasnet_v2_pad`) with valid SHA-256 checksums and tensor shapes | Manifest parsing test (`manifest_tests::test_parse_workspace_manifest_file`, `manifest_tests::test_manifest_v2_model_count_and_checksum_attestation`) | ✅ Verified |
| NGM2 | All 3 ONNX model files download successfully and pass cryptographic SHA-256 verification and dry-run validation | Script test (`scripts/download_models.sh --dry-run`, `scripts/download_models.sh --check-only`) | ✅ Verified |


---

## Component: `scrfd-face-detector` (Issue #37 / GitHub #103)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| NGM3 | `OrtScrfdDetector` parses 9 output tensors across 3 strides (8, 16, 32) with distance-to-border box decoding and startup validation | Unit tests (`scrfd_tests::test_scrfd_decode_stride8_known_output`, `scrfd_tests::test_scrfd_decode_all_strides`, `scrfd_tests::test_scrfd_rejects_invalid_output_count`, `scrfd_tests::test_scrfd_validates_shape_patterns`) | ✅ Verified |
| NGM4 | SCRFD input is BGR 640×640 with letterbox padding and `(pixel - 127.5) / 128.0` normalization | Unit & property tests (`scrfd_tests::test_letterbox_preserves_aspect_ratio`, `scrfd_tests::test_prepare_input_bgr_channel_ordering`, `scrfd_tests::test_letterbox_unproject_roundtrip`) | ✅ Verified |
| NGM5 | SCRFD detection includes 5-point landmarks in `FaceDetection` struct with coordinate un-projection | Unit tests (`scrfd_tests::test_face_detection_carries_landmarks`, `scrfd_tests::test_unproject_coordinates_match_original_image`) | ✅ Verified |

---

## Component: `remove-ort-landmark-detector` (Issue #38 / GitHub #104)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| NGM6 | `OrtLandmarkDetector` removed from `soos-inference-ort`; `FaceLandmarks`, `Point2f`, and `LandmarkDetector` trait preserved | Unit tests (`landmarks_tests::test_point2f_geometry_and_distance`, `landmarks_tests::test_face_landmarks_array_conversion_and_geometry`, `landmarks_tests::test_landmark_detector_trait_mock_dispatch`) | ✅ Verified |

---

## Component: `embedding-512d-w600k` (Issue #39 / GitHub #105)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| NGM7 | `OrtEmbeddingExtractor` uses symmetric `[-1.0, +1.0]` normalization `(pixel - 127.5) / 127.5` and `MockEmbeddingExtractor` produces 512D vectors by default | Unit tests (`embedding_tests::test_embedding_normalization_symmetric_range`, `embedding_tests::test_mock_embedding_default_512d`) | ✅ Verified |

---

## Component: `pad-minifasnet-v2` (Issue #40 / GitHub #106)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| NGM8 | `OrtPadDetector::prepare_input()` produces 80×80 NCHW BGR tensors with `pixel / 255.0` normalization into `[0.0, 1.0]`, deterministic zeroization post-inference, and 80×80 dimension validation | Unit tests (`pad_tests::test_pad_prepare_input_80x80_bgr`, `pad_tests::test_pad_normalization_0_1_range`, `pad_tests::test_pad_invalid_dimensions_message_80x80`, `zeroize_tests::test_inference_input_buffers_zeroized`) | ✅ Verified |
| NGM9 | `OrtPadDetector` defaults `live_class_index` to `DEFAULT_MINIFASNET_LIVE_CLASS_INDEX` (1, MiniFASNetV2 `[PrintPhoto, Live, ScreenReplay]`); `interpret_probabilities` supports explicit class indices (test-only constructors) with ordinal spoof attack classification (`PrintPhoto` vs `ScreenReplay`) | Unit tests (`pad_tests::test_pad_default_live_class_index_is_one`, `pad_tests::test_pad_class_ordering_live_index_0`, `pad_tests::test_pad_class_ordering_configurable`) | ✅ Verified |
| NGM10 | `OrtPadDetector` handles empty probability distributions fail-closed and preserves panic safety and numerical softmax stability | Unit tests (`pad_tests::test_softmax_numerical_stability`, `pad_tests::test_mock_pad_detector_*`) | ✅ Verified |

---

## Component: `vision-pipeline-3-model` (Issue #41 / GitHub #107)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| NGM11 | `VisionPipeline` constructs with 3 backends (detector, pad, extractor), eliminating dedicated landmark detector | Unit tests (`pipeline_tests::test_pipeline_constructs_with_three_backends`) | ✅ Verified |
| NGM12 | Pipeline extracts 5-point landmarks directly from `FaceDetection` and fails closed on missing landmarks (`VisionError::MissingLandmarks`) | Unit tests (`pipeline_tests::test_pipeline_extracts_landmarks_from_detection`, `pipeline_tests::test_pipeline_fails_when_detection_lacks_landmarks`) | ✅ Verified |
| NGM13 | Presentation Attack Detection receives 2.7× expanded bbox context crop resized to 80×80; embedding extractor receives 112×112 aligned crop | Unit tests (`pipeline_tests::test_expand_bbox_centered`, `pipeline_tests::test_expand_bbox_clamped_to_image`, `pipeline_tests::test_pipeline_pad_receives_expanded_crop`, `pipeline_tests::test_pipeline_embedding_receives_aligned_crop`) | ✅ Verified |

---

## Component: `vision-letterbox-and-bbox-crop` (Issue #42 / GitHub #108)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| NGM14 | Letterbox padding preserves aspect ratio with correct coordinate un-projection | Property test (`letterbox_tests::test_letterbox_unproject_roundtrip`, `letterbox_tests::test_letterbox_640x480_to_640x640`, `letterbox_tests::test_letterbox_1280x720_to_640x640`, `letterbox_tests::test_letterbox_square_no_padding`) | ✅ Verified |
| NGM14b | Bounding box crop and resize with bilinear interpolation and out-of-bounds zero (black) padding | Unit tests (`crop_tests::test_crop_and_resize_known_image`, `crop_tests::test_crop_and_resize_out_of_bounds_padding`, `crop_tests::test_crop_and_resize_degenerate_bbox_returns_black`, `crop_tests::test_expand_bbox_for_pad_expansion_and_clamping`) | ✅ Verified |

---

## Component: `model-ids-nextgen` (Issue #43 / GitHub #109)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| NGM15 | Model registry IDs in `daemon` and `enrollment-cli` updated to 3-model architecture (`scrfd_500m_kps`, `minifasnet_v2_pad`, `arcface_w600k_mbf`), eliminating `landmark_5point`, with `OrtScrfdDetector` instantiation and error propagation | Unit & integration tests (`test_enrollment_cli_model_ids_match_manifest`, `test_enrollment_cli_legacy_model_ids_absent`, `test_daemon_refuses_start_with_missing_models`, `test_daemon_refuses_start_with_tampered_models`, `test_models_readme_complete_and_accurate`) | ✅ Verified |

---

## Component: `mock-backends-nextgen` (Issue #44 / GitHub #110)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| NGM16 | Mock backends produce detections with landmarks, 512D embeddings, compatible PAD results, and 3-backend test construction sites | Unit tests (`detector_tests::test_mock_detector_returns_landmarks`, `detector_tests::test_mock_face_detector_canonical_landmarks_for_box`, `embedding_tests::test_mock_embedding_default_512d`, `pad_tests::test_mock_pad_detector_*`, `pipeline_tests::test_pipeline_constructs_with_three_backends`) | ✅ Verified |

---

## Component: `nextgen-model-documentation` (Issue #45 / GitHub #111)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| NGM17 | `AI/ARCHITECTURE.md` §1 and §7, `AI/VERIFICATION_MATRIX.md` (NGM1–NGM17), `Docs/INFERENCE_ORT_CRATE.md`, and `Docs/VISION_CRATE.md` consistently reflect the 3-model pipeline (SCRFD 500M KPS + ArcFace w600k MBF 512D + MiniFASNetV2 80×80), eliminating all legacy references to UltraFace Slim 320, `landmark_5point`, and MobileFaceNet 128D | Documentation audit cross-referencing ADR register (`AI/DECISIONS.md`), manifest v2.0.0 model IDs, and implementation tests (`manifest_tests::test_parse_workspace_manifest_file`, `test_enrollment_cli_model_ids_match_manifest`, `test_enrollment_cli_legacy_model_ids_absent`) | ✅ Verified |

---

## Component: `guided-enrollment-production-unlock` (Issue #22 / GitHub #61)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| GEPU1 | Template import via `soos-enroll import`: accepts JSON float arrays and CBOR `BiometricTemplate`, validates 512D ArcFace embeddings, encrypts with master key, and stores in `/var/lib/soos/biometrics/<uid>.bio` | Contractual integration tests (`import_tests::test_import_embedding_success`, `import_tests::test_import_embedding_dimension_mismatch_fails`, `import_tests::test_import_nonexistent_file_fails`) | ✅ Verified |
| GEPU2 | Config-driven camera device resolution: resolves camera device prioritizing CLI arguments, then daemon config (`/etc/soos/daemon.toml`), then stable `/dev/v4l/by-id/` (preserving Criterion C4) | Contractual integration tests (`import_tests::test_resolve_camera_device_from_daemon_config`, `model_id_tests::test_camera_device_path_uses_stable_by_id`) | ✅ Verified |
| GEPU3 | Hardware camera arbitration between `soos-daemon` and GUI: detects when daemon is holding `/dev/video0`, supports pausing and resuming daemon via Polkit, preventing `EBUSY` crashes on startup | Integration & interactive GUI tests (`SoosApp::is_daemon_active`, `SoosApp::pause_daemon`, `SoosApp::resume_daemon`) | ✅ Verified |
| GEPU4 | Guided enrollment production unlock integration: unprivileged `soos-gui` exports composite template and invokes `pkexec soos-enroll import` to write to `/var/lib/soos/biometrics`, allowing real PAM unlock with background `soos-daemon` | Physical verification (`soos-enroll import`, `soos-enroll verify`, PAM `sudo` authentication) | ✅ Verified |

---

## Component: `biometric-reliability-camera-lifecycle-and-proxy` (Issue #46 / GitHub #132)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| BIO1 | ArcFace ONNX input channel ordering (BGR rather than RGB), threshold calibration (`match_threshold: 0.70`, `pad_threshold: 0.85`) across `VisionPipelineConfig` and GUI/CLIs, and hardened `expand_bbox_for_pad` bounds clamping | Unit tests (`embedding_tests::test_prepare_input_bgr_channel_ordering`, `pipeline_tests::test_vision_pipeline_default_thresholds_calibrated`, `pad_tests::test_pad_threshold_calibration`, `crop_tests::test_expand_bbox_for_pad_expansion_and_clamping`) | ✅ Verified |
| CAM1 | Hardware IR camera prioritization (`sensor_preference = "prefer_ir"` / `SensorPreference`) in daemon config and clean `PixelFormat::Grey` pipeline ingestion | Unit & configuration tests (`config_tests::test_config_sensor_preference_and_idle_timeout`, `pipeline_init_tests::test_pipeline_init_device_selection_prefers_ir`, `color_tests::test_grey_to_rgb_conversion`) | ✅ Verified |
| CAM2 | Camera power lifecycle & on-demand auto-standby (`Active` -> `Idle` -> `Suspended`), releasing V4L2 device file descriptor after `idle_timeout` (default 10s) to extinguish privacy LED, instant wake on `notify_activity()`, and 1000ms PAM timeout | Unit & integration tests (`mock_camera_tests::test_mock_camera_auto_suspend_and_resume_lifecycle`, `mock_camera_tests::test_mock_camera_idle_throttling_and_wake`, `shutdown_tests::test_is_ready_memory_visibility_acquire_release`, `pam_config_tests::test_default_timeout_is_1000ms`) | ✅ Verified |
| PRX1 | Bounded daemon video proxy (`RequestKind::PreviewFrame`, `PreviewResponse`, 2 MiB boundary) with lock-free `ArcSwapOption` load and zero sensitive keyword logging | Unit & integration tests (`preview_tests::test_preview_request_and_response_roundtrip`, `preview_tests::test_preview_frame_large_payload_rejected`, `dispatcher_tests::test_dispatcher_handles_preview_frame_request`, `logging_audit_test::test_daemon_source_code_has_zero_sensitive_data_in_logs`) | ✅ Verified |
| PRX2 | `soos-gui` `IpcCameraManager` querying `/run/soos/daemon.sock` without root privileges, eliminating `pkexec systemctl stop soos-daemon` Polkit prompt and V4L2 `EBUSY` conflict | Integration & unit tests (`crates/gui/src/ipc_camera.rs`, `crates/gui/src/main.rs`, `cargo test -p soos-gui`) | ✅ Verified |

---

## Component: `anti-spoof-ir-gdm-integration` (Issue #47 / GitHub #134)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| ASG1 | MiniFASNet live class index alignment (class index 1 = live, class 0 = print photo, class 2 = screen replay), preventing screen replay spoof attacks | Unit & integration tests (`pad_tests::test_pad_default_live_class_index_is_one`, `pad_tests::test_screen_replay_detected_as_spoof_with_default_index` in `crates/inference-ort`, `pad_tests::test_pipeline_rejects_screen_replay_spoof` in `crates/vision`; production wiring covered by PLC1–PLC3) | ✅ Verified |
| ASG2 | Aspect-ratio preserving ROI expansion for PAD without distortion (`expand_bbox_for_pad` using Minivision shifting algorithm `_get_new_box`) | Unit tests (`pad_tests::test_expand_bbox_shifts_roi_without_distortion`, `crop_tests::test_expand_bbox_for_pad_expansion_and_clamping`) | ✅ Verified |
| ASG3 | Dual-sensor hardware IR camera preference defaulted in `camera-v4l` (`SensorPreference::PreferIr`) and 31-character truncated V4L2 device name classification (`USB2.0 FHD UVC WebCam: USB2.0 I`) | Unit tests (`dual_sensor_tests::test_sensor_preference_defaults_to_prefer_ir`, `dual_sensor_tests::test_v4l2_31_char_truncated_name_classified_as_ir`) | ✅ Verified |
| ASG4 | Latency budget & auto-standby wake calibration (daemon pipeline decision budget 900ms, wake timeout 800ms) ensuring reliable first-attempt authentication within PAM 1000ms deadline | Unit tests (`config_tests::test_daemon_pipeline_decision_budget_calibrated_for_warmup`, `config_tests::test_daemon_camera_config_defaults_prefer_ir`) | ✅ Verified |
| ASG5 | GDM integration with safe disable toggle: PAM module supports `is_disabled()` via `/etc/soos/disabled`, service-specific `/etc/soos/gdm.disable` (for `gdm-password`), and `disabled` argument; CLI provides `soos-admin gdm status\|enable\|disable` to eliminate lockout risk | Unit tests (`config_tests::test_parse_disabled_arg_and_file_check`, `config_tests::test_gdm_disable_file_triggers_disabled_state`, `gdm_tests::test_gdm_status_unconfigured`, `gdm_tests::test_gdm_status_configured_enabled`, `gdm_tests::test_gdm_status_configured_disabled`, `gdm_tests::test_gdm_enable_idempotent`, `gdm_tests::test_gdm_disable_creates_flag`) | ✅ Verified |

---

## Component: `gdm-lockscreen-feedback-and-stability` (Issue #48 / GitHub #136)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| LSF1 | GDM lock screen 2500ms timeout configuration in `soos-admin gdm enable` (`GDM_PAM_LINE` includes `timeout_ms=2500`) allowing adequate time for camera wake and multi-frame processing | Unit tests (`gdm_tests::test_gdm_pam_line_includes_timeout_ms_2500`, `gdm_tests::test_gdm_enable_idempotent`) | ✅ Verified |
| LSF2 | Configurable `warmup_frames` from `daemon.toml` and explicit support for `camera_device = "auto"` sentinel preserving auto-resolution and IR prioritization | Unit tests (`config_tests::test_pipeline_config_warmup_frames_from_toml`, `config_tests::test_pipeline_config_camera_device_auto_resolution`) | ✅ Verified |
| LSF3 | GDM/lockscreen interactive feedback via `PAM_TEXT_INFO` conversation callback with safe pointer guard (`addr >= 0x10000`) and distinct status messages (`[soos] Looking for face...`, `[soos] Face recognized. Unlocking...`, `[soos] Face not recognized.`, `[soos] Biometric spoof detected.`, `[soos] Face verification timed out.`, `[soos] Camera unavailable.`) | Unit tests (`config_tests::test_send_pam_info_null_safe`, `config_tests::test_send_pam_info_low_address_guard`) | ✅ Verified |
| LSF4 | Multi-frame evaluation loop in daemon dispatcher recovering from initial empty capture or low score within client budget, and dynamic decision budget bounding strictly clamped by `connection_timeout` | Integration tests (`pipeline_integration_tests::test_48_multi_frame_evaluation_recovers_from_initial_no_face_to_allow`, `pipeline_init_tests::test_daemon_startup_initializes_all_pipeline_components`) | ✅ Verified |

---

## Component: `camera-latency-lockscreen-gui-preview` (Issue #49 / GitHub #138)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| CLP1 | Zero idle timeout (`idle_timeout_secs = 0` / `Duration::ZERO`) disables camera auto-standby and suspension completely in `v4l_impl.rs` and `mock.rs`, keeping hardware camera continuously streaming when requested | Unit tests (`mock_camera_tests::test_mock_camera_idle_timeout_zero_disables_auto_standby`, `config_tests::test_pipeline_config_idle_timeout_zero_from_toml`) | ✅ Verified |
| CLP2 | Default `warmup_frames` configured from `daemon.toml` defaults to 0 for instant camera wake without discarding initial frames, while preserving Criterion C5 (20) in programmatic `DaemonConfig::default()` | Unit tests (`config_tests::test_daemon_toml_default_warmup_frames_is_zero`, `config_tests::test_pipeline_default_sensor_preference_is_prefer_ir`, `config_tests::test_pipeline_config_warmup_frames_from_toml`) | ✅ Verified |
| CLP3 | Daemon connection dispatcher supports persistent client connections (`handle_connection` loops over requests until EOF or idle timeout), allowing streaming video clients like `soos-gui` to receive uninterrupted preview frames without broken pipe errors | Contractual integration tests (`dispatcher_tests::test_dispatcher_persistent_stream_multiple_requests`, `dispatcher_tests::test_dispatcher_timeout_on_idle_connection`, `layout_tests::test_ipc_camera_manager_receives_persistent_frames`) | ✅ Verified |
| CLP4 | `soos-gui` live preview and `IpcCameraManager` reliability: instant socket probe without latency overhead, lockscreen activity refresh in dispatcher keeps camera hot during screen wake, and extended 1000ms preview wake loop eliminates blank/failed frames | Unit & integration tests (`ipc_camera_tests`, `layout_tests::test_ipc_camera_manager_receives_persistent_frames`, `dispatcher_tests::test_dispatcher_preview_frame_roundtrip`) | ✅ Verified |

---

## Component: `gui-camera-auto-resolution-and-packaging` (Issue #50 / GitHub #140)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| GARP1 | `resolve_camera_device_from_config` dynamically resolves hardware camera device when `camera_device = "auto"`, `"default"`, or empty using `select_camera_device` with configured `sensor_preference`, falling back to `/dev/v4l/by-id/` and `/dev/video0` | Unit tests (`device_resolution_tests::test_resolve_camera_device_auto_resolution`, `device_resolution_tests::test_resolve_camera_device_default_resolution`, `device_resolution_tests::test_resolve_camera_device_explicit_path`, `device_resolution_tests::test_resolve_camera_device_ignores_literal_auto_in_cli_arg`) | ✅ Verified |
| GARP2 | V4L2 MMAP buffer payload slicing in `crates/camera-v4l/src/v4l_impl.rs` bounds buffer to `meta.bytesused`, eliminating trailing buffer padding bytes on compressed/MJPEG formats | Integration tests (`soos_camera_v4l::v4l_impl`, `tests/physical_hardware_tests.rs`) | ✅ Verified |
| GARP3 | `soos-gui` vision worker fallback converts raw camera frame to RGB24 and populates preview slot on neural pipeline error, keeping preview responsive, and sets default MiniFASNet live class index (1) | Unit tests (`layout_tests::test_gui_worker_fallback_renders_raw_rgb_on_pipeline_error`, `layout_tests::test_windowed_mode_live_inspection_layout_with_checkboxes`) | ✅ Verified |
| GARP4 | System provisioning scripts (`scripts/install.sh`, `scripts/uninstall.sh`) manage `soos-gui` binary lifecycle in `/usr/bin/soos-gui` | Invariant tests (`soos-invariants::tests::test_install_script_creates_required_directories`, `soos-invariants::tests::test_uninstall_restores_pam_config`) | ✅ Verified |

---

## Component: `pam-release-panic-unwind` (Review PAM-01 / TCI-01, GitHub #148)

Complements PA3, PA12 and PFT1: those rows are proven under `[profile.test]` (always unwinding); the rows below prove the same contract on the release-built artifact that packaging ships.

| # | Criterion | Test Method | Status |
|---|---|---|---|
| PRU1 | Root `Cargo.toml` `[profile.release]` declares `panic = "unwind"` exactly once, no workspace or `crates/pam` profile sets `panic = "abort"`, and every packaging/Docker build command (`scripts/build_*.sh`, `packaging/debian/rules`, `packaging/rpm/soos.spec`, `packaging/arch/PKGBUILD`, `tests/docker/*.sh`) uses `--release` without a custom `--profile` | Invariant test (`soos-invariants::tests::test_release_profile_unwinds_so_pam_catch_unwind_is_effective`) | ✅ Verified |
| PRU2 | The `soos-pam` `fault-injection` feature is opt-in (`fault-injection = []`, never in `default`), its module and call site are `#[cfg(feature = "fault-injection")]`-gated, and no packaging, install, Dockerfile or CI build command references it | Invariant test (`soos-invariants::tests::test_pam_fault_injection_feature_is_opt_in_and_never_packaged`) | ✅ Verified |
| PRU3 | With the feature, `fault_inject=panic` / `fault_inject=overflow` parse into `FaultInject`, and an injected panic returns `PAM_IGNORE` through every entry path (C ABI `pam_sm_authenticate`, `PamHooks::sm_authenticate`, `authenticate_with_config`), repeatedly in one process, never `PAM_SUCCESS`; without the feature the argument is inert and `PamConfig::fault_inject` stays `None` | Integration tests (`fault_injection_tests::with_feature::test_parse_cstrs_fault_inject_modes`, `test_fault_inject_panic_returns_pam_ignore_via_c_abi`, `test_fault_inject_overflow_returns_pam_ignore_via_c_abi`, `test_fault_inject_via_pam_hooks_returns_pam_ignore`, `test_fault_inject_never_yields_success`, `test_fault_inject_is_repeatable_in_same_process`; `fault_injection_tests::test_default_config_has_no_fault_injection`, `test_fault_inject_unknown_mode_is_ignored`, `without_feature::test_fault_inject_argument_without_feature_is_ignored`) | ✅ Verified |
| PRU4 | A panic inside the **release-built** `pam_soos.so` loaded by a real Linux-PAM host returns `PAM_IGNORE`: the host process is not killed (exit code never ≥ 128 / 134 SIGABRT), a valid password is accepted via `pam_unix` fallback and an invalid password is rejected, for both `fault_inject=panic` and `fault_inject=overflow` | Docker matrix T10 (`tests/docker/test_suite.sh`, run by `./run_tests.sh` and the `pam-integration` CI job) + invariant test pinning the case (`soos-invariants::tests::test_pam_docker_suite_proves_release_panic_returns_pam_ignore`). Red evidence before the fix: `pam_test_runner` aborted with exit 134 under `panic = "abort"` | ✅ Verified |

---

## Component: `pad-live-class-index-single-source` (Review finding PAD-01 / GitHub #146)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| PLC1 | `soos-daemon` builds its PAD detector through `pipeline::build_pad_detector` with the crate default live class index (`DEFAULT_MINIFASNET_LIVE_CLASS_INDEX = 1`) and the configured `vision.pad_threshold`; no literal index on the PAM authentication path | Integration tests with an in-memory ONNX session (`pad_wiring_tests::test_pipeline_pad_detector_uses_default_live_class_index`, `pad_wiring_tests::test_pipeline_pad_detector_classifies_replay_as_spoof` in `crates/daemon`, fixture `tests/fixtures/mod.rs::onnx::minimal_identity_model`) | ✅ Verified |
| PLC2 | `soos-enroll` builds its PAD detector through `service::build_pad_detector` with the crate default live class index, so enrollment and `verify` diagnostics score liveness from the same class as the daemon and the GUI (class 2 = screen replay is a spoof, class 1 = live passes) | Integration tests (`pad_wiring_tests::test_enrollment_pad_detector_uses_default_live_class_index`, `pad_wiring_tests::test_enrollment_pad_detector_classifies_replay_as_spoof`) | ✅ Verified |
| PLC3 | Repository invariant: `DEFAULT_MINIFASNET_LIVE_CLASS_INDEX` is defined once (value 1) in `crates/inference-ort/src/pad.rs`; `new_with_class_index(` / `with_live_class_index(` never appear in production code of any crate (test-only constructors), and at least three production sites construct `OrtPadDetector::new` | Invariant test (`soos-invariants::tests::test_no_pad_live_class_index_override_outside_tests`) | ✅ Verified |

## Component: `pad-multiframe-consensus` (Review finding PAD-02 / GitHub #147)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| PMC1 | Consensus constants and bounds: `DEFAULT_PAD_CONSENSUS_REQUIRED = 3` within `DEFAULT_PAD_CONSENSUS_WINDOW = 5`, `MAX_PAD_CONSENSUS_WINDOW = 32`; `PadConsensusConfig::new` rejects `required == 0`, `required > window`, `window > MAX` with `PolicyError::InvalidConsensus` | Unit tests (`pad_consensus_tests::test_pad_consensus_config_defaults_k3_within_n5`, `pad_consensus_tests::test_pad_consensus_config_rejects_invalid_bounds`) | ✅ Verified |
| PMC2 | `Allow` requires `k` consecutive passing captures (live `>= pad_threshold`, match `>= match_threshold`); fewer than `k`, a single passing capture, or no capture never authorizes | Unit & property tests (`pad_consensus_tests::test_pad_aggregator_three_consecutive_live_frames_allow`, `test_pad_aggregator_fewer_than_k_live_frames_is_pending_timeout`, `test_pad_aggregator_first_passing_frame_does_not_allow`, `test_pad_aggregator_no_frames_is_unavailable_timeout`, `test_pad_aggregator_custom_required_count_is_honored`, `prop_allow_iff_trailing_k_frames_pass_without_spoof`) | ✅ Verified |
| PMC3 | Any spoof-classified capture (PAD spoof, liveness score below threshold, or non-finite score) vetoes `Allow` for the whole request; the veto is sticky and survives any number of subsequent live captures | Unit & property tests (`pad_consensus_tests::test_pad_aggregator_alternating_spoof_live_never_allows`, `test_pad_aggregator_spoof_veto_is_sticky_for_the_request`, `test_pad_aggregator_spoof_after_consensus_reached_revokes_allow`, `test_pad_aggregator_pad_score_below_threshold_is_spoof_even_if_classified_live`, `test_pad_aggregator_non_finite_scores_fail_closed`, `prop_any_spoof_frame_prevents_allow`) | ✅ Verified |
| PMC4 | Non-spoof interruptions (`NoFace`, `MultipleFaces`, live-but-not-matching) reset the consecutive run without vetoing; retained history is bounded by the window | Unit tests (`pad_consensus_tests::test_pad_aggregator_no_face_frame_resets_consecutive_run`, `test_pad_aggregator_multiple_faces_frame_resets_consecutive_run`, `test_pad_aggregator_low_match_score_breaks_run_without_veto`, `test_pad_aggregator_history_is_bounded_by_window`) | ✅ Verified |
| PMC5 | Daemon dispatcher integration: alternating live/spoof captures, a spoof followed by live captures, and a live capture under the PAD threshold all return `Deny`/`PadFailed`; `k` consecutive live captures return `Allow`/`FaceMatch` after at least `k` PAD evaluations within `DECISION_BUDGET_MS`; existing single-spoof (`test_15_*`), stale-frame (`test_12_7_*`), rate-limit (`test_12_5_*`) and no-face recovery (`test_48_*`) contracts unchanged | Integration tests (`pipeline_integration_tests::test_147_alternating_live_spoof_pad_never_allows`, `test_147_spoof_frame_vetoes_subsequent_live_frames_for_whole_request`, `test_147_live_frame_below_pad_threshold_vetoes_request`, `test_147_k_consecutive_live_frames_allow_within_budget`) | ✅ Verified |

---

## Component: `preview-frame-authorization` (Review finding CAM-01 / DMN-02 — GitHub #143)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| PFA1 | Default configuration denies `RequestKind::PreviewFrame` to every unprivileged peer (`[preview]` absent or `enabled = false`, empty `allowed_uids`); a root peer is always authorized; the allow-list is bounded to `MAX_PREVIEW_ALLOWED_UIDS` (64) and larger configurations fail closed | Unit tests (`preview_authorization_tests::test_preview_config_defaults_deny_unprivileged_peers`, `test_authorize_preview_root_peer_always_allowed`, `test_authorize_preview_default_denies_unprivileged_peer`, `test_authorize_preview_listed_but_disabled_is_denied`, `test_preview_config_rejects_oversized_allow_list`, `test_daemon_config_default_preview_is_disabled`, `test_daemon_config_parses_preview_section`, `test_daemon_config_rejects_oversized_preview_allow_list`) | ✅ Verified |
| PFA2 | Dispatcher serves preview frames only after kernel peer verification: a peer that is disabled, not allow-listed, or declares a foreign `uid_hint` receives a standard `Response` (`ProtocolError` / `UidMismatch`, at most 59 bytes) and zero pixel bytes | Integration tests (`preview_authorization_tests::test_preview_frame_rejected_when_disabled`, `test_preview_frame_rejected_when_enabled_but_uid_not_listed`, `test_preview_frame_uid_mismatch_rejected`, `test_authorize_preview_uid_mismatch_is_denied_even_when_listed`, `test_authorize_preview_enabled_but_not_listed_is_denied`) | ✅ Verified |
| PFA3 | Unprivileged preview peers must own an active logind session (`SessionValidator`); an authorized, allow-listed peer with an active session receives real `PreviewResponse` frames from the daemon camera | Integration tests (`preview_authorization_tests::test_preview_frame_rejected_without_active_session`, `test_preview_frame_allowed_for_configured_uid_serves_frames`, migrated `dispatcher_tests::test_dispatcher_preview_frame_roundtrip`, `dispatcher_tests::test_dispatcher_persistent_stream_multiple_requests`) | ✅ Verified |
| PFA4 | Preview requests are rate limited per peer UID (`[preview] max_requests_per_sec`, default 40, 1 s window, 64 tracked UIDs) with `ProtocolError` / `RateLimited` beyond the quota | Integration test (`preview_authorization_tests::test_preview_frame_rate_limited_per_peer_uid`) | ✅ Verified |
| PFA5 | A refused preview request never wakes the camera (`notify_activity()` not called) nor enters the readiness wait loop; `PreviewResponse` and every encoded daemon response are zeroized on drop | Integration & unit tests (`preview_authorization_tests::test_preview_frame_denied_peer_does_not_wake_camera`, `preview_tests::test_preview_response_zeroize_erases_pixel_data`, `logging_audit_test::test_daemon_source_code_has_zero_sensitive_data_in_logs`) | ✅ Verified |
| PFA6 | `soos-gui` `IpcCameraManager` sends a fresh random nonce and its real UID, maps a nonce-bound refusal to `IpcPreviewError` (`Unauthorized` stops polling without reconnecting, `RateLimited` keeps polling), and `probe_preview` reports authorization before the GUI commits to the daemon proxy | Integration tests (`ipc_camera_tests::test_ipc_camera_manager_stops_on_unauthorized_response`, `test_ipc_camera_manager_keeps_polling_after_rate_limit`, `test_probe_preview_reports_unauthorized`, `test_probe_preview_accepts_authorized_reply`, `test_probe_preview_reports_io_error_when_socket_absent`, `test_ipc_preview_error_display_is_english_and_specific`, `layout_tests::test_ipc_camera_manager_receives_persistent_frames`) | ✅ Verified |

---

## Component: `package-master-key-isolation` (GitHub #144 / review finding ONB-01)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| PMK1 | `scripts/install.sh --destdir <stage>` never generates key material inside the staging tree: no `*.key` file anywhere under the stage, `var/lib/soos/master.key` absent, and the `usr/libexec/soos/provision-master-key` helper staged with mode `0755` | Invariant tests (`soos-invariants::tests::test_install_script_destdir_stages_no_key_material`, `soos-invariants::tests::test_install_script_creates_required_directories`) | ✅ Verified |
| PMK2 | The shared helper `scripts/provision_master_key.sh` (`/usr/libexec/soos/provision-master-key`) generates a 32-byte non-zero key with mode `0600` from inception (`umask 077`, private temporary file, atomic hard-link publish), is idempotent (an existing key is never overwritten, its mode is tightened to `0600`) and leaves no temporary file | Invariant test (`soos-invariants::tests::test_provision_master_key_helper_generates_0600_key_once`) | ✅ Verified |
| PMK3 | The helper fails closed on a symlink or a non-regular file at the key path and never writes through the symlink target | Invariant test (`soos-invariants::tests::test_provision_master_key_helper_refuses_symlink_and_non_regular`) | ✅ Verified |
| PMK4 | Every native package generates the key on the target host at first install: `packaging/debian/postinst` (`configure`), `packaging/arch/soos.install` (`post_install`) and `packaging/rpm/soos.spec` (`%post`) call the shipped helper; the helper is installed by `install.sh`, `PKGBUILD` and the RPM `%install`/`%files` (with `master.key` kept `%ghost`) and removed by `scripts/uninstall.sh`; no installer or scriptlet keeps an inline key generator | Invariant tests (`soos-invariants::tests::test_package_scriptlets_provision_key_via_shared_helper`, `soos-invariants::tests::test_uninstall_restores_pam_config`) | ✅ Verified |
| PMK5 | Package builders (`scripts/build_deb.sh`, `scripts/build_arch.sh`, `packaging/debian/rules`) run the fail-closed guard `scripts/check_no_key_material.sh` on the staged tree; the guard rejects any `*.key` file at any depth and a missing tree, and accepts a clean tree | Invariant test (`soos-invariants::tests::test_package_builders_refuse_staged_key_material`) | ✅ Verified |
| PMK6 | Built artifacts contain no key entry (`dpkg-deb -c`, `rpm -qlp --noghost`, `bsdtar -tf`), the installed key is `0600 root:root` and 32 bytes, it survives package removal, and two fresh installs of the same artifact yield distinct keys | Docker package test (`tests/docker/test_packages.sh`: `verify_package_has_no_key_material`, `verify_key_survives_removal`, `verify_fresh_install_generates_distinct_key`); executed manually in `ubuntu:24.04` and `archlinux:base` containers (walkthrough 83 §8) | ✅ Verified |
