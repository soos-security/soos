# Candid Review Report

- **Date**: 2026-10-02
- **Target Branch**: `fix/review-1002-batch`
- **Base (merge-base)**: `095eae9`
- **Reviewed-Diff-Fingerprint**: `e45e484f7f0d1bd6bdff01077fec5dea9e90467df7a5a812ba40f19feac95675`
- **Audited Files** (136): `.github/workflows/ci.yml`, `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/reviews/FULL_PROJECT_REVIEW_2026-10-02.md`, `AI/walkthroughs/165_pam_uid_resolution_fail_closed.md`, `AI/walkthroughs/166_packaging_ownership_arch_pam_ci.md`, `AI/walkthroughs/167_storage_key_race_evidence_and_cli.md`, `AI/walkthroughs/168_gui_camera_vision_review_fixes.md`, `AI/walkthroughs/169_daemon_review_minor_fixes.md`, `Cargo.lock`, `Dockerfile`, `Docs/BIOMETRIC_STORE_CRATE.md`, `Docs/CAMERA_V4L_CRATE.md`, `Docs/CI_CD_AND_SECURITY.md`, `Docs/DAEMON.md`, `Docs/DISTRIBUTION_DEPLOYMENT.md`, `Docs/ENROLLMENT_CLI.md`, `Docs/EVIDENCE_STORE_CRATE.md`, `Docs/GUI_APPLICATION.md`, `Docs/INFERENCE_ORT_CRATE.md`, `Docs/IPC_PROTOCOL.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `Docs/PAM_MODULE.md`, `Docs/VISION_CRATE.md`, `crates/admin-cli/src/args.rs`, `crates/admin-cli/src/gdm.rs`, `crates/admin-cli/src/logs.rs`, `crates/admin-cli/src/main.rs`, `crates/admin-cli/src/test_pam.rs`, `crates/admin-cli/tests/gdm_restore_stale_backup_tests.rs`, `crates/admin-cli/tests/logs_bounded_file_tests.rs`, `crates/admin-cli/tests/test_pam_exit_deadline_tests.rs`, `crates/biometric-store/src/crypto.rs`, `crates/biometric-store/src/store.rs`, `crates/biometric-store/tests/key_publication_race_tests.rs`, `crates/biometric-store/tests/template_fifo_tests.rs`, `crates/camera-v4l/src/config.rs`, `crates/camera-v4l/src/daemon_config.rs`, `crates/camera-v4l/src/diagnostics.rs`, `crates/camera-v4l/src/error.rs`, `crates/camera-v4l/src/lib.rs`, `crates/camera-v4l/src/sensor.rs`, `crates/camera-v4l/src/status.rs`, `crates/camera-v4l/src/v4l_guard.rs`, `crates/camera-v4l/src/v4l_impl.rs`, `crates/camera-v4l/src/v4l_impl/supervisor_tests.rs`, `crates/camera-v4l/tests/enumeration_tests.rs`, `crates/camera-v4l/tests/gcv_review_tests.rs`, `crates/daemon/src/config.rs`, `crates/daemon/src/dispatcher.rs`, `crates/daemon/src/inference.rs`, `crates/daemon/src/main.rs`, `crates/daemon/src/pipeline.rs`, `crates/daemon/src/preview_image.rs`, `crates/daemon/src/sd_notify.rs`, `crates/daemon/src/session.rs`, `crates/daemon/src/socket.rs`, `crates/daemon/tests/daemon_review_minor_tests.rs`, `crates/daemon/tests/evidence_key_opt_in_tests.rs`, `crates/daemon/tests/inference_budget_tests.rs`, `crates/daemon/tests/password_failed_evidence_offload_tests.rs`, `crates/daemon/tests/peer_limits_tests.rs`, `crates/daemon/tests/pipeline_integration_tests.rs`, `crates/daemon/tests/preview_ir_sensor_tests.rs`, `crates/enrollment-cli/src/guided_enrollment.rs`, `crates/enrollment-cli/src/service.rs`, `crates/enrollment-cli/tests/enroll_concurrent_template_tests.rs`, `crates/enrollment-cli/tests/guided_enrollment_zeroize_tests.rs`, `crates/evidence-store/src/crypto.rs`, `crates/evidence-store/src/error.rs`, `crates/evidence-store/src/lib.rs`, `crates/evidence-store/src/store.rs`, `crates/evidence-store/tests/bounded_lock_tests.rs`, `crates/evidence-store/tests/key_publication_race_tests.rs`, `crates/gui/src/app.rs`, `crates/gui/src/args.rs`, `crates/gui/src/camera_source.rs`, `crates/gui/src/daemon_control.rs`, `crates/gui/src/ipc_camera.rs`, `crates/gui/src/privileged.rs`, `crates/gui/src/state.rs`, `crates/gui/src/worker.rs`, `crates/gui/tests/gcv_review_tests.rs`, `crates/gui/tests/import_privacy_tests.rs`, `crates/inference-ort/src/detector.rs`, `crates/inference-ort/src/embedding.rs`, `crates/inference-ort/src/manifest.rs`, `crates/inference-ort/src/pad.rs`, `crates/inference-ort/tests/gcv_review_tests.rs`, `crates/pam/Cargo.toml`, `crates/pam/src/config.rs`, `crates/pam/src/ipc.rs`, `crates/pam/src/lib.rs`, `crates/pam/src/syslog.rs`, `crates/pam/tests/common/mod.rs`, `crates/pam/tests/deadline_uid_tests.rs`, `crates/pam/tests/ipc_hardening_tests.rs`, `crates/pam/tests/pam_bindings_tests.rs`, `crates/pam/tests/pam_fail_quiet_tests.rs`, `crates/pam/tests/pam_feedback_tests.rs`, `crates/pam/tests/pam_handle_tests.rs`, `crates/pam/tests/pam_silent_tests.rs`, `crates/pam/tests/response_expiry_tests.rs`, `crates/pam/tests/uid_resolution_fail_closed_tests.rs`, `crates/protocol/src/codec.rs`, `crates/protocol/src/message.rs`, `crates/protocol/src/types.rs`, `crates/protocol/tests/tagged_encoder_tests.rs`, `crates/vision/src/color.rs`, `crates/vision/src/ir_liveness.rs`, `crates/vision/src/pipeline.rs`, `crates/vision/tests/gcv_review_tests.rs`, `models/manifest.toml`, `packaging/debian/rules`, `packaging/pam/arch/system-auth`, `packaging/pam/arch/system-auth.snippet`, `packaging/pam/debian/soos`, `packaging/pam/debian/soos-notify`, `run_tests.sh`, `scripts/build_arch.sh`, `scripts/build_deb.sh`, `scripts/install.sh`, `tests/distro/arch_linux_test.sh`, `tests/docker/Dockerfile.arch`, `tests/docker/Dockerfile.fedora`, `tests/docker/Dockerfile.ubuntu`, `tests/docker/authselect_profile_test.sh`, `tests/docker/pam_rollback_test.sh`, `tests/docker/test_packages.sh`, `tests/docker/test_suite.sh`, `tests/invariants/src/daemon_docs_contract.rs`, `tests/invariants/src/gcv_review_contract.rs`, `tests/invariants/src/lib.rs`, `tests/invariants/src/packaging_ownership_contract.rs`, `tests/invariants/src/pam_uid_resolution_contract.rs`

## 1. Executive Summary

This is the second review of the 2026-10-02 review batch (GitHub #300-#316). The first review was
APPROVED against fingerprint `bee370a9...ab100a` and raised 3 MINOR findings and 4 suggestions.
Commit `49fcaa5` addresses all 3 MINOR findings and suggestions S3 and S4; S1 and S2 are deferred to
a follow-up issue.

I froze the target again with `./scripts/candid_subagent.sh --prepare` on a clean working tree.
It gives `e45e484f...c95675`. I recomputed the same value from `HEAD^{tree}` with the pinned
`git diff` options and exclusions of the `--rev` gate, so the pre-push hook and CI will compute
the same fingerprint.

`git diff 1d4cd5e 49fcaa5` touches exactly 9 files (+69/-14), all listed in section 3.0. Nothing
else changed between the two reviewed trees.

The full-batch audit of the first review still applies to every file that `49fcaa5` did not touch,
and I summarise it in section 3. The new delta adds no fail-open path, weakens no test, and logs no
secret.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

Removed or changed assertions in the whole frozen patch are unchanged from the first review. Each
one is owner-approved:

- **(a)** `deadline_uid_tests::test_uid_argument_requires_matching_resolved_uid` (formerly
  `..._bypasses_the_resolver`).
- **(a)** `pam_feedback_tests::test_feedback_uid_argument_mismatching_pam_user_returns_ignore`, plus
  the new `..._matching_pam_user_sends_request`.
- **(a)** `pam_feedback_tests::test_feedback_unknown_user_returns_ignore_without_daemon_contact`
  (formerly `..._falls_back_to_process_uid`).
- **(a)** Matrix rows PDR4 and PHS2 reworded.
- **(c)** `tests/invariants/src/lib.rs` `test_pam_config_ordering_matches_spec`: the priority is now
  parsed as a number (stricter than the old substring check); the `deb_notify` `contains` line was
  only moved.
- **(d)** `crates/gui/tests/import_privacy_tests.rs`: two expectations `soos-enroll` ->
  `/usr/bin/soos-enroll`.

Patch lines 1447 and 9611 are documentation text, not tests.

Setup-only migrations keep their assertions word for word:
- **(b)** resolver closures return `Some(..)`; contract tests run as user `root` with `uid=0`; the
  null-handle path uses `uid=<process uid>`;
- `FakeNode` / `FakeProbe` gain the `driver` and `device_caps` fields;
- evidence drain calls are added in `pipeline_integration_tests` and `peer_limits_tests`;
- new `mod` lines.

No `#[ignore]`, `cfg(any())`, `should_panic` or tolerance escape hatch was added. One new inline
`mod tests` was added in `crates/admin-cli/src/test_pam.rs`.

The delta in `49fcaa5` only adds tests. It adds
`camera-v4l/tests/gcv_review_tests.rs::test_gcv_akvcam_and_vimc_capture_nodes_are_rejected`
(`akvcam`, `"AKVCam "` for case and trim, `vimc`, plus `uvcvideo` as the negative control).
No assertion was removed or changed.

## 3. Deep Reasoning Audit

### 3.0 Delta `49fcaa5` (reviewed line by line)

| File | Change | Verdict |
|---|---|---|
| `crates/camera-v4l/src/sensor.rs:322` | `VIRTUAL_CAPTURE_DRIVERS` = `v4l2 loopback`, `v4l2loopback`, `akvcam`, `vivid`, `vimc` | PASS. The existing trimmed, case-insensitive comparison applies. `RDS_OUTPUT` stays out of the output mask; it is radio-only, so this is accepted. |
| `crates/camera-v4l/tests/gcv_review_tests.rs` | new GCV7 test | PASS. It fails against the old list; the negative control prevents an implementation that rejects everything. |
| `crates/{biometric,evidence}-store/src/crypto.rs` `sync_parent_dir` | after a successful `link(2)`, the key directory is fsynced and its error propagated | PASS (see the analysis after this table). |
| `scripts/build_arch.sh` | `--tar` / `--compress` without a value -> message + `usage` + exit 1 | PASS. `usage()` is defined before the parser. |
| `AI/VERIFICATION_MATRIX.md` GCV21 | ✅ Verified, citing both `import_privacy_tests` | PASS. Both cited tests exist; the citation invariant is part of the owner's invariants run. |
| `Docs/GUI_APPLICATION.md`, `Docs/CAMERA_V4L_CRATE.md`, walkthrough 168 | docs synced (absolute `pkexec /usr/bin/soos-enroll`, denylist table, first §9 bullet marked resolved, new "Candid review follow-ups" section) | PASS. English only. |

How `sync_parent_dir` behaves:
- The directory is only fsynced when `link(2)` succeeded. An EEXIST from `link(2)` is
  short-circuited by `and_then`, so the loser of a first-boot race still reads the winner's key.
- If the directory fsync fails after a successful link, the caller gets an `Io` error while the
  key is already on disk. This fails closed: the next start reads that same key, and it is never
  confused with EEXIST. The temporary file is still removed beforehand.
- No `unwrap` is involved, and `File::open` on the parent directory is read-only.

### Logic & Architecture
- **PAM UID selection.** `target_uid` runs before both the event branch and the auth branch. An
  unresolved user, a NUL in the name, an NSS error or a `uid=` that does not match PAM_USER returns
  PAM_IGNORE, with no connection and no event. `uid=0` is accepted only for root. `getuid()` is
  used only on the null-handle path. PASS.
- **Arch stack.**
  - `systemd_home success=3` and `unix success=2` both land on `pam_permit`.
  - A failed face plus a failed password reaches `authfail [default=die]`, so it always fails.
  - The event line cannot be reached on any success path.
  - A locked account still fails: `done` does not override the earlier `required` preauth
    failure.

  PASS.
- **Package ownership.** All three package types are root-owned: Arch through the GNU tar or
  bsdtar owner flags, Debian through `--root-owner-group`, RPM by default. The harness builds them
  as an unprivileged user and checks both owner names and numeric IDs. PASS.
- **Packaging staging.** `/run` is not staged; `--unitdir` is passed; Debian profiles are
  `Default: no` and postinst enables them explicitly. PASS.
- **Config loader.** A missing file still gives the defaults, so default installs are unaffected.
  A directory, FIFO, device, oversized or non-UTF-8 file is a startup error.
  `enforce_active_session=false` is refused without the mock camera. PASS.
- **Socket directory group.** The check runs on the locked descriptor and agrees with the unit's
  `RuntimeDirectory=soos` plus `Group=soos`. PASS.
- **Inference estimate decay.** It lowers the estimate toward the default and never below it; abuse
  can only make a response late, which fails closed. PASS.
- **IR preview.** The YUYV, NV12, RGB24 and MJPEG luma computations are correct, cannot overflow,
  and check the buffer length. PASS.
- **Virtual cameras.** The rule runs in enumeration, diagnostics and the supervisor, for an explicit
  path too. It reads v4l's `capabilities` field, which is `device_caps`. PASS.

### PAM Concurrency & Deadlines
- The budget starts before the PAM_SERVICE read, the flag checks and NSS.
- `GRND_NONBLOCK` is used; `EAGAIN` -> PAM_IGNORE, and EINTR / short reads are retried at most 8
  times.
- No threads or Tokio in `crates/pam`. `test-pam` uses one cumulative deadline.

PASS (no PAM code in the delta).

### Panic Safety & Fail-Closed
- `PAM_SUCCESS` comes only from `Ok((verdict, _)) if !verdict.should_ignore()`.
- `take_panic_location` cannot panic.
- `v4l` opens and drops go through the guard.

The delta adds no panic path. PASS.

### Test Integrity & Anti-Weakening
- Section 2: only the owner-approved changes (a)-(d), plus setup-only migrations.
- The new GCV7 test was red before the list change and has a negative control.

PASS.

### Memory, Bounds & Secrets
- The tagged encoder makes one allocation and zeroizes on error.
- Key publication: `O_EXCL` 0600 file, fsync, `link(2)`, then (new) a directory fsync.
- Evidence writes run on the blocking pool and are drained at shutdown; the lock wait is bounded.
- Template reads use `O_NONBLOCK` and require a regular file.
- Buffers holding biometric data are `Zeroizing`; `Debug` output prints no pixels or embeddings.

PASS.

### Supply Chain & Automation
- Base images are pinned by digest; CI only adds a named volume over `target/`.
- `getrandom` was removed from `soos-pam`; no dependency was added.
- The delta changes only shell argument validation.

PASS.

### English-Only Policy
- The delta and the commit message are in English, and the non-ASCII scan of the batch is clean.

PASS.

## 4. Detailed Findings & Action Items

The 3 MINOR findings of the first review are resolved by `49fcaa5`:
- GCV21 is now ✅ Verified.
- The GUI docs and walkthrough 168 are synced.
- `akvcam` and `vimc` are rejected.

Suggestions S3 (directory fsync) and S4 (`build_arch.sh` value checks) are also implemented.

Still open, non-blocking, deferred to a follow-up issue by the coordinator:
- **[SUGGESTION]** `packaging/pam/arch/system-auth:6` (S1): `success=done` skips
  `pam_faillock.so authsucc` and `pam_env.so` after a face match. This is documented in
  `Docs/DISTRIBUTION_DEPLOYMENT.md` 5.2. A jump to `pam_permit` needs an ADR, because ARCHITECTURE
  section 5 prescribes `done`.
- **[SUGGESTION]** `crates/daemon/src/config.rs:357` (S2): `[pipeline] allow_virtual_camera` is not
  read by the `soos-daemon` loader. This is documented and fails closed.

New findings on the delta: none.

## 5. Final Verdict

No CRITICAL, MAJOR or MINOR finding remains.

**VERDICT: APPROVED**
