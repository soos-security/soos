# Walkthrough 90 — GUI Responsiveness and Camera Error States

- **Date**: 2026-09-30
- **Issues**: Review findings CAM-06 (GitHub #154) and CAM-07 (GitHub #155) —
  **Branch**: `fix/gui-responsiveness-and-errors`
- **Matrix criteria**: GRE1–GRE6 (new)

---

## 1. Context & Objectives

The full project review (`AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`, area Camera Detection &
GUI) confirmed two MAJOR findings in `soos-gui`:

1. **CAM-06**: `render_header` ran `systemctl is-active` through `std::process::Command` on every
   `ui()` pass. The vision worker requests a repaint per camera frame, so the GUI forked about 30
   processes per second on the UI thread. The Pause/Resume buttons ran `pkexec ... .status()`
   synchronously from the click handler, freezing the window for as long as the Polkit dialog was
   open. The template import, delete and system profile listing also ran `pkexec` on the UI thread.
2. **CAM-07**: no `tracing` subscriber was installed although `tracing-subscriber` was a declared
   dependency, so every `warn!`/`info!` in the GUI process (including the V4L supervisor's
   `Camera error on ...`) was dropped; `CameraManager` exposed no status accessor and the UI showed
   the same "Connecting to camera and initializing models..." spinner for every failure.

Objectives: throttled background service polling, every blocking privileged call off the UI thread
with a channel back to it, a stderr tracing subscriber, and distinct user-visible camera error
states — without ever logging frames or embeddings.

## 2. Architect Design

- `soos-camera-v4l::status` (new module):
  - `CameraErrorKind` (11 variants: device kinds `DeviceNotFound`, `DeviceBusy`,
    `PermissionDenied`, `UnsupportedDevice`, `Starved`, `Io`, and remote source kinds
    `SourceUnreachable`, `SourceUnauthorized`, `SourceRateLimited`, `SourceUnavailable`,
    `SourceProtocol`), `CameraErrorKind::ALL`, `from_os_code`, English `Display`.
  - `CameraError::kind()` — classification that also looks inside the generic `Io` variant, so
    `EACCES` becomes `PermissionDenied` without changing `from_io_error` or any variant.
  - `CameraStatus { Starting, Ready, Suspended, Stopped, Error { kind, failures } }` and the
    thread-safe `CameraStatusCell` (`record_error` counts consecutive same-kind failures,
    saturating).
  - `CameraManager::status()` with a default derived from `is_ready()` — existing implementors
    (daemon test spies) compile unchanged.
- `soos-gui` (new modules, keeping the shared camera functions untouched):
  - `daemon_control`: `DaemonStatusProbe` trait, `SystemctlProbe`, `PollThrottle` (injectable
    `now: Instant`), `DaemonState`, `DaemonMonitor` (background thread, atomic state,
    `request_refresh`), `DAEMON_POLL_INTERVAL = 2 s`.
  - `privileged`: `PrivilegedAction` / `PrivilegedOutcome`, `PrivilegedExecutor` trait,
    `PkexecExecutor`, `TaskRunner` (one action in flight, `mpsc` result channel, repaint
    callback), `TaskRunnerError::{Busy, Spawn}`, `MAX_PROFILE_LIST_BYTES`.
  - `camera_status`: `StatusBanner`, `BannerSeverity`, `camera_status_banner`,
    `render_status_banner`.
  - `logging`: `DEFAULT_LOG_DIRECTIVE`, `build_env_filter`, idempotent `init`.
- Out of scope (parallel work on #150/#152): switching between the IPC and direct camera managers
  after Pause/Resume (the `CameraSource` state machine from the CAM-06 recommendation). With this
  change the stalled IPC case is at least visible as "Daemon preview unreachable".

## 3. Plan Evaluation

Checked against `AI/ARCHITECTURE.md` and `AGENTS.md`: no change to PAM, IPC framing or daemon; no
`unsafe`; business-crate `#![forbid(unsafe_code)]` kept and added to the new GUI modules; no new
dependency (`tracing-subscriber` already had `fmt` and `env-filter`); `CameraManager` changes are
additive. The GUI keeps the root daemon as camera owner; nothing new opens `/dev/video*`.

## 4. Tester Contract (Red Phase)

| Test file | Tests | Contract |
|---|---|---|
| `crates/camera-v4l/tests/camera_status_tests.rs` | 9 | GRE5 |
| `crates/gui/tests/responsiveness_tests.rs` | 11 | GRE1–GRE3 |
| `crates/gui/tests/camera_status_tests.rs` | 10 | GRE4, GRE6 |

Red evidence:

- Before any production code, all three test targets failed to compile on the specified API only:
  `unresolved imports soos_camera_v4l::{CameraErrorKind, CameraStatus, CameraStatusCell}`,
  `no method named kind found for enum CameraError`, `no method named status`,
  `unresolved import soos_gui::{camera_status, daemon_control, logging, privileged}`.
- With the new modules in place but `app.rs`/`main.rs` not yet migrated, the static guards failed
  on assertions: `test_app_ui_code_spawns_no_process ... FAILED` (app.rs still contained
  `Command::new`) and `test_gui_main_installs_tracing_subscriber_first ... FAILED` (no
  `soos_gui::logging::init()` in `main.rs`).

No existing test was modified, weakened or deleted.

## 5. Auditor Constraints

1. No `unwrap`/`expect`/`panic` in new production code; poisoned mutexes are recovered with
   `into_inner`.
2. Bounded concurrency: one monitor thread (joined on drop, 50 ms shutdown tick) and at most one
   privileged worker at a time; `soos-enroll list` output capped at 1 MiB.
3. `pkexec`/`systemctl` use fixed argument vectors, no shell, stdin closed.
4. The embedding handed to `soos-enroll import` lives in `Zeroizing` buffers; the temporary file is
   created `0600` with `O_EXCL` under a random name (previously `std::fs::write` with the process
   umask and a predictable name) and removed by a drop guard on every path.
5. `PrivilegedAction`'s `Debug` prints only the UID and embedding dimension.
6. Logs carry error kinds, failure counts, device paths and service state only, and transitions are
   logged once (no per-frame log spam). Frames, embeddings and credentials are never logged.
7. A failure to spawn the monitor thread degrades to `DaemonState::Unknown`; a failure to spawn a
   privileged worker is reported in the UI; neither panics.

## 6. Implementation

- `crates/camera-v4l/src/status.rs` (new), `manager.rs` (default `status()`), `mock.rs`
  (`status()` reports injected error/starvation/stop), `v4l_impl.rs` (supervisor records
  `err.kind()` in a `CameraStatusCell`, restarts the count after a session that published frames,
  reports `Suspended` and `Stopped`; the existing `warn!` is kept).
- `crates/gui/src/ipc_camera.rs`: `IpcPreviewError::kind()`, a `CameraStatusCell` updated by
  `WorkerState::set_error`, transition-only `warn!`/`info!`, `status()` override. The public
  `last_error()` API is unchanged.
- `crates/gui/src/app.rs`: removed `SoosApp::{is_daemon_active, pause_daemon, resume_daemon}` and
  all `Command::new` calls. The header reads `DaemonMonitor::state()`, Pause/Resume submit to the
  `TaskRunner` (buttons disabled with a spinner while busy, errors shown next to them);
  `handle_task_outcomes` applies results at the start of every frame; profile listing, import and
  delete go through the runner; the no-frame placeholder renders the camera status banner and the
  header shows its title; camera status transitions are logged.
- `crates/gui/src/main.rs`: `soos_gui::logging::init()` before argument parsing.

## 7. Candid Review

`./scripts/candid_review.sh` (Layer 1): PASSED — no unsafe additions, no PAM changes, no forbidden
dependencies, English-only additions. Layer 2 (`candid_subagent.sh`) is run by the release loop.

## 8. Verification Results

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | clean |
| `cargo test --locked --workspace --all-targets --all-features` | 591 passed, 0 failed (final run). Earlier runs under a host load average of 17–46 (parallel agent builds) hit two pre-existing timing-sensitive tests in untouched code paths — `soos-vision` `bench_tests::test_pipeline_latency_budget_under_150ms_p95` and `soos-camera-v4l` `mock_camera_tests::test_mock_camera_idle_throttling_and_wake` (50 ms sleeps) — both pass on re-run |
| `./scripts/candid_review.sh` | PASSED |

## 9. Known Limitations / Follow-ups

- The IPC-vs-direct camera source swap after Pause/Resume (CAM-06 recommendation) is left to the
  #150/#152 camera-resolver work; the GUI now at least reports "Daemon preview unreachable".
- The negotiated capture format is not yet shown in the header (only resolution, FPS and latency
  once frames arrive).
- Matrix row GEPU3 still cites the removed `SoosApp::is_daemon_active/pause_daemon/resume_daemon`
  helpers; GRE1/GRE2 supersede that evidence.
