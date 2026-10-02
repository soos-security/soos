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
| `systemctl show --property=ActiveState --value soos-daemon.service` | `soos-gui-daemon-monitor` | `daemon_control` |
| `pkexec` (pause/resume daemon, list/import/delete templates) | `soos-gui-privileged` | `privileged` |
| Camera-source probing, switching and device release | `soos-gui-camera-source` | `camera_source` |

- **Daemon state polling**: `DaemonMonitor` probes through the `DaemonStatusProbe` trait
  (production: `SystemctlProbe`) at most once per `DAEMON_POLL_INTERVAL` (2 s), gated by the
  clock-injectable `PollThrottle`, and publishes `DaemonState::{Unknown, Active, Inactive}` in an
  atomic. `SystemctlProbe` reads the unit's `ActiveState` (`/usr/bin/systemctl show
  --property=ActiveState --value`) and maps it with `active_state_means_running`: only
  `inactive` and `failed` are `Inactive`; `active`, `reloading`, `refreshing`, `activating`
  (start-up, `auto-restart`) and `deactivating`, an unknown value and a `systemctl` that cannot
  be run all count as `Active`, so the GUI never opens the device directly while the daemon is
  restarting (EBUSY fight; GitHub #314, CAM-NEW-7). The header reads it lock-free. After Pause/Resume the UI calls `request_refresh()` so the
  new state appears within one monitor tick (50 ms) instead of the next interval.
- **Privileged operations**: the UI submits a `PrivilegedAction` to `TaskRunner`, which runs it on
  a worker thread through the `PrivilegedExecutor` trait (production: `PkexecExecutor`, fixed
  argument vectors, no shell, stdin closed) and returns immediately. Programs are named by
  absolute path, never resolved through the caller's `PATH` (GitHub #314, CAM-NEW-7):
  `PKEXEC_PROGRAM` (`/usr/bin/pkexec`), `SYSTEMCTL_PROGRAM` (`/usr/bin/systemctl`) and
  `SOOS_ENROLL_PROGRAM` (`/usr/bin/soos-enroll`, where `scripts/install.sh` with its default
  prefix, the Debian, Arch and RPM packages install it). The template import passes the same
  absolute `SOOS_ENROLL_PROGRAM` from `privileged::import_helper_args` (pinned by the
  `import_privacy_tests` contract, owner-approved 2026-10-02, walkthrough 168). The `PrivilegedOutcome` comes
  back over an `mpsc` channel that `SoosApp::handle_task_outcomes` drains each frame; the worker
  wakes the UI with `request_repaint`. Only one privileged action runs at a time
  (`TaskRunnerError::Busy`), so the user never faces stacked Polkit dialogs; the Pause/Resume
  buttons are disabled and a spinner is shown meanwhile.
- **Direct-mode store mutations (GitHub #291)**: with a local store (`GuiStore::System` or
  `GuiStore::Developer`) the UI never calls `BiometricStore::enroll` / `delete` itself: both
  take the store lock and may wait up to `STORE_LOCK_TIMEOUT` (5 s) while `soos-enroll` holds
  it. The UI submits a `store_tasks::StoreTask` (`Enroll(template)` or `Delete { uid }`) to
  `StoreTaskRunner`, which runs it on the `soos-gui-store` thread and returns immediately; the
  `StoreTaskOutcome` is drained each frame by `SoosApp::handle_store_task_outcomes` and the
  worker wakes the UI with `request_repaint`. One store task runs at a time
  (`StoreTaskSubmitError::Busy`). A lock timeout is shown as `StoreTaskError::Busy`, whose
  message is `STORE_BUSY_MESSAGE` ("another soos operation is using the template store; try
  again"); nothing is written or removed. `StoreTask`'s `Debug` output prints only the UID and
  embedding dimension. A separate runner (rather than new `PrivilegedAction` variants) keeps
  local saves independent of a pending Polkit dialog (Pause/Resume) and leaves the
  `PrivilegedExecutor` contract unchanged.
- **Template import (GitHub #156, review findings CAM-08 / STO-12)**: the fused embedding is
  serialized to JSON in a zeroizing buffer and piped to the standard input of
  `pkexec /usr/bin/soos-enroll import --uid <uid> --file -` (`privileged::import_helper_args`,
  `import_template_with`). No file is created, under the temporary directory or anywhere else;
  the child is always reaped, even if it exits before reading its stdin (denied Polkit prompt).
  The fused embedding is held in `Zeroizing<Vec<f32>>` from the enrollment session to the
  pipe. `PrivilegedAction`'s `Debug` output prints only the UID and embedding dimension, never
  embedding values.
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

## 1b. Biometric Store Selection (GitHub #156)

`store_mode::resolve_gui_store` picks the store once at startup; there is no implicit fallback
location (the former silent `soos-gui-master.key` / `soos-gui-biometrics` store in the system
temporary directory is gone):

| Mode | When | Templates |
|---|---|---|
| `GuiStore::System` | `--key-file` and `--biometrics-dir` are accessible (root session) | written directly to the system store |
| `GuiStore::Polkit` | the system store is not accessible (unprivileged session) | no local store at all; list / import / delete go through `pkexec /usr/bin/soos-enroll` |
| `GuiStore::Developer` | explicit `--dev-store <DIR>` (absolute path) | `<DIR>/master.key` and `<DIR>/biometrics` (created `0700`); never used by PAM |

The developer mode shows a persistent orange banner under the header ("DEVELOPER STORE:
templates are saved in `<DIR>` and are NOT used by PAM") and logs it at startup; it never
imports into the system store. In Polkit mode the Profiles tab lists the system templates but
the in-process match test needs a readable template (root session or `--dev-store`).

```bash
soos-gui                              # system store (root) or Polkit mode (user)
soos-gui --mock --dev-store "$HOME/.local/share/soos-dev"   # hardware-free development
```

`--mock` requires `--dev-store` (clap `requires`, GitHub #314 CAM-NEW-5): templates computed by
the mock models carry the production model id, so they must never be imported into the system
store; `soos-gui --mock` alone is a usage error.

Verification: `crates/gui/tests/import_privacy_tests.rs` (matrix rows ISE1–ISE4).

## 1c. Guided Enrollment Liveness (GitHub #217 / #218)

The worker feeds every analyzed frame to the guided enrollment session through
`worker::feed_guided_enrollment`, and the session is created by
`worker::new_guided_enrollment_session` with `LivenessPolicy::strict()`:

- a sample needs 3 consecutive live frames (PAD live and score >= `pad_threshold`, NaN rejects);
- a spoof or below-threshold frame discards the current step; the third one aborts the session
  ("Enrollment aborted: repeated spoof detections. Cancel and restart.");
- a frame rejected by the pre-PAD quality gate (`VisionAnalysis::quality_rejection`) shows
  "Face too small or blurred: move closer and hold still." and is never sampled;
- a frame with a face but no PAD verdict, or with no face, breaks the live streak without counting
  as a spoof;
- a spoof PAD verdict is counted even when the frame yields no pose or no embedding
  (`GuidedEnrollmentSession::record_presentation_attack`, GitHub #285);
- only a frame with exactly one face is ever sampled (GitHub #304): `VisionPipeline::analyze_frame`
  reports every detection (`VisionAnalysis::face_count`) but runs PAD, alignment and embedding
  only for a single face, `feed_guided_enrollment` breaks the live streak and returns `None` for
  any other count, and `worker::guided_enrollment_feedback` shows "One face only: make sure
  nobody else is in view of the camera." (`GuiEnrollmentFeedback::OneFaceOnly`) for a frame
  with several faces. This is the rule of the CLI enrollment (`process_frame` rejects more than
  one detection).

## 1d. Live Verification Reference (GitHub #278 / #298)

Selecting a profile in the live-verification panel calls `worker::select_match_reference` with
the stored template (or `None` when the store lookup fails or finds nothing):

- only a template of the loaded embedding model (`template_matches_model`: same id, same
  dimension) becomes the match reference;
- a foreign template (for example a pre-SFace `arcface_w600k_mbf` 512-D one) clears the reference
  and shows "Re-enrollment required: this template was enrolled with model '<id>' (<n>-D)";
- a failed or empty lookup clears the reference and the note;
- in every case the score computed against the previous reference is withdrawn in the same
  critical section that installs the new reference. The worker publishes scores through
  `worker::update_live_match_score`, which holds the reference lock while it writes the score,
  so no stale score is ever displayed for the new selection.
- the reference is a `Zeroizing<Vec<f32>>` cloned straight from the template's zeroized vector,
  so the template copy is wiped when it is replaced, cleared or dropped (row SGF5).

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
| `SourceUnavailable` | Daemon camera unavailable | Daemon has no camera frame to serve (`Verdict::Unavailable`, or an empty preview after the first frame) |
| `SourceProtocol` | Daemon preview protocol error | GUI/daemon version mismatch, stale or unstamped daemon `Response` |

`IpcCameraManager` maps each `IpcPreviewError` to the matching `Source*` kind
(`IpcPreviewError::kind`). Every preview reply goes through `ipc_camera::frame_from_preview`
(GitHub #305, #306, #314 S2):

| Reply | Before the first frame | After a frame was shown |
|---|---|---|
| Empty preview (no data, `format = 255`) | `Starting` ("Connecting to camera"), keep polling | frame and overlays withdrawn, not ready, `SourceUnavailable` until frames resume |
| Known format, payload length equal to the geometry (any non-empty length for MJPEG) | frame shown, `Ready` | frame shown, `Ready`, error cleared |
| Unknown format code, `255` with data, zero dimension with data, length mismatch | `SourceProtocol`, reconnect | `SourceProtocol`, reconnect |
| Greyscale (`format = 1`, how the daemon sends every infrared frame) | Monochrome PAD path (IR gate, IR threshold) | same |

Unknown format codes used to be decoded as RGB24 and an empty preview used to leave the last
frame frozen on screen as `Ready`. The reply buffer and the published frame copies
(`LatestFrameData::rgb`, `aligned_crop`) are `Zeroizing`, and `LatestFrameData`'s `Debug`
prints only lengths (GitHub #314, CAM-NEW-6). A daemon refusal (`Response` echoing the request nonce) is trusted
only after `Response::check_freshness` with the shared `MAX_RESPONSE_FUTURE_SKEW_NS` against
CLOCK_MONOTONIC, exactly like `pam_soos.so` and `soos-admin test-pam`: a stale, unstamped or
future-dated `Response` is `IpcPreviewError::Protocol`, and no `Response` is ever an authorized
preview (GitHub #289, `Docs/IPC_PROTOCOL.md` §9). In direct mode the device comes from the shared
`daemon.toml` reader (`Docs/CAMERA_V4L_CRATE.md`); its notes (unusable file, ignored key, never a
value) are logged as warnings. Non-error states render as "Connecting to camera" (`Starting`),
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

Matrix rows GRE1–GRE6 and ISE1–ISE4 in `AI/VERIFICATION_MATRIX.md`; tests in
`crates/gui/tests/responsiveness_tests.rs`, `crates/gui/tests/camera_status_tests.rs`,
`crates/gui/tests/import_privacy_tests.rs` and `crates/camera-v4l/tests/camera_status_tests.rs`.
Direct-mode store mutations off the UI thread: rows SGU1–SGU2, `crates/gui/tests/store_task_tests.rs`.
Live verification reference selection: rows SGF1–SGF2, `crates/gui/tests/match_reference_selection_tests.rs`;
wipe-on-drop reference: row SGF5, `crates/gui/tests/match_reference_zeroize_tests.rs`.
Failure paths without a daemon or camera (oversized, zero-length and truncated preview replies,
daemon without camera, `EACCES` socket, direct-mode `EACCES` / `EBUSY`) are covered by
`crates/gui/tests/ipc_camera_failure_tests.rs` (matrix CHT5–CHT6, GitHub #198).
