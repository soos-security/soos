# Candid Review Report

- **Date**: 2026-10-01
- **Target Branch**: `fix/hw-remainder-batch`
- **Base (merge-base)**: `043d250`
- **Reviewed-Diff-Fingerprint**: `3f4c4ff44f6c6509827583b8dd6c199e2d5d61ad4c36619587ad954ae9757890`
- **Audited Files**: `.agents/skills/dev-workflow/references/project-facts.md`, `.github/workflows/ci.yml`, `.gitignore`, `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/158_systemd_unit_acceptance.md`, `AI/walkthroughs/159_camera_capture_backend_seam.md`, `AI/walkthroughs/160_embedding_real_face_evaluation.md`, `AI/walkthroughs/161_pad_second_model_attestation.md`, `Cargo.lock`, `Docs/CAMERA_V4L_CRATE.md`, `Docs/CI_CD_AND_SECURITY.md`, `Docs/INFERENCE_ORT_CRATE.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `Docs/VISION_CRATE.md`, `crates/camera-v4l/src/diagnostics.rs`, `crates/camera-v4l/src/sensor.rs`, `crates/camera-v4l/src/v4l_guard.rs`, `crates/camera-v4l/src/v4l_impl.rs`, `crates/camera-v4l/src/v4l_impl/supervisor_tests.rs`, `crates/daemon/tests/pad_live_camera_check_tests.rs`, `crates/daemon/tests/pad_nonface_pipeline_real_model_tests.rs`, `crates/daemon/tests/pad_v1se_real_model_tests.rs`, `crates/inference-ort/src/pad.rs`, `crates/inference-ort/tests/pad_input_range_tests.rs`, `crates/inference-ort/tests/pad_real_model_tests.rs`, `crates/inference-ort/tests/pad_resample_tests.rs`, `crates/inference-ort/tests/pad_tests.rs`, `crates/vision/Cargo.toml`, `crates/vision/tests/embedding_lfw_evaluation_tests.rs`, `models/README.md`, `models/manifest.toml`, `models/optional_models.toml`, `scripts/compare_pad_models.py`, `scripts/convert_pad_models.py`, `scripts/download_models.sh`, `scripts/fetch_lfw_eval.sh`, `tests/docker/Dockerfile.systemd`, `tests/docker/systemd_unit_acceptance_test.sh`, `tests/invariants/src/capture_backend_contract.rs`, `tests/invariants/src/embedding_evaluation_contract.rs`, `tests/invariants/src/lib.rs`, `tests/invariants/src/pad_second_model_attestation_contract.rs`, `tests/invariants/src/systemd_unit_acceptance_contract.rs` (45 files)

Fingerprint check: `--prepare` on the clean working tree (HEAD `966cb06`) printed the
fingerprint above. The `--rev` recipe on `HEAD^{tree}` (same pinned `git diff` options and
excludes) gives the same SHA-256.

This is a third-round review. Round 1 (`0e4b953e…`, CHANGES_REQUESTED) covered the whole batch up
to `e1d4dce`. Round 2 (`55e5de47…`, APPROVED with two MINOR findings) covered
`e1d4dce..ed25e8d`. This round checks the delta `git diff ed25e8d HEAD` (commit `966cb06`): 5
files, +47 / -7. That commit touches only:

- `tests/docker/Dockerfile.systemd`
- `tests/docker/systemd_unit_acceptance_test.sh`
- `tests/invariants/src/systemd_unit_acceptance_contract.rs` (additions only)
- `AI/VERIFICATION_MATRIX.md` (SUA8 row)
- `AI/walkthroughs/158_systemd_unit_acceptance.md`

Nothing else changed. The analysis of the earlier rounds carries over.

## 1. Executive Summary

Both round-2 MINOR findings are fixed. There are no new findings.

1. **Mask existence check.** After the mask loop, the image build now fails if
   `systemd-sysctl.service`, `systemd-modules-load.service` or `systemd-binfmt.service` is
   missing under `/usr/lib/systemd/system`.
   - The check looks at the shipped unit file, not the `/etc` mask symlink, so a renamed unit
     can no longer go unnoticed at build time.
   - Round 1 confirmed that all three units exist in the current `soos-sua-runtime` image.
   - The Dockerfile comment, the SUA8 row and walkthrough 158 now describe the masks correctly:
     a mask succeeds for any name, and the host sysctl comparison is the backstop.
2. **Volatile sysctls.** The host sysctl comparison now also excludes
   `kernel.perf_event_max_sample_rate`, `kernel.tainted` and `fs.aio-nr`. No `sysctl.d` file
   sets any of them, so dropping them does not weaken the isolation check. It only removes the
   risk of a false failure on the required check.

Host evidence observed in this session: `kernel.sysrq = 16` and `fs.protected_regular = 1`.
These are the host's own values, restored after the owner's `sudo sysctl --system`, and they are
unchanged after the post-fix harness run the coordinator reported. Before the fix, the same run
had moved them to 176 and 2.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- Delta: no `-` line contains an assertion. The invariant
  `test_systemd_acceptance_container_never_writes_host_state` gains two checks:
  - the unit existence check and its exact unit list;
  - the three excluded volatile keys.

  These are additions only.
- Batch level: unchanged from round 1. All removed or changed assertions match the owner
  approval of 2026-10-01 (golden logits, five `/ 255.0` range tests, the `checkerboard_160`
  threshold rule with the compensating `pad_nonface_pipeline_real_model_tests`).

## 3. Deep Reasoning Audit

### Logic & Architecture

- **Build check.**
  - `[ -e ... ] || { ...; exit 1; }` runs inside a `RUN` under `/bin/sh`, chained with `&&`
    after the mask loop, so any failure fails the build.
  - The mask loop still has `|| exit 1`.
  - Scenario: a unit is renamed upstream. The build stops with "host-state unit ... not found"
    → PASS.
- **Snapshot filter.** The regex adds alternatives only. The `kernel.`, `vm.` and `fs.`
  tunables that `sysctl.d` writes stay in the comparison (`sysrq`, `kptr_restrict`, `printk`,
  `yama.ptrace_scope`, `mmap_min_addr`, `max_map_count`, `protected_*`). Scenario: the
  `systemd-sysctl` mask is removed. The `sysrq` and `protected_regular` differences would still
  fail the run → PASS.
- **Wording.** The comment says `kernel.tainted` is set by "any module load". Strictly, only
  some loads set it (out-of-tree, unsigned or forced modules) and kernel warnings also set it.
  This is cosmetic and is not raised as a finding.

### PAM Concurrency & Deadlines

The delta does not touch `crates/pam` → PASS (N/A).

### Panic Safety & Fail-Closed

The delta has no Rust production code. The harness still fails closed on a missing unit, an
unmasked unit, an empty snapshot or any remaining sysctl difference → PASS.

### Test Integrity & Anti-Weakening

The new invariant assertions would fail if the existence loop or any of the three exclusions
were removed. No existing assertion changed → PASS.

### Memory, Bounds & Secrets

Nothing in the delta handles secrets or biometric data → PASS.

### Supply Chain & Automation

The CI workflow is unchanged since round 2: SHA-pinned checkout, `contents: read`, gated by
`CI Success`, model URLs pinned to revisions → PASS.

### English-Only Policy

All added comments, the matrix row and the walkthrough text are in English → PASS.

## 4. Detailed Findings & Action Items

There are no CRITICAL, MAJOR or MINOR findings.

- **[SUGGESTION]** (carried over) `tests/docker/systemd_unit_acceptance_test.sh` (`cleanup`
  trap): also run `host_sysctls_unchanged` in report-only mode on the failure path.
- **[SUGGESTION]** (carried over) `.github/workflows/ci.yml` (`systemd-unit` job): cache the
  verified models with a SHA-pinned `actions/cache`, keyed on the manifest hash.
- **[SUGGESTION]** (carried over) `scripts/download_models.sh:293-312`: reject a `filename` that
  appears in both the main and the optional manifests.

## 5. Final Verdict

All findings from rounds 1 and 2 are resolved. The delta is limited to test tooling,
invariants and docs, and it adds checks without weakening any existing one.

**VERDICT: APPROVED**
