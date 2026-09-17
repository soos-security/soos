# Walkthrough 34 — Enrollment CLI Model IDs and Lazy Initialization

- **Issue**: #19 (`fix(enrollment-cli): Correct model registry IDs and lazy initialization`)
- **GitHub Issue**: #58
- **Branch**: `fix/enrollment-cli-model-ids`
- **Component**: `enrollment-cli`
- **Verification Criteria**: `EN7`, `EN8`, `EN9`

---

## 1. Overview & Problem Statement

The `soos-enrollment-cli` tool (`soos-enroll`) previously exhibited three critical issues:
1. **Model Registry ID Mismatch (#19.1)**: Eager model session loading requested legacy identifiers (`face_detector`, `facial_landmarks`, `face_embedding`) that did not correspond to the attested entries in `models/manifest.toml` (`ultraface_slim_320`, `landmark_5point`, `mobilefacenet_arcface`, `minifasnet_pad`), causing session resolution failures.
2. **Eager Initialization Bottleneck (#19.2)**: `build_service` eagerly spawned the V4L2 camera capture loop and loaded all 4 ONNX models even for non-biometric administrative operations (`list`, `delete`). As a consequence, running `soos-enroll list` or `soos-enroll delete` failed on headless nodes or test systems lacking camera hardware or model files.
3. **Unstable Camera Path Default (#19.3)**: Defaulted to `/dev/video0` rather than a persistent `/dev/v4l/by-id/` hardware identifier, violating Criterion C4.

---

## 2. Multi-Agent TDD Implementation

### Phase 1: Architecture & Specification
- Refactored `EnrollmentService` to maintain optional camera and vision pipeline handles:
  - `store: Arc<BiometricStore>`
  - `camera: Option<Arc<dyn CameraManager>>`
  - `pipeline: Option<Arc<VisionPipeline>>`
  - `require_root: bool`
- Exposed two distinct service builders:
  - `build_store_only(&cli)`: Allocates `MasterKey` and `BiometricStore` only; leaves camera and vision pipeline uninitialized.
  - `build_full_service(&cli)`: Allocates store, resolves camera hardware via `/dev/v4l/by-id/`, verifies `ModelRegistry` integrity, and instantiates all 4 attested ONNX sessions.
- Added typed error variants to `EnrollmentCliError`:
  - `CameraNotInitialized`: returned if camera capture is invoked on store-only service.
  - `PipelineNotInitialized`: returned if vision processing is invoked on store-only service.
- Implemented `resolve_camera_device(cli_device: Option<PathBuf>) -> PathBuf`:
  - Checks explicit CLI override first.
  - Scans `/dev/v4l/by-id/` deterministically for hardware symlinks.
  - Falls back to `/dev/v4l/by-id/default-camera` per Criterion C4.

### Phase 1.5: Plan Evaluation
- The Plan Evaluator Sub-Agent audited the implementation plan against `AI/ARCHITECTURE.md` across the 6 architectural pillars in `AI/plan_evaluator_report.md` and issued `VALIDATION_VERDICT: APPROVED`.

### Phase 2: Contractual Tests (Red Phase)
- Authored automated tests before production code implementation:
  - `tests/model_id_tests.rs`:
    - `test_enrollment_cli_model_ids_match_manifest`: Asserts all 4 model IDs match official `models/manifest.toml`.
    - `test_camera_device_path_uses_stable_by_id`: Verifies `/dev/v4l/by-id/` prefix and explicit override precedence.
  - `tests/list_tests.rs`:
    - `test_list_command_works_without_camera_or_models`: Confirms `build_store_only` and `service.list()` succeed when camera and models are absent.
  - `tests/delete_tests.rs`:
    - `test_delete_command_works_with_store_only`: Confirms template deletion and secure shredding work in store-only mode.
- Verified initial compilation failure during the TDD Red Phase.

### Phase 3: Security & Panic Safety Audit
- Verified zero `unwrap()` or `expect()` in library production code.
- Enforced `#![forbid(unsafe_code)]` across `lib.rs` and `main.rs`.
- Confirmed zero plaintext embeddings or keys logged to console.

### Phase 4: Developer Implementation (Green Phase)
- Implemented model registry constants and resolution in `crates/enrollment-cli/src/service.rs`.
- Implemented `build_store_only`, `build_full_service`, and `build_service`.
- Updated `crates/enrollment-cli/src/main.rs`:
  - `Commands::List` and `Commands::Delete` invoke `build_store_only(&cli)`.
  - `Commands::Enroll` and `Commands::Verify` invoke `build_full_service(&cli)`.
- Re-exported all builders and constants in `crates/enrollment-cli/src/lib.rs`.
- Formatted with `cargo fmt` and confirmed zero Clippy warnings (`cargo clippy --all-targets --all-features -- -D warnings`).

### Phase 5: Independent Candid Review
- Executed `./scripts/candid_review.sh` and authored `AI/candid_review_report.md` with `VERDICT: APPROVED`.

---

## 3. Acceptance & Verification Evidence

All 30 unit and integration tests in `soos-enrollment-cli` pass cleanly:

```bash
$ cargo test -p soos-enrollment-cli
     Running tests/delete_tests.rs
test test_delete_command_works_with_store_only ... ok
test test_delete_existing_template_with_auto_confirm ... ok
test test_delete_interactive_prompt_cancelled ... ok
test test_delete_non_existent_uid_fails ... ok

     Running tests/enroll_tests.rs
test test_enroll_already_enrolled_overwrite_rejected ... ok
test test_enroll_interactive_confirmation_rejected ... ok
test test_enroll_nominal_with_auto_confirm ... ok

     Running tests/list_tests.rs
test test_list_command_works_without_camera_or_models ... ok
test test_list_empty_store_returns_empty_vec ... ok
test test_list_multiple_enrolled_users_returns_sorted_summaries ... ok

     Running tests/model_id_tests.rs
test test_camera_device_path_uses_stable_by_id ... ok
test test_enrollment_cli_model_ids_match_manifest ... ok

     Running tests/quality_tests.rs
test test_quality_fails_when_all_frames_below_confidence ... ok
test test_quality_ignores_multi_face_or_zero_face_frames ... ok
test test_quality_selects_highest_scoring_single_face ... ok

     Running tests/root_check_tests.rs
test test_root_check_bypassed_when_flag_disabled ... ok
test test_root_check_enforced_against_euid ... ok
test test_root_required_error_message ... ok

     Running tests/scaffold_tests.rs
test test_cli_global_options ... ok
test test_cli_parse_delete_subcommand ... ok
test test_cli_parse_enroll_subcommand_with_uid ... ok
test test_cli_parse_enroll_subcommand_with_username ... ok
test test_cli_parse_list_subcommand_table_and_json ... ok
test test_cli_parse_verify_subcommand ... ok

     Running tests/shred_tests.rs
test test_shred_empty_file_removes_cleanly ... ok
test test_shred_nonexistent_file_returns_error ... ok
test test_shred_overwrites_and_removes_file ... ok

     Running tests/verify_tests.rs
test test_verify_matching_user_reports_allow_and_metrics ... ok
test test_verify_non_matching_user_reports_deny ... ok
test test_verify_unenrolled_uid_fails ... ok
```

| Criterion | Description | Evidence | Status |
|---|---|---|---|
| **EN7** | Model registry IDs match `manifest.toml` | `test_enrollment_cli_model_ids_match_manifest` | ✅ Verified |
| **EN8** | Lazy initialization for `list` & `delete` | `test_list_command_works_without_camera_or_models`, `test_delete_command_works_with_store_only` | ✅ Verified |
| **EN9** | Deterministic `/dev/v4l/by-id/` camera path (C4) | `test_camera_device_path_uses_stable_by_id` | ✅ Verified |
