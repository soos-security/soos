# Candid Review Report

- **Date**: 2026-09-30
- **Target Branch**: `fix/p1-pad-camera-batch`
- **Base (merge-base)**: `2e15447`
- **Reviewed-Diff-Fingerprint**: `0878fd4aa8d905aeadbc7c6e7df2c17454bf416869dbe15589578ab586758a74`
- **Review round**: 2. The round-1 report (fingerprint `cfc89676…`) returned CHANGES_REQUESTED.
- **Issues claimed**: #150, #151, #152, #153, #154, #155, #169, #170, #171, #172
- **Audited Files** (79): `AI/ARCHITECTURE.md`, `AI/BACKLOG.md`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`,
  `AI/walkthroughs/{53,58,63,66,85,86,87,88,89,90}_*.md`,
  `Docs/{CAMERA_V4L_CRATE,ENROLLMENT_CLI,GUI_APPLICATION,INFERENCE_ORT_CRATE,IPC_PROTOCOL,POLICY_CRATE,README,VISION_CRATE}.md`,
  `crates/camera-v4l/src/{config,error,frame,lib,manager,mock,resolver,stable_path,status,v4l_impl}.rs`,
  `crates/camera-v4l/tests/{camera_status,error_recovery,ir_capture_plan,resolver,supervision}_tests.rs`,
  `crates/daemon/src/{config,dispatcher,health,main,pipeline}.rs`,
  `crates/daemon/tests/{camera_health,camera_resolution,pipeline_integration,threshold_config}_tests.rs`,
  `crates/enrollment-cli/src/{lib,service}.rs`, `crates/enrollment-cli/tests/camera_resolver_parity_tests.rs`,
  `crates/gui/src/{app,camera_mode,camera_source,camera_status,daemon_control,ipc_camera,lib,logging,main,privileged,worker}.rs`,
  `crates/gui/tests/{bounded_output,camera_mode,camera_source,camera_status_retry,camera_status,responsiveness}_tests.rs`,
  `crates/inference-ort/tests/pad_real_model_tests.rs`, `crates/policy/src/threshold.rs`,
  `crates/policy/tests/threshold_floor_tests.rs`, `crates/vision/src/{error,ir_liveness,lib,pipeline}.rs`,
  `crates/vision/tests/{ir_pad_policy,ir_sensor_policy,pad}_tests.rs`, `tests/invariants/src/{lib,pad_contract}.rs`,
  `tests/physical/adversarial_test.sh`

## 1. Executive Summary

This round reviews the full diff against `origin/main`. It pays particular attention to the
six rework commits (`05e4082`, `fcef3a5`, `784551c`, `61df178`, `093d7e9`, `05dac29`), which
answer the round-1 findings.

Status of the round-1 findings:

| # | Round-1 finding | Status | Evidence in code |
|---|---|---|---|
| 1 | MAJOR: #154 runtime source switching missing; GUI Resume re-creates #150 EBUSY | **Resolved** | `crates/gui/src/camera_source.rs` (planner, `SwitchableCamera`, supervisor thread, `HandoverExecutor`); `app.rs` wraps `PkexecExecutor` in `HandoverExecutor`; `main.rs` no longer selects the source once |
| 2 | MAJOR: IR policy keyed on `Grey` only | **Resolved** | `Frame::sensor_type`; `plan_capture` classifies the opened node, prefers `Grey` on `Infrared` under auto negotiation, and stamps every frame (`v4l_impl.rs`); `PadInputModality::for_frame` is used by both `process_frame` and `analyze_frame` |
| 3 | MINOR: daemon parity tests hit an unused function | **Resolved** | `initialize_pipeline` calls `resolve_pipeline_camera(config, &SystemCameraEnumerator::default())`, which is built on `plan_camera_device`; invariant `test_daemon_production_camera_resolution_uses_tested_resolver` |
| 4 | MINOR: two by-id scanners | **Resolved** | `stable_device_path` delegates to `SystemCameraEnumerator::by_id_aliases`; the local 256 cap is deleted; invariant `test_single_bounded_by_id_scanner` |
| 5 | MINOR: banner always says "retrying" | **Resolved** | `error_is_retried` in `camera_status.rs`; `CameraBlockReason::is_transient` plus a bounded re-probe in the planner; per-reason messages |
| 6 | MINOR: no IR corpus measurement | **Documented follow-up** | walkthrough 86 §8; walkthrough 90 "Follow-ups" |
| 7 | MINOR: `Command::output()` unbounded | **Resolved** | `read_bounded` (`Read::take(limit + 1)`); the helper is killed on an oversized or failed read; the pipe is closed before `wait()`, so no deadlock |
| 8 | MINOR: GEPU3 cites removed helpers | **Resolved** | the GEPU3 row is marked superseded by GRE1/GRE2/GRE7/GRE8; new rows GRE7–GRE10, PIR6, CSR6 |
| 9 | SUGGESTION: two camera state machines | **Documented follow-up** | walkthrough 89 §7 |
| 10 | SUGGESTION: unplug during standby | **Documented follow-up** | walkthrough 89 §7 |

The rework introduces no CRITICAL or MAJOR defect. I found four new MINOR robustness issues
(§4). None of them opens the camera while the daemon owns it, and none weakens a PAD decision
the daemon makes.

Local verification on this tree:
- `cargo fmt --all -- --check` is clean.
- `cargo clippy --locked --workspace --all-targets -- -D warnings` is clean.
- `cargo test --locked` passes with 0 failures across `soos-gui`, `soos-vision`,
  `soos-camera-v4l`, `soos-daemon`, `soos-invariants`, `soos-enrollment-cli` and `soos-policy`.

## 2. Test Changes (mechanical listing from step 3)

- Removed or changed assertion lines (`^-.*assert|#[test]|...`) in the frozen patch: **none**.
  None in the rework range `16e461e..HEAD` either.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`): **none**. The
  `tolerance` hits are the golden-logit tolerance `1e-3` in the new `pad_real_model_tests.rs`
  and the docs that describe it. `test_real_pad_golden_logits_detect_channel_order_swap`
  asserts that this tolerance is tight enough, which justifies it under #172.
- Inline `mod tests` changes: none.
- Pre-existing test files modified (unchanged since round 1, all additive or justified):
  - `crates/vision/tests/pad_tests.rs`: one rename. The body is unchanged, and the new name
    states the test is not a metric (#172).
  - `crates/daemon/tests/pipeline_integration_tests.rs`: gains a `new_with_format` fixture
    and #169 tests. The default path is unchanged.
  - `crates/camera-v4l/tests/error_recovery_tests.rs`: gains #150 tests.
  - `tests/physical/adversarial_test.sh`: stops fabricating metrics in mock mode. Guarded
    by an invariant.
- New test files in the rework:
  - `ir_capture_plan_tests.rs`
  - `ir_sensor_policy_tests.rs`
  - `camera_source_tests.rs` (19 tests)
  - `camera_status_retry_tests.rs`
  - `bounded_output_tests.rs`
  - two invariants in `tests/invariants/src/lib.rs`
- `784551c` edits only the new, not-yet-merged `ir_sensor_policy_tests.rs` fixture (a YUYV
  encoder refactor). It changes no assertion.
- Verdict on test integrity: **no weakening found**.

## 3. Deep Reasoning Audit

### Logic & Architecture

- **IR sensor propagation (#169).**
  - Scenario: IR node `"USB2.0 FHD UVC WebCam: USB2.0 I"` offers `[Yuyv, Grey]`.
    `plan_capture` classifies it `Infrared` and negotiates `Grey`.
  - Scenario: the same node offers only YUYV/MJPEG. It streams YUYV but stays tagged
    `Infrared`, so the frame is `Monochrome`, the IR gate runs, and the 0.95 threshold
    applies.
  - Scenario: `auto_format = false` with YUYV on the IR node. The format is honored and the
    tag is kept.
  - Every production `Frame::new` site was checked:
    - The V4L2 capture stamps the tag.
    - The mock keeps `Unknown`, so the format rule applies.
    - The GUI IPC reconstruction keeps `Unknown`. See MINOR 3.
  - The daemon (`dispatcher.rs`) and `soos-enroll` consume the V4L2 frames unchanged.
  - → PASS.
- **Runtime source switching (#154/#150).** Scenarios tried:
  1. Start with the daemon paused (Direct), then press Resume.
     `HandoverExecutor::execute` → `release_for_daemon`, which sets `Pending`, takes the
     planner lock, then replaces and releases the direct manager. `release_manager` stops the
     manager and drops the last reference, and the V4L2 `Drop` joins the capture thread
     (device closed). pkexec runs only after that. The tests assert that the direct manager
     is already dropped when Resume executes.
  2. Resume is cancelled in Polkit. `finish_handover(false)` sets `Idle`, the flag change
     triggers a re-probe, and the GUI returns to Direct.
  3. Resume succeeds. `Started{10 s}` keeps Direct disabled until the monitor reports
     `Active`, then the GUI switches to IPC.
  4. Pause while in IPC. The monitor reports `Inactive`, the planner probes, gets
     `NotRunning`, and switches to Direct.
  5. Unknown daemon state. The planner returns `None` and never touches the device.
  6. The daemon is started externally while the GUI is Direct. `DirectV4l` plus
     `daemon_active` is probed on every tick, and the decision can never be Direct.
  7. A supervisor tick is racing with Resume. Both paths serialize on the planner mutex.
     `release_for_daemon` checks `camera.mode()` after it takes the lock, so a manager
     opened by an in-flight tick is still released.
  - → PASS.
- **Resolver single path (#152).** `resolve_pipeline_camera` on the mock path returns
  `config.camera` unchanged, as before. On the explicit path, `plan_camera_device` keeps it
  verbatim. On the auto path it delegates to the shared resolver. → PASS.
- **Bounded retry.** Found a case where `DirectOpenFailed` escapes the bound → MINOR 1.

### PAM Concurrency & Deadlines

- The diff does not touch `crates/pam`. No threads or Tokio reach the PAM module.
- The daemon-side EBUSY recovery is the #150 supervisor backoff, which checks `running`
  every 20 ms.
- → PASS.

### GUI Threads, Locks & UI-Thread Blocking

- **Lock order.**
  - `tick`: planner → handover (inside `handover_active`).
  - `release_for_daemon`: handover (released) → planner.
  - `finish_handover`: handover only.
  - No cycle, so no deadlock.
- **`SwitchableCamera`.** The `RwLock` is held only to clone an `Arc` or for `mem::replace`.
  `release_manager` runs outside the lock. No thread waits on a manager while holding the
  lock.
- **UI thread.** It calls only `status()`, `notice()`, `generation()` and `mode()` (all
  non-blocking) and spawns the supervisor.
  - Probes, joins and `pkexec` run on the supervisor and privileged threads.
  - Blocking there is bounded: IPC socket timeouts, a 3 s release deadline, and capture
    backoff that checks `running` every 20 ms.
  - The only UI-thread join is the supervisor `Drop` at application exit. That is
    acceptable.
- → PASS (MINOR 2 concerns a panic path).

### Panic Safety & Fail-Closed

- No `unwrap/expect/panic!/todo!/unreachable!` was added in production `src/`. Poisoned
  locks are recovered with `into_inner`.
- The IR policy is now fail-closed on the sensor tag as well as the format:
  - NaN or low scores are rejected.
  - A gate rejection never consults the model (spy test).
  - `analyze_frame` reports not-live below the IR threshold.
- There is still no path from an error to `Allow`.
- The GUI planner errs toward *not* opening the device: Unknown state, a pending handover,
  and a failed monitor spawn all leave the camera disabled.
- → PASS.

### Test Integrity & Anti-Weakening

- See §2.
- The new tests can fail against plausible wrong implementations:
  - The format-only modality fails `test_infrared_sensor_is_monochrome_modality_for_every_pixel_format`.
  - A missing release fails `test_handover_releases_direct_camera_before_resume_executes`.
  - A single-shot source choice fails `test_supervisor_switches_sources_on_daemon_state_changes`.
- → PASS.

### Memory, Bounds & Secrets

- `read_bounded` caps the buffer at `limit + 1` while reading. The helper stdout is dropped
  inside `read_bounded`, so `wait()` cannot block on a full pipe, even if `kill` of the
  setuid child fails.
- `Frame::sensor_type` holds no pixel data, and `Zeroize` still covers `data`.
- Logs carry modes, reasons and paths only. No frames or embeddings are logged.
- → PASS.

### Supply Chain & Automation

- No changes to `Cargo.*`, `deny.toml`, `.github/` or the hooks. → PASS.

### English-Only Policy

- All added code, comments, docs, walkthroughs and commit subjects are English. → PASS.

## 4. Detailed Findings & Action Items

No CRITICAL or MAJOR findings.

1. **[MINOR]** `crates/gui/src/camera_source.rs:118-123` with `:454-460`. The
   `DirectOpenFailed` retry is not bounded, contrary to GRE9 and to the planner documentation.
   - The retry cycle:
     1. `step` increments `transient_retries` on the blocked state.
     2. `step` calls `force(DirectV4l)`, which does not reset the counter because the
        previous state was `Blocked`.
     3. `apply` calls `open_direct`, which fails.
     4. `planner.force(Blocked(DirectOpenFailed))` then *resets* the counter to 0, because
        the current state is now `DirectV4l`.
   - Concrete failure: `V4lCameraManager::spawn` fails (thread spawn `EAGAIN` under a
     process or thread limit). The supervisor then retries every second forever instead of
     stopping after `MAX_TRANSIENT_PROBE_RETRIES`.
   - Impact is low: the loop is rate-limited, and `spawn` fails only on thread creation.
   - Required: keep the counter when `apply` forces a block right after a step-initiated
     switch. Add a supervisor-level test with a backend whose `open_direct` always fails.
2. **[MINOR]** `crates/gui/src/camera_source.rs:649-653`. `HandoverExecutor::execute`
   calls `finish_handover` only when `inner.execute` returns.
   - Concrete failure: `inner.execute` panics. The privileged thread dies, `TaskRunner::poll`
     logs "terminated without an outcome", and `Handover` stays `Pending` forever. The GUI
     then never reopens the direct camera until it restarts.
   - This fails closed for the device but is a permanent availability loss.
   - Required: use a drop guard, or `catch_unwind`, that calls `finish_handover(false)`.
3. **[MINOR]** `crates/gui/src/ipc_camera.rs:324-331` (documented in walkthrough 86 §8).
   The preview wire carries the format but not the sensor type. In IPC mode, the GUI rebuilds
   frames as `SensorType::Unknown`.
   - Concrete failure: a daemon configured with `auto_format = false` and `format = yuyv` on
     an IR node sends YUYV previews. The GUI's `analyze_frame` then scores them on the colour
     path at 0.85. The guided-enrollment liveness gate (`worker.rs:80-85`) can accept an IR
     capture as live that the daemon would reject at 0.95.
   - The walkthrough says "Production decisions are unaffected". That is too strong: GUI
     enrollment into the system store is a production write.
   - Verification is still fail-closed, because the daemon uses its own tagged frames.
   - Required: track the protocol bump as a follow-up, and reword the walkthrough to name the
     enrollment-gate exposure.
4. **[MINOR]** `crates/gui/src/daemon_control.rs:31-40` with
   `crates/gui/src/camera_source.rs:143-157`. The monitor maps every non-`active` unit state
   to `Inactive`, and the planner now opens `/dev/video*` on `Inactive`. The unit reports
   non-`active` states such as `activating`, `activating (auto-restart)` and `deactivating`.
   - Concrete failure: `soos-daemon` crashes with `Restart=on-failure`. The GUI polls during
     the auto-restart window and grabs the camera directly. The restarted daemon meets
     `EBUSY` until the next poll shows `active`, about 2 s later.
   - The system self-heals through the #150 backoff, but face unlock is unavailable in that
     window.
   - Required (follow-up): treat `activating` and `deactivating` as "daemon owns the device",
     for example by parsing `systemctl is-active` output instead of using only its exit code.
5. **[SUGGESTION]** The handover has only mock-level coverage. The walkthrough 90
   follow-up correctly asks for a hardware check on a dual-sensor laptop: start the GUI
   paused, press Resume, and confirm the daemon logs no `EBUSY`. Run it before closing #150
   and #154 on real hardware.

Round-1 follow-ups still open (documented, non-blocking): finding 6 (IR corpus through
`VisionPipeline`), suggestions 9 and 10.

## 5. Final Verdict

Every blocking round-1 finding (1, 2) is fixed in production code with tests that fail
against the previous behavior. Findings 3, 4, 5, 7 and 8 are resolved in code or in the
matrix, and 6, 9 and 10 are recorded as follow-ups. No test was weakened. The new findings
are MINOR robustness issues that neither open a fail-open path nor re-create the daemon/GUI
device fight in the reviewed Resume and Pause flows.

**VERDICT: APPROVED**
