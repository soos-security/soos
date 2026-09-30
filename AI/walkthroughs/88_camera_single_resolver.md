# Walkthrough 88 — Single Camera Resolver and GUI Camera Ownership

- **Date**: 2026-09-30
- **Issues**: Review findings CAM-02 (GitHub #150) and CAM-04 (GitHub #152) — **Branch**: `fix/camera-single-resolver`
- **Matrix criteria**: CSR1–CSR5 (new)

---

## 1. Context & Objectives

The full project review (`AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`, area Camera Detection & GUI)
confirmed two MAJOR camera bugs:

1. **CAM-04 (#152) — three divergent resolvers.** The daemon auto-detected with
   `select_camera_device(.., PreferIr)` when `camera_device` was the sentinel or absent on disk. The
   enrollment CLI (also used by the GUI direct mode) skipped auto-detection when `daemon.toml` or
   its `camera_device` key was missing and returned the alphabetically first `/dev/v4l/by-id/`
   entry, which can be a metadata node (`...-video-index1`) or the RGB sensor of an IR+RGB laptop.
   `"default"` was a sentinel in the CLI but only worked in the daemon "by accident" through
   `!exists()`. Result: enrollment on one sensor, authentication on another.
2. **CAM-02 (#150) — EBUSY ping-pong.** The GUI chose the daemon IPC only if
   `/run/soos/daemon.sock` *existed* and a preview round-trip succeeded. A user outside the `soos`
   group cannot traverse `/run/soos` (`0750`), so `exists()` was false and the GUI opened the camera
   directly while the daemon was running. uvcvideo lets the second open succeed and only fails
   `VIDIOC_S_FMT` with `EBUSY`, which was stringified into `SetFormat { reason }` instead of
   `DeviceBusy`. After the daemon's idle release the GUI grabbed the device and the next PAM
   request failed with `CameraUnavailable` → `PAM_IGNORE`, with nothing shown to the user.

Objective: one shared resolver used by all three binaries; the GUI never grabs the device while the
root daemon (exclusive owner of `/dev/video*`, ADR 2026-09-12) owns it, and surfaces a clear state.

## 2. Architect Design

- `crates/camera-v4l/src/resolver.rs` (new, small public API):
  - `AUTO_CAMERA_DEVICE` (`/dev/v4l/by-id/default-camera`), `is_auto_camera_device(&Path)`,
    `parse_sensor_preference(&str)`.
  - `trait CameraEnumerator { capture_devices(); by_id_aliases(); }` with the production
    `SystemCameraEnumerator` (sysfs + bounded `/dev/v4l/by-id` scan, 64 entries, dangling aliases
    skipped) and hermetic fakes in tests.
  - `resolve_camera_device(explicit, preference, &dyn CameraEnumerator) -> CameraResolution { path, source, sensor_type }`.
  - `CameraConfig::explicit_device()`.
- `CameraError::from_ioctl_error(path, err, fallback)` and `CameraError::is_device_busy()`.
- Daemon: `pipeline::resolve_pipeline_camera(&PipelineConfig, &dyn CameraEnumerator)`, called by
  `initialize_pipeline`; `config.rs` uses the shared sentinel and preference helpers.
- Enrollment CLI: `resolve_camera_device_from_config[_with]` reads `daemon.toml` and delegates to
  the shared resolver (CLI flag, then `camera_device`).
- GUI: `crates/gui/src/camera_mode.rs` — `probe_daemon_socket` / `classify_socket_error`
  (`DaemonSocketProbe`), `decide_camera_mode(socket, preview, daemon_active) -> CameraMode
  { DaemonIpc, DirectV4l, Blocked(CameraBlockReason) }`, `CameraBlockReason::user_message()`, and
  `UnavailableCameraManager`. `SoosApp::set_camera_notice` renders the reason in place of the spinner.

Decision recorded in `AI/DECISIONS.md` (2026-09-30). Behaviour change: the daemon no longer
replaces an explicit `camera_device` that is absent at startup; an explicit path now means the same
thing in every binary.

## 3. Tester Contract (TDD Red)

New test files, written before any production code:

- `crates/camera-v4l/tests/resolver_tests.rs` (10 tests, CSR1).
- `crates/camera-v4l/tests/error_recovery_tests.rs` (+4 tests, CSR5).
- `crates/enrollment-cli/tests/camera_resolver_parity_tests.rs` (4 tests, CSR2).
- `crates/daemon/tests/camera_resolution_tests.rs` (4 tests, CSR3).
- `crates/gui/tests/camera_mode_tests.rs` (7 tests, CSR4).

Red evidence (before implementation):

```
error[E0432]: unresolved imports `soos_camera_v4l::is_auto_camera_device`, `soos_camera_v4l::parse_sensor_preference`,
  `soos_camera_v4l::resolve_camera_device`, `soos_camera_v4l::CameraEnumerator`, `soos_camera_v4l::CameraResolutionSource`,
  `soos_camera_v4l::SystemCameraEnumerator`, `soos_camera_v4l::AUTO_CAMERA_DEVICE`
error[E0599]: no variant, associated function, or constant named `from_ioctl_error` found for enum `CameraError` (x3)
error[E0599]: no method named `is_device_busy` found for enum `CameraError` (x2)
```

The existing resolver contracts (`device_resolution_tests`, `import_tests::test_resolve_camera_device_from_daemon_config`,
`model_id_tests::test_camera_device_path_uses_stable_by_id`, `config_tests::test_pipeline_config_camera_device_auto_resolution`)
were kept unchanged and pass.

## 4. Auditor Constraints

1. No `unwrap`/`expect`/indexing in production paths (resolver, error mapping, GUI mode).
2. Bounded I/O: the by-id scan is capped at `MAX_BY_ID_ENTRIES` (64).
3. No frame, embedding or credential in logs; only device paths, sensor type and preference.
4. No new `unsafe`; `soos-gui` keeps `#![forbid(unsafe_code)]`.
5. `systemctl is-active` is the existing fixed-argument helper (`SoosApp::is_daemon_active`); no shell.
6. Fail closed: any ambiguity (daemon active, EACCES, preview refused) blocks direct access.

## 5. Implementation

- `v4l_impl.rs`: `set_format`, MMAP stream creation and `DQBUF` errors go through
  `from_ioctl_error`; `EBUSY` gets a dedicated supervisor warning.
- `main.rs` (GUI): connect-based probe, preview probe only when reachable, `decide_camera_mode`,
  blocked notice passed to `SoosApp`.
- Docs: `Docs/CAMERA_V4L_CRATE.md` (resolver and busy classification; fixed the "RGB by default"
  drift — the default has been `PreferIr`), `Docs/ENROLLMENT_CLI.md` (`--camera-device`),
  `Docs/IPC_PROTOCOL.md` (no direct fallback on preview refusal).

## 6. Validation

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`: clean.
- `cargo test --locked --workspace --all-targets --all-features`: green.
- `./scripts/candid_review.sh`: passed.

## 7. Follow-ups / Hardware Validation

- Live check on an IR+RGB laptop: `soos-enroll` and the daemon log the same by-id alias; the GUI
  run by a user outside `soos` shows the "add the user to the 'soos' group" notice and the daemon
  keeps authenticating while the GUI is open.
- Not in scope: provisioning `/etc/soos/daemon.toml` from the installer (no longer needed for
  convergence, since the missing file and `camera_device = "auto"` now resolve identically);
  daemon re-enumeration after hot-plug (#151/#153). Runtime switching of the GUI source after
  Pause/Resume is now implemented (walkthrough 90, section 10).

## 8. Candid Review Rework (2026-09-30, findings 3 and 4)

- **Finding 3**: `resolve_pipeline_camera` was exercised only by the parity tests while production
  called `plan_camera_device(.., auto_select_camera_device)`. `initialize_pipeline` now calls
  `resolve_pipeline_camera(config, &SystemCameraEnumerator::default())`, which is built on
  `plan_camera_device` (mock camera: no enumeration). All existing `camera_resolution_tests` and
  `camera_health_tests` pass unchanged.
- **Finding 4**: `stable_device_path` scanned `/dev/v4l/by-id` itself (bound 256) next to the
  resolver's `by_id_aliases` (bound 64). It now delegates to
  `SystemCameraEnumerator::with_by_id_dir(dir).by_id_aliases()`; the single bound is
  `MAX_BY_ID_ENTRIES` (64) in `resolver.rs`. The existing `test_stable_device_path_*` tests pass
  unchanged (sorted aliases keep the lexicographically-first choice).
- **Red evidence**: the new invariants `test_daemon_production_camera_resolution_uses_tested_resolver`
  ("initialize_pipeline must resolve its camera through resolve_pipeline_camera") and
  `test_single_bounded_by_id_scanner` ("stable_path.rs must not scan /dev/v4l/by-id itself") failed
  before the change. Matrix row CSR6.
