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

- ~~The IPC-vs-direct camera source swap after Pause/Resume~~: implemented in section 10.
- The negotiated capture format is not yet shown in the header (only resolution, FPS and latency
  once frames arrive).
- ~~Matrix row GEPU3 cites removed helpers~~: marked superseded by GRE1, GRE2, GRE7 and GRE8.

## 10. Candid Review Rework (2026-09-30, findings 1, 5, 7, 8)

### Finding 1 — runtime camera-source switching (#154 / #150)
The source was chosen once in `main.rs`. Starting the GUI with the daemon paused and pressing
**Resume** made the daemon hit `EBUSY` on `VIDIOC_S_FMT` (the GUI kept streaming); pressing
**Pause** in IPC mode left the feed blank until restart.

- `camera_source::CameraSourcePlanner` (pure, injected clock, lazy probe): an unknown daemon state
  never selects a source; a direct source is left as soon as the daemon is active or a Resume
  handover is pending (the decision can then never be `DirectV4l`); IPC is re-evaluated only while
  the daemon is not active; steady sources are never re-probed.
- `SwitchableCamera` is the `CameraManager` handed to the vision worker. `CameraSourceSupervisor`
  (own thread, 100 ms tick) applies decisions and always stops and drops the previous source
  (`release_manager` waits for the last transient reference, then the drop joins the capture
  thread) **before** opening the next one. The UI drops the displayed frame when the source
  generation changes, and the worker withdraws a frozen frame when the source stops being ready.
- `HandoverExecutor` wraps `PkexecExecutor`: for `ResumeDaemon` it releases the direct V4L2 manager
  on the privileged worker thread before `pkexec systemctl start`, keeps direct mode disabled while
  the Resume is pending and until the daemon is seen active (10 s grace), and re-enables it at once
  when the Resume fails. `main.rs` no longer runs a one-shot startup decision.

### Finding 5 — accurate retry messaging
`CameraBlockReason::is_transient` (rate limit, I/O, unavailable, daemon unreachable or starting,
direct open failure) drives bounded re-probes (`MAX_TRANSIENT_PROBE_RETRIES` = 60, every 1 s);
permanent reasons are re-evaluated only on a daemon state change. `camera_status::error_is_retried`
removes the "retrying automatically" claim for `SourceUnauthorized` (the IPC worker stops for good).

### Finding 7 — bounded helper output
`list_profiles` pipes the helper stdout through `privileged::read_bounded`
(`Read::take(MAX_PROFILE_LIST_BYTES + 1)`) instead of `Command::output()`, and kills the helper on an
oversized or failed read.

### Finding 8 — matrix
GEPU3 is marked superseded (its history is kept); new rows GRE7–GRE10, PIR6 and CSR6.

### Red evidence
- `crates/gui/tests/camera_source_tests.rs` (19 tests) against a stubbed module: 17 failed on
  assertions (planner never switching, `SwitchableCamera::replace` not installing,
  `release_manager` not dropping, `HandoverExecutor` not releasing; e.g.
  `test_handover_releases_direct_camera_before_resume_executes`: "the direct V4L2 manager must be
  dropped before pkexec starts the daemon"). The two that passed assert fail-closed defaults.
- `crates/gui/tests/camera_status_retry_tests.rs`: 2/2 failed with `error_is_retried` stubbed to
  `true`.
- `crates/gui/tests/bounded_output_tests.rs`: 2/4 failed (a 4 MiB reader was consumed whole;
  `list_profiles` still used `.output()`).
- The pre-existing contract `camera_status_tests::test_gui_main_installs_tracing_subscriber_first`
  requires `main.rs` to log after installing the subscriber; `main.rs` keeps a startup log line for
  the camera-source policy (the test is unchanged).
- The threaded supervisor tests were run 10 times in a row: 10/10 green.

### Follow-ups (not addressed in this rework)
- Finding 6 (IR corpus through `VisionPipeline`): walkthrough 86, section 8.
- Suggestions 9 (single camera state machine) and 10 (unplug during standby): walkthrough 89,
  section 7.
- Hardware validation of the handover on a dual-sensor laptop: start the GUI with the daemon
  paused, press **Resume**; the daemon log must show the stream opening without `EBUSY`.
