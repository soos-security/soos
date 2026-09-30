# `soos-gui` — Responsiveness, Privileged Operations and Diagnostics

`soos-gui` (crate `crates/gui`) is the eframe/egui desktop application for live model
diagnostics, guided enrollment and profile management. This page documents its threading model
and how it reports failures. Camera device selection and the daemon preview proxy are covered by
`Docs/IPC_PROTOCOL.md` and `Docs/CAMERA_V4L_CRATE.md`.

## 1. Threading Model (GitHub #154, review finding CAM-06)

The UI thread (`SoosApp::ui`) runs once per repaint, and the vision worker requests a repaint for
every camera frame (about 30 per second). The UI thread therefore never spawns a process and never
waits on I/O it does not own.

| Work | Thread | Module |
|---|---|---|
| Camera capture / daemon preview polling | `soos-v4l-capture` or `soos-gui-ipc-cam` | `soos-camera-v4l`, `ipc_camera` |
| Face detection, PAD, embedding | `soos-gui-worker` | `worker` |
| `systemctl is-active soos-daemon.service` | `soos-gui-daemon-monitor` | `daemon_control` |
| `pkexec` (pause/resume daemon, list/import/delete templates) | `soos-gui-privileged` | `privileged` |
| Camera-source probing, switching and device release | `soos-gui-camera-source` | `camera_source` |

- **Daemon state polling**: `DaemonMonitor` probes through the `DaemonStatusProbe` trait
  (production: `SystemctlProbe`) at most once per `DAEMON_POLL_INTERVAL` (2 s), gated by the
  clock-injectable `PollThrottle`, and publishes `DaemonState::{Unknown, Active, Inactive}` in an
  atomic. The header reads it lock-free. After Pause/Resume the UI calls `request_refresh()` so the
  new state appears within one monitor tick (50 ms) instead of the next interval.
- **Privileged operations**: the UI submits a `PrivilegedAction` to `TaskRunner`, which runs it on
  a worker thread through the `PrivilegedExecutor` trait (production: `PkexecExecutor`, fixed
  argument vectors, no shell, stdin closed) and returns immediately. The `PrivilegedOutcome` comes
  back over an `mpsc` channel that `SoosApp::handle_task_outcomes` drains each frame; the worker
  wakes the UI with `request_repaint`. Only one privileged action runs at a time
  (`TaskRunnerError::Busy`), so the user never faces stacked Polkit dialogs; the Pause/Resume
  buttons are disabled and a spinner is shown meanwhile.
- **Template import**: the fused embedding is serialized into a fresh `0600` file created with
  `O_EXCL` under a random name, passed to `pkexec soos-enroll import`, and removed on every exit
  path (drop guard). `PrivilegedAction`'s `Debug` output prints only the UID and embedding
  dimension, never embedding values.
- The `soos-enroll list` output accepted from the helper is bounded by
  `MAX_PROFILE_LIST_BYTES` (1 MiB) while it is read (`read_bounded`, `Read::take`); an oversized
  output kills the helper.

## 1a. Runtime Camera Source (GitHub #154 / #150)

The camera source is not chosen once at startup. `camera_source::CameraSourceSupervisor` follows
the `DaemonMonitor` state and installs the source in the `SwitchableCamera` read by the vision
worker:

| Daemon state | Source |
|---|---|
| Unknown (no probe yet) | none: the device is never opened (fail-closed) |
| Active, preview authorized | daemon IPC preview (`IpcCameraManager`) |
| Active, preview refused / socket unreachable | blocked notice (`CameraBlockReason`) |
| Inactive (paused) | direct V4L2 through the shared resolver (`resolve_camera_device_from_config`) |

The previous source is always stopped and dropped (device closed) before the next one is opened.
**Resume** first releases a direct V4L2 manager on the privileged worker thread
(`HandoverExecutor`) and keeps direct mode disabled until the daemon is seen active, so the daemon
never meets `EBUSY` because of the GUI; a failed Resume re-enables direct mode. Transient blocks
(rate limit, I/O, daemon starting) are re-probed every second, at most
`MAX_TRANSIENT_PROBE_RETRIES` times; permanent ones (not authorized, not in the `soos` group) only
when the daemon state changes. The decision logic is the pure `CameraSourcePlanner`.

## 2. Camera Error States (GitHub #155, review finding CAM-07)

Every `CameraManager` exposes `status() -> CameraStatus` (see `Docs/CAMERA_V4L_CRATE.md`). When no
analyzed frame is available, the central panel renders `camera_status::camera_status_banner`
instead of a generic spinner, and the header shows the banner title. Each `CameraErrorKind` has a
distinct title and an actionable hint, plus the consecutive failure count and whether the source
retries automatically (`error_is_retried`: every kind except `SourceUnauthorized`, where the IPC
preview worker stops):

| Kind | Title | Typical cause |
|---|---|---|
| `DeviceNotFound` | Camera not found | Wrong `camera_device`, unplugged camera |
| `DeviceBusy` | Camera is busy | `soos-daemon` owns the device (`EBUSY`) |
| `PermissionDenied` | Camera permission denied | User not in `video` group (`EACCES`) |
| `UnsupportedDevice` | Camera format unsupported | No decodable capture format |
| `Starved` | Camera stopped sending frames | Privacy shutter, cable |
| `Io` | Camera I/O error | Other V4L2 failure |
| `SourceUnreachable` | Daemon preview unreachable | Daemon paused or stopped |
| `SourceUnauthorized` | Daemon preview not authorized | UID not in `[preview] allowed_uids` |
| `SourceRateLimited` | Daemon preview rate-limited | `[preview] max_requests_per_sec` |
| `SourceUnavailable` | Daemon camera unavailable | Daemon has no camera frame to serve |
| `SourceProtocol` | Daemon preview protocol error | GUI/daemon version mismatch |

`IpcCameraManager` maps each `IpcPreviewError` to the matching `Source*` kind
(`IpcPreviewError::kind`). Non-error states render as "Connecting to camera" (`Starting`),
"Camera ready" (`Ready`), "Camera in standby" (`Suspended`) and "Camera stopped" (`Stopped`).

## 3. Logging

`main.rs` calls `soos_gui::logging::init()` before anything else. It installs a
`tracing_subscriber::fmt` subscriber writing to **stderr**, filtered by `RUST_LOG`
(`build_env_filter`; unset, empty or invalid directives fall back to `info`). Initialization is
idempotent and never panics. Logged events carry error kinds, failure counts, device paths and
service state only: camera status transitions (UI thread, logged once per transition), preview
failures and recoveries (`ipc_camera`, once per transition), daemon state changes and privileged
operation failures. Frames, embeddings and credentials are never logged.

Example: `RUST_LOG=soos_gui=debug,soos_camera_v4l=debug soos-gui`.

## 4. Verification

Matrix rows GRE1–GRE6 in `AI/VERIFICATION_MATRIX.md`; tests in
`crates/gui/tests/responsiveness_tests.rs`, `crates/gui/tests/camera_status_tests.rs` and
`crates/camera-v4l/tests/camera_status_tests.rs`.
