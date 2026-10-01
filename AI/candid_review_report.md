# Candid Review Report

- **Date**: 2026-10-01
- **Target Branch**: `fix/p3-p2-remaining-batch`
- **Base (merge-base)**: `abd6016` (`origin/main`)
- **Reviewed-Diff-Fingerprint**: `1a6b44856a3d2514ae11519a6e0397699b87c5fd4c788f2a2fb021a8e324009b`
- **Audited Files**: `.agents/skills/{candid-reviewer/SKILL.md,dev-workflow/references/project-facts.md}`, `.dockerignore`, `AGENTS.md`, `AI/{ARCHITECTURE,DECISIONS,MOCK_STRATEGY,VERIFICATION_MATRIX}.md`, `AI/plan_evaluation_report.md` (deleted), `AI/walkthroughs/{135..146}_*.md`, `Cargo.lock`, `Dockerfile`, `Docs/{BIOMETRIC_STORE_CRATE,CAMERA_V4L_CRATE,CI_CD_AND_SECURITY,DAEMON,DISTRIBUTION_DEPLOYMENT,ENROLLMENT_CLI,EVIDENCE_STORE_CRATE,GUI_APPLICATION,INFERENCE_ORT_CRATE,IPC_PROTOCOL,PACKAGING_AND_PROVISIONING,PAM_MODULE,VISION_CRATE}.md`, `README.md`, `crates/admin-cli/{Cargo.toml,src/{args,camera,lib,main}.rs,tests/{camera_command_tests,status_tests,test_pam_tests,wire_tag_tests}.rs}`, `crates/biometric-store/{src/{crypto,lib,store}.rs,tests/{aad_binding_tests,aad_migration_tests}.rs}`, `crates/camera-v4l/{src/{capture,diagnostics,lib,resolver,sensor,v4l_impl}.rs,tests/camera_diagnostics_tests.rs}`, `crates/daemon/{Cargo.toml,src/{config,dispatcher,lib,logging,main,pipeline,sd_notify,shutdown}.rs,tests/{config_tests,graceful_shutdown_tests,pad_ensemble_wiring_tests,pad_evidence_tests,panic_hook_tests,pcx_wire_routing_tests,request_id_logging_tests,response_timestamp_tests,sd_notify_tests,status_memory_locked_tests,systemd_readiness_tests}.rs}`, `crates/enrollment-cli/{src/{guided_enrollment,service}.rs,tests/quality_gate_report_tests.rs}`, `crates/evidence-store/{src/{crypto,store}.rs,tests/{aad_binding_tests,aad_codec_tests}.rs}`, `crates/gui/{src/worker.rs,tests/guided_spoof_without_embedding_tests.rs}`, `crates/inference-ort/{src/embedding.rs,tests/{embedding_io_contract_tests,embedding_preprocessing_evaluation_tests,manifest_tests}.rs}`, `crates/pam/{build.rs,src/{config,ipc,lib,syslog}.rs,tests/{config_service_boundary_tests,config_warning_tests,fault_injection_tests,pam_fail_quiet_tests,pam_handle_tests,panic_hook_capture_tests,panic_hook_chain_tests,pcx_client_tag_tolerance_tests}.rs}`, `crates/policy/tests/pad_final_reason_tests.rs`, `crates/protocol/{src/{message,types}.rs,tests/{property_tests,wire_exclusivity_tests}.rs}`, `crates/vision/src/quality.rs`, `models/{README.md,manifest.toml}`, `packaging/soos-daemon.service`, `scripts/{candid_review,candid_subagent,check_build_deps,download_models,install_rustup}.sh`, `tests/distro/{arch_linux_test,debian_ubuntu_test,fedora_rhel_test,run_distro_validation}.sh`, `tests/docker/{Dockerfile.arch,Dockerfile.fedora,Dockerfile.ubuntu,mock_daemon.py,test_packages.sh,test_suite.sh}`, `tests/fixtures/embedding_onnx.rs`, `tests/invariants/src/{artifact_freshness_contract,camera_diagnostics_contract,candid_review_contract,fixtures_contract,lexing_contract,lib,model_download_size_contract,pam_hygiene_contract,review_followups_contract,rustup_bootstrap_contract,storage_aad_contract,unit_start_guard_contract}.rs`

## 1. Executive Summary

This is a re-review. The previous review (fingerprint `16c88469…a282`, CHANGES_REQUESTED) of the
same 12-branch merge found one MAJOR process defect and two MINOR defects, all now fixed:

- MAJOR 1 (untracked `.claude/worktrees/*` gitlinks in the review target): the worktrees were
  removed. `target/candid_diff.patch` has 0 `new file mode 160000` entries, `git status` is clean,
  and the working-tree fingerprint equals the committed-tree fingerprint
  (`HEAD^{tree}` → `1a6b4485…009b`), so the pre-push and CI `--rev` gates will see the same diff.
- MINOR 2 (stale `Type=simple` text): fixed in `packaging/soos-daemon.service` and
  `Docs/PACKAGING_AND_PROVISIONING.md`.
- MINOR 3 (`camera probe` opening arbitrary paths): fixed by `ensure_v4l2_char_device`.
- Suggestion on the `v4l_impl.rs` comment: applied. The `StartLimitIntervalSec` suggestion is
  deferred to a follow-up issue, which is acceptable for a SUGGESTION.

The only change since the previous review is commit `2dc63a6` (7 files, +75/-9:
`AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/142_camera_diagnostics_command.md`,
`Docs/PACKAGING_AND_PROVISIONING.md`, `crates/camera-v4l/src/{diagnostics,v4l_impl}.rs`,
`crates/camera-v4l/tests/camera_diagnostics_tests.rs`, `packaging/soos-daemon.service`), checked
with `git diff --stat 71d161e HEAD`; the rest of the diff is byte-identical to the previously
reviewed tree. The full-pillar analysis of the previous review still applies to it and is
summarised in §3.

Local evidence: `./scripts/candid_review.sh` (Layer 1) PASSED. Tests passed with 0 failures:
`cargo test --locked --all-features` for `soos-camera-v4l`, `soos-admin-cli` and `soos-invariants`
(this pass), and for `soos-pam`, `soos-biometric-store`, `soos-evidence-store` and `soos-protocol`
(previous pass, code unchanged). `cargo clippy -p soos-camera-v4l --all-targets --all-features --
-D warnings` is clean.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

Delta since the previous review: one test **added**
(`camera_diagnostics_tests::test_cdx_system_probe_refuses_non_v4l2_nodes_before_open`), none
removed or modified. Full-branch listing, unchanged from the previous review:

| Location | Change | Justification |
|---|---|---|
| `crates/pam/tests/pam_handle_tests.rs:342-350` | offline case asserts `messages.is_empty()` | Owner pre-approved (#221); stricter than before |
| `crates/daemon/tests/config_tests.rs:200-205` | `20` → `DAEMON_DEFAULT_WARMUP_FRAMES` | Owner pre-approved (#205) |
| `crates/admin-cli/tests/{status_tests,wire_tag_tests}.rs` + third literal | `memory_locked: false` added | Owner pre-approved (#201); additions only |
| `crates/admin-cli/tests/test_pam_tests.rs:60,110` | timeout 250 → 5000 ms | Owner pre-approved (#285) |
| `crates/pam/tests/fault_injection_tests.rs:97-99` | SAFETY comment reworded | Owner pre-approved |
| `crates/inference-ort/tests/manifest_tests.rs:77` | licence `MIT` → `NOASSERTION` | Owner pre-approved (#278); still exact equality |
| `tests/distro/*.sh`, `tests/docker/test_packages.sh` | skip-build guard removed (always build) | Owner pre-approved (#244); stronger |
| `tests/docker/Dockerfile.*`, `test_suite.sh`, `mock_daemon.py` | verified rustup bootstrap, comment, unknown-tag parity | Harness hardening; no check removed |
| `tests/invariants/src/{fixtures_contract,lexing_contract}.rs` | doc-comment rewording | Owner pre-approved |

No new `#[ignore]`, `#[cfg(any())]`, `should_panic`, `tolerance` or `epsilon` in tests; the only
new inline test is the additive `capture.rs::test_rfx_dequeue_interrupted_does_not_count_as_stall`.
**No unapproved weakening.**

## 3. Deep Reasoning Audit

### Logic & Architecture

- Delta: `ensure_v4l2_char_device` runs first in `system_details`, uses `std::fs::metadata`
  (follows symlinks, so a `/dev/v4l/by-id/` link is checked on its target), and accepts only
  `is_char_device()` with `libc::major(rdev) == 81`. Scenarios tried: regular file, `/dev/null`
  (char device, major 1), missing path (still `NotFound` through `from_io_error`), by-id symlink
  to a video node (accepted). PASS.
- Remaining stat → open window (path swapped between `stat` and `open`): only reachable by
  someone who can already rewrite the operator-supplied path; the tool is a non-setuid operator
  CLI. Acceptable.
- The unit comment now matches `Type=notify`; the contradictory doc row is gone. PASS.
- Rest of the branch, unchanged: PAM argv warnings and `pam_sm_authenticate` → `sm_authenticate`
  routing; AAD binding (copied, UID-rewritten, legacy cross-UID and moved-evidence scenarios all
  refused; rollback documented as out of scope, SAD7); `build_response` fail-closed clock handling;
  spoof evidence opt-in; optional PAD member manifest-gated; resolver on top of
  `explain_camera_resolution`. PASS.

### PAM Concurrency & Deadlines

- The delta does not touch `crates/pam`. Unchanged: no async/threads, cumulative deadline on every
  read/write, the removed completeness check was dead code (`read_exact_before_deadline` returns
  `TruncatedResponse` on EOF), fail-quiet keeps `PAM_IGNORE`. PASS.

### Panic Safety & Fail-Closed

- Delta: no `unwrap`/`expect`; `ensure_v4l2_char_device` runs inside the existing `catch_unwind`
  of `SystemV4lDeviceProbe::details`. Unchanged: every PAM entry point goes through
  `syslog::catch_entry`; `PAM_SUCCESS` only for `!verdict.should_ignore()` (= `Allow`). PASS.

### Test Integrity & Anti-Weakening

- The new CDX11 test would fail against the previous implementation: a `0o000` regular file
  gives `PermissionDenied` (non-root) or an ioctl error, not the expected
  `Other(Some(ENOTTY))`, and `/dev/null` would be opened and give `ENOTTY` only after `open(2)`.
  The regular-file assertion therefore pins "refused before open". When run as root (Docker CI) the
  assertions still hold because the guard runs before `open`. PASS.

### Memory, Bounds & Secrets

- Delta adds no allocation and no logging. Unchanged: nonce logged only as `short_request_id`,
  daemon panic hook logs the location only, luma plane zeroized, embedding length checked, model
  download size-bounded and SHA-256-attested. PASS.

### Supply Chain & Automation

- Delta: `libc` is already a `soos-camera-v4l` dependency; no `Cargo.*`, workflow or hook change.
  Review target hygiene: no gitlinks; working-tree and `HEAD^{tree}` fingerprints identical. PASS.
- Unchanged: `install_rustup.sh` verifies the digest before chmod/exec, HTTPS/TLS 1.2+, refuses
  floating channels; Docker contexts are the repository root; systemd sandbox keeps
  `/dev/video*`, `/var/lib/soos`, `/run/soos` access. PASS.

### English-Only Policy

- Delta (code, comments, matrix row CDX11, walkthrough 142 §7a, commit message) is English. PASS.

## 4. Detailed Findings & Action Items

- No CRITICAL, MAJOR or MINOR findings remain.
- **[SUGGESTION]** `packaging/soos-daemon.service:12-13,27` — with `TimeoutStartSec=60`,
  `RestartSec=2` and `StartLimitBurst=5` within `StartLimitIntervalSec=60`, a start that always
  times out never trips the start limit. Deferred by the owner to a follow-up issue; consider
  `StartLimitIntervalSec=` ≥ 5 × (`TimeoutStartSec` + `RestartSec`).

## 5. Final Verdict

**VERDICT: APPROVED**
