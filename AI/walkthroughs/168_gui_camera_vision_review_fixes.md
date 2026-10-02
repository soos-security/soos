# Walkthrough 168 — GUI, Camera and Vision Review Fixes (2026-10-02 review)

- **Date**: 2026-10-02
- **Issue**: GitHub #304, #305, #306, #307 (MAJOR), #313 and #314 (minor groups) from
  `AI/reviews/FULL_PROJECT_REVIEW_2026-10-02.md`; no backlog issue — **Branch**:
  `fix/gui-camera-vision-review`
- **Matrix criteria**: GCV1–GCV21 (component `gui-camera-vision-review-fixes`)

## 1. Context & Objectives

| Item | Finding | Delivered |
|---|---|---|
| #304 VIS-NEW-1 | GUI guided enrollment sampled `analyze_frame`, which picked the best face without a face-count check: a dominant second face could become the template | `VisionAnalysis::face_count`; `analyze_frame` runs PAD / alignment / embedding only for one face; GUI gate and "One face only" feedback |
| #305 CAM-NEW-1 | The preview dropped the IR sensor stamp; the GUI took the colour PAD path on IR frames | daemon sends IR frames as Grey (wire format 1), no protocol bump |
| #306 CAM-NEW-2 | GUI froze on the last frame and stayed Ready when the daemon camera failed | empty preview after the first frame: frame withdrawn, not ready, `SourceUnavailable` |
| #307 CAM-NEW-3 | Resolver accepted v4l2loopback / vivid nodes | `virtual_node_rejection`, enumeration filter, diagnostics status, supervisor refusal with opt-in |
| #313 VIS-NEW-2..8 | zero-size embedding input, non-finite embeddings, manifest layouts, unwiped buffers, `Debug` leaks, SCRFD channel-order docs, stale docs | all delivered (VIS-NEW-7 docs only) |
| #314 CAM-NEW-4..7, S1, S2 | NUL path panic, `--mock` without `--dev-store`, unwiped GUI frame copies, transitional systemd states, relative pkexec programs, mock-camera doc, unchecked wire formats | all delivered except the import helper of CAM-NEW-7 (b), see §9 |

## 2. Architect Design

- `soos_vision::VisionAnalysis::face_count(&self) -> usize` = `detections.len()` (the
  `process_frame` rule, which rejects more than one detection whatever its confidence).
  `analyze_frame` filters the primary face with `detections.len() == 1`. No new struct field, so
  every `VisionAnalysis` literal keeps compiling.
- `soos_gui::worker::GuiEnrollmentFeedback { Step(EnrollmentStepFeedback), OneFaceOnly }` and
  `guided_enrollment_feedback(..)`. `feed_guided_enrollment` keeps its signature and gains the
  gate (`face_count() != 1` → streak broken, `None`). `crates/enrollment-cli/src/guided_enrollment.rs`
  (owned by another batch) is untouched: the GUI-level variant lives in the GUI.
- `soos_daemon::preview::preview_image_for_frame`: an `Infrared` frame in a colour format is
  converted to its luma plane (YUYV / NV12 `Y`, BT.601 luma of RGB24 and decoded MJPEG) and the
  greyscale rules apply (passthrough or decimation). Only `crates/daemon/src/preview_image.rs`
  changed in the daemon.
- `soos_gui::ipc_camera::frame_from_preview(&mut PreviewResponse) -> Result<Option<Frame>, IpcPreviewError>`:
  `Ok(None)` empty, known format with exact length (any non-empty MJPEG), `Protocol` otherwise.
  `WorkerState::had_frame` distinguishes warm-up from a camera failure.
- `soos_camera_v4l`: `VirtualNodeRejection { MemToMem, OutputCapable, VirtualDriver }`,
  `virtual_node_rejection(driver, device_caps)`, `VIRTUAL_CAPTURE_DRIVERS`,
  `V4L2_OUTPUT_CAPABLE_CAPS`, `V4L2_MEM_TO_MEM_CAPS`; `V4lNodeCapabilities { driver, device_caps }`;
  `NodeStatus::Rejected(VirtualNodeRejection)`; `CameraError::VirtualDevice { path, reason }`
  (`UnsupportedDevice` kind, re-resolution consulted); `CameraConfig::allow_virtual_device` +
  builder; `DaemonConfigKey::AllowVirtualCamera`, `DaemonCameraConfig::allow_virtual_camera`.
- `v4l_guard::open_device_guarded(path) -> io::Result<GuardedDevice>` (NUL → `InvalidInput`,
  panic guard, guarded close in `Drop`); `GuardedDevice::get() -> Option<&v4l::Device>` (no
  `expect`). `V4lBackend::Device = GuardedDevice`.
- GUI: `--mock` `requires = "dev_store"`; `LatestFrameData::{rgb, aligned_crop}` are `Zeroizing`
  with a redacting `Debug`; `daemon_control::active_state_means_running(&str)`; `SystemctlProbe`
  reads `ActiveState`; `privileged::{PKEXEC_PROGRAM, SYSTEMCTL_PROGRAM, SOOS_ENROLL_PROGRAM}`.
- Inference: `prepare_input` zero-dimension guard; `BiometricEmbedding::normalize` rejects
  non-finite components and norms; manual `Debug` on `BiometricEmbedding`, `PipelineOutput`,
  `VerificationOutcome`, `VisionAnalysis`; softmax exponentials and the probabilities of
  `evaluate_liveness` zeroizing (`softmax` keeps `Vec<f32>`); IR luma and MJPEG decode buffers
  zeroizing; `models/manifest.toml` declares `input_layout = "NCHW"` for SCRFD and MiniFASNetV2.
- ADR 2026-10-02 "Virtual V4L2 Nodes Are Never Biometric Cameras".

## 3. Plan Evaluation

Condensed (orchestrator-scoped batch): the plan was checked against the issue texts, the
orchestrator decisions and the file-ownership constraints. Two deviations from the review text,
both orchestrator decisions: an explicit `camera_device` on a virtual node fails closed unless
opted in (review: "still works"), and #304 is enforced in vision/GUI instead of
`guided_enrollment.rs`.

## 4. Tester Contract

| Test | Matrix | Red evidence |
|---|---|---|
| `crates/vision/tests/gcv_review_tests.rs` (5) | GCV1, GCV12 | compile error `no method named face_count`; with an API stub: `no PAD on a multi-face frame`, Debug and `process_frame` count assertions failed (3 of 5) |
| `crates/gui/tests/gcv_review_tests.rs` (10) | GCV1–GCV3, GCV15–GCV18 | compile errors on the new APIs; with the gates disabled: `left: Some(SampleAccepted { step: Frontal, collected: 1, target: 4 })` from a two-face frame, `an empty preview after a frame must mark the source not ready` |
| `crates/daemon/tests/preview_ir_sensor_tests.rs` (5) | GCV2 | 4 failed on the old code: `left: 2 right: 1` (IR YUYV sent as YUYV), `left: 3 right: 1` (NV12) |
| `crates/camera-v4l/tests/gcv_review_tests.rs` (6) | GCV4–GCV6, GCV14 | compile errors on the new APIs; with the rule disabled: loopback and vivid enumerated (`["/dev/video0", "/dev/video1", "/dev/video2"]`), diagnostics `Candidate`; without the open guard: `v4l2/api.rs:118 called Result::unwrap() on an Err value: NulError` |
| `v4l_impl::supervisor_tests::test_gcv_*` (3) | GCV6 | new API (`driver` / `device_caps` / `allow_virtual_device`) |
| `crates/inference-ort/tests/gcv_review_tests.rs` (4) | GCV8–GCV10, GCV12 | 4 failed on the old code: `prepare_input must not panic` (underflow at `embedding.rs:333`), `NaN must be rejected`, `scrfd_500m_kps must declare input_layout`, Debug printed values |
| `tests/invariants/src/gcv_review_contract.rs` (4) | GCV10, GCV11, GCV14, GCV19 | source contracts written with the fix |

### Migrated existing tests (setup only, no assertion changed)

| Test file | Change | Reason |
|---|---|---|
| `crates/camera-v4l/tests/enumeration_tests.rs` (`FakeProbe::with`) | `V4lNodeCapabilities` literal gains `driver: "uvcvideo"`, `device_caps: 0` | new fields mandated by #307 |
| `crates/camera-v4l/src/v4l_impl/supervisor_tests.rs` (`FakeNode`) | fake node gains `driver` / `device_caps` (uvc defaults) | same |

### Flakiness check

The IPC socket tests (`test_gcv_empty_preview_*`) use generous 5 s waits and no wall-clock bound.

## 5. Auditor Constraints

1. No `unwrap` / `expect` added to production code (`GuardedDevice` exposes `Option`).
2. No pixel, luma or embedding value in any log or `Debug` output.
3. Fail closed: unknown wire formats are `Protocol`; virtual nodes refused by default; unknown
   systemd states keep the daemon as camera owner; mistyped opt-in is `false`.
4. Every new buffer holding face pixels or scores is `Zeroizing`.
5. Bounded reads only (`systemctl show` output capped at 256 bytes before parsing).
6. No protocol change (`PreviewResponse` unchanged).

## 6. Implementation

Files: `crates/vision/src/{pipeline,color,ir_liveness}.rs`,
`crates/inference-ort/src/{embedding,pad,manifest,detector}.rs`, `models/manifest.toml`,
`crates/daemon/src/preview_image.rs`, `crates/camera-v4l/src/{sensor,diagnostics,v4l_guard,v4l_impl,error,status,config,daemon_config,lib}.rs`,
`crates/gui/src/{ipc_camera,worker,state,app,args,daemon_control,privileged,camera_source}.rs`,
docs (`Docs/CAMERA_V4L_CRATE.md`, `Docs/GUI_APPLICATION.md`, `Docs/IPC_PROTOCOL.md`,
`Docs/INFERENCE_ORT_CRATE.md`, `Docs/VISION_CRATE.md`), `AI/DECISIONS.md`,
`AI/VERIFICATION_MATRIX.md`, invariant `tests/invariants/src/gcv_review_contract.rs`.

Notable decisions: `softmax` keeps its `Vec<f32>` signature (changing it broke the iteration in
the existing `pad_tests::test_softmax_numerical_stability`), the production caller wraps the result
instead. The NUL `camera_device` is listed in `mistyped_keys` (no path can contain NUL), so
`soos-admin camera list` reports the file as `Malformed` like any mistyped camera key.
VIS-NEW-7 is documentation only: no SCRFD preprocessing change without an owner decision.

## 7. Candid Review

Layer 1 (`./scripts/candid_review.sh`): PASSED (no `unsafe` additions, no PAM changes, English
policy). The layer 2 sub-agent review is left to the orchestrator (the fingerprint is bound to the
final diff).

## 8. Verification Results

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | clean |
| `SOOS_MODELS_DIR=<real models> cargo test --no-fail-fast --locked --all-features -p soos-camera-v4l -p soos-vision -p soos-inference-ort -p soos-gui -p soos-daemon -p soos-admin-cli -p soos-protocol -p soos-invariants` | 1649 passed, 0 failed, 6 ignored |
| `cargo test --no-fail-fast --locked --all-features -p soos-daemon -p soos-enrollment-cli` | 571 passed, 0 failed |
| `ORT_SKIP_DOWNLOAD=1 cargo check --locked --target i686-unknown-linux-gnu --all-targets --all-features` (touched crates) | clean |
| `./scripts/candid_review.sh` | PASSED |

Without `SOOS_MODELS_DIR` the two `scrfd_real_model_tests` fail on this host because
`/var/lib/soos/models` lacks `sface_2021dec.onnx` (environment, not this change).

Real hardware (metadata only, no frame stored): `soos-admin camera list` on the development
laptop lists `/dev/video0` (driver `uvcvideo`, `device_caps 0x04200001`) as `candidate` and
selects it; the uvcvideo metadata node stays `not_video_capture`. The rejection rule does not
touch a physical webcam.

## 9. Known Limitations / Follow-ups

- **Resolved (see the integration note below; owner-approved 2026-10-02).** ~~CAM-NEW-7 (b), import path (owner decision required)~~: `privileged::import_helper_args` still
  starts with the relative `soos-enroll`, which `pkexec` resolves through the caller's `PATH`.
  Changing it to `/usr/bin/soos-enroll` changes two existing assertions:
  `import_privacy_tests::test_import_helper_args_use_stdin` (`"soos-enroll"` →
  `"/usr/bin/soos-enroll"`) and `import_privacy_tests::test_import_finalize_pipes_embedding_without_temp_file`
  (`"soos-enroll\nimport..."` → `"/usr/bin/soos-enroll\nimport..."`). Matrix GCV21 is Pending.
- `[pipeline] allow_virtual_camera` is not yet read by the `soos-daemon` loader
  (`crates/daemon/src/config.rs`) nor by `soos-enroll`; both always refuse virtual nodes (fail
  closed). The daemon loader also does not reject a NUL `camera_device` at load time: the guarded
  open turns it into a recoverable error instead of a panic.
- The absolute program paths assume the default installation prefix `/usr`
  (`scripts/install.sh --prefix` other than `/usr` would break the GUI privileged actions).

## Integration note: CAM-NEW-7(b) import path (owner-approved 2026-10-02)

The GUI import helper now calls `SOOS_ENROLL_PROGRAM` (`/usr/bin/soos-enroll`) like every other
privileged action, so pkexec never resolves the program through the caller's `PATH`. Owner-approved
change of two existing assertions in `crates/gui/tests/import_privacy_tests.rs`:

| Test | Old expectation | New expectation |
|---|---|---|
| `test_import_helper_args_use_stdin` | first argument `"soos-enroll"` | `"/usr/bin/soos-enroll"` |
| `test_import_finalize_pipes_embedding_without_temp_file` | `"soos-enroll\nimport\n…"` | `"/usr/bin/soos-enroll\nimport\n…"` |

The rest of both tests (stdin, no temporary file, `--yes` consent) is unchanged. With this, #314 is
fully delivered.

## Candid review follow-ups (batch merge)

- `VIRTUAL_CAPTURE_DRIVERS` also lists `akvcam` (a virtual webcam whose capture node reports
  capture-only capabilities) and the `vimc` test driver; red test
  `gcv_review_tests::test_gcv_akvcam_and_vimc_capture_nodes_are_rejected` (failed before the list
  change). Matrix GCV21 now cites the import-path tests; `Docs/GUI_APPLICATION.md` shows the
  absolute `pkexec /usr/bin/soos-enroll` commands.
- Both store crates fsync the key directory after publishing a new key (`sync_parent_dir`), so a
  crash right after first-boot creation cannot lose the key while templates encrypted with it
  survive; `scripts/build_arch.sh` refuses `--tar` / `--compress` without a value.
