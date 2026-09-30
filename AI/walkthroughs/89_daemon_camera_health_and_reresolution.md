# Walkthrough 89 — Daemon Camera Re-Resolution, Truthful Health and Supervisor Panic Safety

- **Date**: 2026-09-30
- **Issues**: Review findings CAM-03 (GitHub #151), CAM-05 / DMN-07 (GitHub #153), plus the
  `panic = "unwind"` follow-up from walkthrough 78 — **Branch**: `fix/daemon-camera-health`
- **Matrix criteria**: CSH1–CSH6 (new)

---

## 1. Context & Objectives

The full project review (`AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`) confirmed three defects
behind "the camera is sometimes detected wrongly":

1. **CAM-03** — `initialize_pipeline` resolved the camera once at startup. When nothing was found
   the `/dev/v4l/by-id/default-camera` sentinel was handed to the supervisor, which retried it
   forever; a camera re-enumerated under a new `/dev/videoN` after suspend or replug was never
   picked up. Worse, `|| !device_path.exists()` silently replaced an explicitly configured by-id
   link that udev had not created yet with *another* camera (e.g. RGB instead of IR).
2. **CAM-05 / DMN-07** — `main.rs` called `health.set_camera_ready(true)` right after
   `V4lCameraManager::spawn`, which only spawns a thread. `soos-admin status`, the GUI and
   `is_healthy` reported a ready camera during permanent `DeviceNotFound`/`EBUSY` backoff. The
   verifier noted that `is_ready()` alone is also `false` during auto-standby, so the fix must
   distinguish idle from failed.
3. **Follow-up of walkthrough 78** — with `panic = "unwind"` in release, a panic in the
   `soos-v4l-capture` thread no longer aborts the daemon; the thread just died, leaving a stale
   readiness state.

## 2. Architect Design

Scope is kept to daemon supervision and health so it does not overlap the shared camera resolver
work (#150/#152) or GUI/PAD changes.

- `soos_camera_v4l::CameraHealth` (`Starting`, `Streaming`, `Standby`, `Recovering`, `Dead`) with
  `is_operational()` = `Streaming | Standby`; new `CameraManager::health()` with a default derived
  from `is_ready()` so existing implementors (`IpcCameraManager`, test spies) are unaffected.
- `DevicePathResolver` trait (blanket impl for `Fn() -> Option<PathBuf> + Send + Sync`), so the
  shared resolver from #150/#152 can be plugged in as a closure.
- `V4lCameraManager::spawn_with_resolver(config, resolver)`; `spawn(config)` keeps a fixed path.
  `current_device_path()` exposes the path the supervisor currently targets.
- `stable_device_path(node, by_id_dir)` maps `/dev/videoN` to its `/dev/v4l/by-id/...` link.
- Daemon: `AUTO_SELECT_DEVICE_SENTINEL`, `plan_camera_device(cfg, auto_select)` (enumeration seam),
  `camera_device_resolver(cfg)` (`Some` only in auto-selection mode), `auto_select_camera_device`.
- `HealthState::attach_camera(Arc<dyn CameraManager>)` / `camera_health()`; `snapshot()` derives
  `camera_ready` from the attached camera, else from the legacy flag. `HealthStatus` is unchanged,
  so the IPC `StatusResponse` schema is unchanged.

## 3. Tester Contract (Red Phase)

Stubs with the old behavior were added first so that the contract compiled and failed on
assertions (not only on compilation):

- `crates/camera-v4l/tests/supervision_tests.rs` — 9 tests; 6 failed red
  (`test_v4l_health_recovering_when_device_missing`, `test_supervisor_reresolves_device_after_enodev`,
  `test_supervisor_reresolution_is_bounded_by_backoff`, `test_supervisor_panic_marks_camera_dead`,
  `test_mock_health_tracks_streaming_failure_and_standby`, `test_stable_device_path_prefers_by_id_link`).
- `crates/daemon/tests/camera_health_tests.rs` — 7 tests; 6 failed red, including
  `test_pipeline_init_keeps_configured_by_id_when_missing_and_retries`, which failed on the old
  `|| !exists()` substitution (planned path became `/dev/video7`), and
  `test_camera_ready_false_when_device_missing`.

All tests are hardware-free: nonexistent V4L2 paths drive the real supervisor into
`DeviceNotFound`, and `MockCameraManager` covers streaming, error and standby. A panicking
resolver closure injects the supervisor fault.

## 4. Auditor Constraints

1. No `unwrap`/`expect`/indexing in production paths; poisoned locks use `into_inner()`.
2. The resolver is called at most once per backoff period and only after `DeviceNotFound` /
   `UnsupportedCapability`; backoff stays capped at `max_backoff`.
3. Unknown encoded health values decode to `Dead` (fail-closed).
4. `catch_unwind` wraps the whole supervisor; after a panic readiness and the published frame are
   withdrawn and the thread is never restarted. The panic payload is not logged.
5. The by-id scan is bounded (256 entries) and ignores unreadable or dangling links.
6. No frame, embedding or credential is logged; only device paths and state labels.

## 5. Implementation (Green Phase)

- `crates/camera-v4l/src/manager.rs`: `CameraHealth`, `CameraManager::health()` default.
- `crates/camera-v4l/src/v4l_impl.rs`: shared `AtomicU8` health and `RwLock<PathBuf>` current path;
  `run_v4l_supervisor` = `catch_unwind` around the new `supervise` loop; health transitions
  Starting → Streaming (first stabilized frame) → Standby (auto-standby) → Starting (resume),
  Recovering on any error; re-resolution after the backoff sleep resets backoff when the path
  changes. `health()` reports `Standby` for a streaming camera past its idle timeout.
- `crates/camera-v4l/src/mock.rs`: `health()` override (error/starved → Recovering, idle → Standby).
- `crates/camera-v4l/src/stable_path.rs`: `stable_device_path`, `DEFAULT_BY_ID_DIR`.
- `crates/daemon/src/pipeline.rs`: no silent substitution; auto-selection spawns the supervisor
  with a re-enumerating resolver returning by-id paths.
- `crates/daemon/src/health.rs`, `crates/daemon/src/main.rs`: live `camera_ready`; a 500 ms task
  logs camera state transitions.

The systemd unit was left unchanged: late device arrival is now handled by re-resolution rather
than by boot ordering.

## 6. Verification

```bash
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test   --locked --workspace --all-targets --all-features
./scripts/candid_review.sh
```

All four passed. The new suites pass 9/9 and 7/7, and the pre-existing `health_tests` and
`pipeline_init_tests` still pass unchanged.

## 7. Follow-ups

- Expose a `camera_state` string / last-frame age in a v2 `StatusResponse` (protocol change, out of
  scope here).
- Plug the shared resolver from #150/#152 into `camera_device_resolver` once it lands.
- **Single camera state machine** (candid review suggestion 9): the manager tracks `CameraHealth`
  (daemon) and `CameraStatus` (GUI) in parallel with different vocabularies (`Dead` vs `Stopped`,
  `Standby` vs `Suspended`). They agree today; derive one from the other to prevent skew.
- **Unplug during standby** (candid review suggestion 10): `Standby` counts as operational, so a
  camera unplugged during standby reports `camera_ready = true` until the next activity. Consider a
  cheap existence check of the device path in `health()` while in standby.
