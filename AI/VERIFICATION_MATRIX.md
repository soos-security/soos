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

---

## Component: `pam` (cdylib)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| PA1 | Returns `PAM_IGNORE` when daemon is unavailable | Integration test (`test_ipc_offline_daemon_returns_ignore`), Docker `pamtester` | ☑ Validated |
| PA2 | Returns `PAM_IGNORE` on timeout (> 250ms) | Simulated slow daemon test (`test_ipc_slow_daemon_timeout`), Docker T2/T3 | ✅ Verified |
| PA3 | `catch_unwind` wraps all FFI entry points | Code review & panic tests | ☑ Validated |
| PA4 | NEVER starts Tokio runtime | Invariant test (`test_pam_crate_has_no_tokio_dependency`) | ☑ Validated |
| PA5 | Zero `unwrap()` or `expect()` in production code | Invariant test (`test_pam_crate_has_no_unwraps_or_expects`) | ☑ Validated |
| PA6 | Neither reads nor transmits passwords | Invariant test & code audit | ☑ Validated |
| PA7 | Correct C ABI (loadable by Linux-PAM) | Docker `pamtester` & `pam_test_runner` T1 test & ABI symbol verification | ✅ Verified |
| PA8 | Absent module = PAM authentication remains functional | Docker T8 test | ✅ Verified |
| PA9 | Returns `PAM_IGNORE` on daemon crash mid-request | Unit & integration tests (`test_ipc_daemon_crash_immediate_disconnect_returns_ignore`, `test_ipc_daemon_crash_partial_header_returns_ignore`, `test_ipc_daemon_crash_truncated_body_returns_ignore`), Docker T4/T5 | ✅ Verified |
| PA10 | Multi-distribution PAM stack integration across Debian/Ubuntu, RHEL/Fedora, and Arch Linux | Invariant test (`test_pam_docker_matrix_files_and_distro_configs_exist`), Docker matrix runner (`tests/docker/run_matrix.sh`) | ✅ Verified |
| PA11 | `pam-bindings` 0.3.0 `PamHooks` trait implementation with unhandled hook defaults | Unit & integration tests (`test_pam_hooks_unhandled_hooks_return_ignore`, `test_pam_hooks_authenticate_offline_daemon_returns_ignore`, `test_pam_crate_uses_pam_bindings_and_implements_pam_hooks`) | ✅ Verified |
| PA12 | Syslog panic logging on caught panics without secret leakage | Unit & integration tests (`test_syslog_panic_message_formatting`, `test_syslog_panic_message_sanitization`, `test_syslog_log_panic_execution`, `test_pam_crate_has_syslog_panic_logging_without_secrets`) | ✅ Verified |

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
| EN7 | Model registry ID attestation matching `models/manifest.toml` (`ultraface_slim_320`, `landmark_5point`, `mobilefacenet_arcface`, `minifasnet_pad`) | Model ID attestation test | ✅ Verified (`test_enrollment_cli_model_ids_match_manifest`) |
| EN8 | Lazy initialization: non-biometric commands (`list`, `delete`) execute store-only without camera or neural models | Lazy init test | ✅ Verified (`test_list_command_works_without_camera_or_models`, `test_delete_command_works_with_store_only`) |
| EN9 | Deterministic hardware camera addressing defaulting to `/dev/v4l/by-id/` (Criterion C4) | Hardware path resolution test | ✅ Verified (`test_camera_device_path_uses_stable_by_id`) |

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
| PAD1 | MiniFASNet anti-spoofing model attested in `models/manifest.toml` with SHA-256 checksum and input/output shapes | Manifest parsing test (`manifest_tests::test_parse_workspace_manifest_file`) | ✅ Verified |
| PAD2 | `PadDetector` trait, `OrtPadDetector`, and `MockPadDetector` with numerically stable softmax and fault injection | Unit tests (`pad_tests::test_mock_pad_detector_nominal_live`, `test_mock_pad_detector_spoof_*`, `test_softmax_numerical_stability`) | ✅ Verified |
| PAD3 | Vision pipeline short-circuits on spoof detection, completely skipping embedding extraction | Pipeline unit tests (`pad_tests::test_pipeline_rejects_printed_photo_spoof`, `test_pipeline_rejects_screen_replay_spoof`) | ✅ Verified |
| PAD4 | Genuine live face candidates pass PAD and extract biometric embeddings | Pipeline unit tests (`pad_tests::test_pipeline_accepts_live_face`, `test_pad_threshold_calibration`) | ✅ Verified |
| PAD5 | FAR/FRR benchmark on test fixtures population confirms 0.0% False Accept Rate and 0.0% False Reject Rate | Benchmark test (`pad_tests::test_pad_far_frr_benchmark`) | ✅ Verified |
| PAD6 | PAD verification execution latency remains well within the 35ms budget allocated in `ARCHITECTURE.md` §7 | Benchmark test (`pad_tests::test_pad_latency_budget_compliance`) | ✅ Verified |
| PAD7 | Daemon integration: PAD presentation attack yields `Verdict::Deny` with `ReasonClass::PadFailed` | Integration test (`pipeline_integration_tests::test_15_pad_presentation_attack_spoof_returns_deny_pad_failed`) | ✅ Verified |

---

## Component: `production-hardening` (Issue #16 / GitHub #23)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| H1 | Memory zeroization: `BiometricEmbedding`, `Frame`, and `PipelineOutput` wipe sensitive vectors and camera pixel buffers on drop | Unit tests (`zeroize_tests::test_biometric_embedding_zeroize_trait`, `frame_zeroize_tests::test_frame_zeroize_trait`) | ✅ Verified |
| H2 | Swap protection: `mlock(2)` and `mlockall(2)` page locking prevents sensitive keys and embedding vectors from being paged to disk/swap | Integration tests (`hardening_tests::test_mlock_slice_and_munlock_slice_lifecycle`, `test_locked_buffer_raii_wrapper`, `test_mlock_process_address_space_call`) | ✅ Verified |
| H3 | Systemd sandboxing validation: verifies all security directives in `soos-daemon.service` (`MemoryDenyWriteExecute`, `RestrictSUIDSGID`, `SystemCallArchitectures=native`) | Unit & integration tests (`systemd_test::test_systemd_unit_file_sandboxing_directives`, `hardening_tests::test_systemd_hardening_directives_complete`) | ✅ Verified |
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

## Component: `packaging` (Issue #26 / GitHub #65)

| # | Criterion | Test Method | Status |
|---|---|---|---|
| PK1 | `scripts/install.sh` provisions `/var/lib/soos/{biometrics,models,evidence}` with mode `0700`/`0755` (`root:root`), `/run/soos` with mode `0750` (`root:soos`), installs binaries, generates 32-byte `master.key` (mode `0600`), and verifies models | Invariant test (`test_install_script_creates_required_directories`) | ✅ Verified |
| PK2 | PAM configuration templates for Debian (`pam-auth-update`), Fedora (`authselect`), and Arch Linux (`system-auth`) conform strictly to universal PAM stack ordering in `ARCHITECTURE.md` §5 (`pam_soos.so` before `pam_unix`, `event=password-failed` after `pam_unix`) | Invariant test (`test_pam_config_ordering_matches_spec`) | ✅ Verified |
| PK3 | `scripts/uninstall.sh` executes safe rollback, restoring PAM configuration backups, disabling systemd units, removing binaries, and preserving biometric data by default under `--keep-data` | Invariant test (`test_uninstall_restores_pam_config`) | ✅ Verified |
| PK4 | `soos-admin add-user <username>` validates POSIX username conventions and adds user to `soos` system group via `usermod -aG soos <username>` | Unit & integration tests (`test_add_user_to_soos_group`) | ✅ Verified |


