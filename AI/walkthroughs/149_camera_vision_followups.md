# Walkthrough 149 — Camera / Vision Follow-ups of the P3 Batch

- **Date**: 2026-10-01
- **Issue**: GitHub #287 ("Camera / vision" items; no backlog id) — **Branch**: `fix/p3fu-camera-vision`
- **Matrix criteria**: CVF1–CVF8 (component `camera-vision-followups`)
- **ADR**: 2026-10-01 "Capture Supervisor Keeps the By-Id IR Token on Shared Stems; Camera / Vision
  Follow-ups" (`AI/DECISIONS.md`)

---

## 1. Context & Objectives

Walkthrough 142 (§4, §8) and the #287 review list left four camera / vision follow-ups:

| # | Item | State on `main` (`36577b6`) |
|---|---|---|
| 1 | `soos-admin camera list` and `/etc/soos/daemon.toml` | `--device` / `--sensor-preference` only mirrored the daemon keys; the installer never ran the command |
| 2 | `v4l` 0.14 panics on non-UTF-8 capability strings | Guarded in `SystemV4lDeviceProbe` (admin) only; the daemon probe (`SystemV4lNodeProbe`), the frame-size hints and the capture open path were exposed |
| 3 | Supervisor vs resolver classification for shared by-id stems | The supervisor kept the by-id IR token the resolver withholds; undocumented as a decision |
| 4 | `BiometricEmbedding::to_vec()` | Returned a plain `Vec<f32>` copy of the template |

The fifth #287 camera bullet (#212 / #278 model choice and threshold recalibration) and the
hardware checks are out of scope (owner / hardware).

## 2. Architect Design (Phase 1)

**Blast radius**: `soos-camera-v4l` (`v4l_guard.rs` new, `sensor.rs`, `diagnostics.rs`,
`v4l_impl.rs`, `lib.rs`), `soos-admin-cli` (`daemon_config.rs` new, `args.rs`, `camera.rs`,
`main.rs`, `Cargo.toml` + `toml`), `soos-inference-ort` (`embedding.rs`), `scripts/install.sh`,
`scripts/camera_report.sh` (new), invariants. Consumers of `BiometricEmbedding::to_vec`: none in
production or tests (grep), so the type change needs no caller update. `soos-daemon`,
`soos-enroll` and `soos-gui` pick up the guarded probe through the shared resolver unchanged.

**Item 1** — `soos_admin_cli::daemon_config`:
`DEFAULT_DAEMON_CONFIG_PATH = "/etc/soos/daemon.toml"`, `MAX_DAEMON_CONFIG_BYTES = 1 MiB`,
`DaemonCameraSettings { camera_device: Option<PathBuf>, sensor_preference: Option<SensorPreference>,
sensor_preference_unrecognized: bool }`, `DaemonConfigError { NotFound, NotARegularFile,
Unreadable(io::ErrorKind), TooLarge { limit }, Malformed }`,
`read_daemon_camera_settings(&Path) -> Result<DaemonCameraSettings, DaemonConfigError>`.
The daemon's own `DaemonConfig` is in the `soos-daemon` crate (Tokio, ORT, a binary's deps): not
reusable by the non-biometric CLI, so only the two keys are parsed, with the daemon's field types
(`PathBuf`, `String`) and the shared vocabulary. `camera.rs`: `SettingOrigin { CommandLine,
DaemonConfig, Default }`, `CameraListSettings` + `note()`, `resolve_list_settings(cli_device,
cli_preference, config_path)`. `args.rs`: `CameraListArgs::config` (`--config`, default the path
above) and `camera_list_sensor_preference_given(&ArgMatches)`. The existing contract
`test_cdx_camera_list_args_parse` pins `sensor_preference: SensorPreference` with the clap default
`prefer_ir`, so the field type is kept and "typed on the command line" is read from
`ArgMatches::value_source` instead. Sentinels: a sentinel device from the flag or the file means
auto-detection; an unknown preference keeps `prefer_ir` (as the daemon does) with a note.

**Item 2** — `soos_camera_v4l::v4l_guard`: `V4L_PANIC_MESSAGE`, `guard_v4l_call<T>(FnOnce() -> T)
-> io::Result<T>`, crate-private `query_caps_guarded`, `enum_formats_guarded`, `set_format_guarded`
(the three `v4l` 0.14 calls whose conversions `unwrap`/`expect`). Error taxonomy: daemon probe →
node skipped; admin probe → `probe_failed: io_error`; open path → `CameraError::QueryCapabilities`
/ `SetFormat` → `CameraErrorKind::UnsupportedDevice` → supervisor backoff (instead of `Dead`).

**Item 3** — `pub fn supervisor_sensor_hints(device_path, frame_sizes) -> SensorHints`, the single
hint builder of `open_and_stream`. Security analysis: the scorer's strongest rule is the by-id IR
token, so adding the by-id hint can only move a node to `Infrared`. Applying the resolver's
shared-stem rule in the supervisor would turn exactly the nodes the token marks today from
`Infrared` into `Rgb` (IR PAD policy → RGB policy): a PAD weakening, rejected. Decision: keep,
document (ADR), test the monotonicity.

**Item 4** — `pub fn to_vec(&self) -> Zeroizing<Vec<f32>>`.

**Installer** — `scripts/camera_report.sh [--admin] [--config] [--timeout 1..300, default 15]`,
always exit 0 (2 for a usage error); `install.sh` step 10 calls it on a live install only, after
`COMMITTED=true`, with `|| warn`.

Latency budget: not touched (no auth-path change; the guard adds one `catch_unwind` frame per
metadata ioctl at open time only).

## 3. Plan Evaluation

Condensed in this single-agent run (no separate plan-evaluator report). Checks done against the
code: the CDX10 invariant forbids `set_format` in `diagnostics.rs` and any streaming token in
`crates/admin-cli/src` (respected: the guarded wrappers live in `v4l_guard.rs`); RFX5 exempts the
`self.vector` receiver of `to_vec` (still the receiver); RFX7 only forbids the phrase "same hints
as the resolver" in `v4l_impl.rs` (respected).

## 4. Tester Contract (Phase 2)

| Test | Criterion | Red evidence |
|---|---|---|
| `crates/admin-cli/tests/camera_config_tests.rs` (5 tests) | CVF1–CVF3 | Compile failure: unresolved `soos_admin_cli::daemon_config`, `camera::resolve_list_settings`, `camera::SettingOrigin`, `args::camera_list_sensor_preference_given` |
| `crates/camera-v4l/tests/v4l_panic_guard_tests.rs` (2 tests) | CVF4 | Compile failure: unresolved `soos_camera_v4l::v4l_guard` |
| `crates/camera-v4l/tests/supervisor_classification_tests.rs` (2 tests) | CVF5 | Compile failure: unresolved `soos_camera_v4l::supervisor_sensor_hints` |
| `crates/inference-ort/tests/embedding_copy_zeroize_tests.rs` (1 test) | CVF6 | `E0308 mismatched types: expected Zeroizing<Vec<f32>>, found Vec<f32>` |
| `tests/invariants/src/camera_vision_followups_contract.rs` (5 tests) | CVF4–CVF8 | 5 failed: raw `query_caps(` / `enum_formats(` / `set_format(` calls found, no `supervisor_sensor_hints`, no ADR, `to_vec` returns `Vec<f32>`, `install.sh` does not call `scripts/camera_report.sh`, helper missing |

`test_cvf_guard_catches_the_real_v4l_non_utf8_capability_panic` builds a raw `v4l2_capability`
with `\xff\xfe` in `card` and proves the upstream panic exists (`Capabilities::from`) before
asserting it is converted.

Migrated existing tests: **none**. No existing test file was edited; the only change to
`tests/invariants/src/lib.rs` is the new module declaration.

## 5. Auditor Constraints (Phase 3)

| # | Constraint | Applies to | Verified by |
|---|---|---|---|
| 1 | No `unwrap`/`expect`/`panic!`/indexing in new production code | `v4l_guard.rs`, `daemon_config.rs`, `camera.rs`, `args.rs`, `main.rs` | grep (empty), clippy `-D warnings` |
| 2 | No new `unsafe`; admin-cli keeps `#![forbid(unsafe_code)]` and never calls `v4l` | admin-cli, camera-v4l | grep, CDX10 invariant |
| 3 | Bounded read of the configuration (1 MiB + 1 byte `take`) | `read_daemon_camera_settings` | CVF3 test |
| 4 | Panic payload never propagated (may hold device-chosen bytes); fixed message | `guard_v4l_call` | CVF4 test |
| 5 | Unrecognized `sensor_preference` value is not echoed; every displayed path sanitized (`sanitize_display_text`) | `CameraListSettings::note` | CVF1 test, code review |
| 6 | JSON on stdout keeps its schema (note on stderr) | `main.rs` | existing CDX8 snapshots unchanged and green |
| 7 | Camera report is metadata only, live install only, after commit, bounded, never fatal | `install.sh`, `camera_report.sh` | CVF7/CVF8 invariants |
| 8 | No PAD weakening: supervisor never downgrades Infrared | `supervisor_sensor_hints` | CVF5 tests |
| 9 | Template copies wiped on drop | `BiometricEmbedding::to_vec` | CVF6 tests, RFX5 |
| 10 | Supply chain: `toml` is an existing workspace dependency (no new crate in `Cargo.lock`, only the `soos-admin-cli` dependency list) | `crates/admin-cli/Cargo.toml` | `Cargo.lock` diff (1 line) |

Pre-existing, not changed: `v4l`'s mmap `Stream` drop panics on a failing `VIDIOC_STREAMOFF`
(other than `ENODEV`); that happens after streaming and is still caught by the supervisor's
`catch_unwind` (camera `Dead`, fail-closed). Clearance: CLEARED.

## 6. Implementation (Phase 4)

- `crates/camera-v4l/src/v4l_guard.rs`: the guard and the three wrappers. `sensor.rs`
  (`SystemV4lNodeProbe::probe`, `frame_sizes_at`), `diagnostics.rs` (`system_details`, the outer
  guard now uses `guard_v4l_call`) and `v4l_impl.rs` (`open_and_stream`) call only the wrappers.
- `v4l_impl.rs`: `supervisor_sensor_hints` replaces the inline `SensorHints` literal; its doc
  comment carries the decision.
- `crates/admin-cli/src/daemon_config.rs` + `camera.rs` + `args.rs`; `main.rs` parses through
  `Cli::command().get_matches()` / `Cli::from_arg_matches` so the explicit preference is visible,
  prints `settings.note()` on stderr and resolves with the merged settings.
- `crates/inference-ort/src/embedding.rs`: `to_vec` returns `Zeroizing<Vec<f32>>`.
- `scripts/camera_report.sh` (new, 0755) and `scripts/install.sh` step 10.
- Docs: `Docs/CAMERA_V4L_CRATE.md` (daemon configuration paragraph, panic-guard section,
  supervisor decision, matrix mapping), `Docs/PACKAGING_AND_PROVISIONING.md` §3.2,
  `Docs/DISTRIBUTION_DEPLOYMENT.md`, `Docs/INFERENCE_ORT_CRATE.md`; ADR and matrix rows.

Real-hardware check (owner's laptop camera, Azurewave/IMC 13d3:54bf "Integrated Camera", RGB only,
`uvcvideo`; no frame stored):

- `soos-admin camera list` with no `/etc/soos/daemon.toml`: stderr `camera settings:
  /etc/soos/daemon.toml not found; using the soos-daemon defaults`, `camera_device: auto
  (default)`, `sensor_preference: prefer_ir (default)`; `/dev/video0` candidate (MJPG, YUYV, 1280x720
  down to 320x180, `rgb (colour_formats)`), `/dev/video1` `not_video_capture`; selected the by-id
  link of `video0`, `fallback_first_candidate`, exit 0.
- With a scratch `daemon.toml` (`sensor_preference = "rgb"`): `prefer_rgb (daemon.toml)`,
  `reason: preferred_sensor_matched`; adding `--sensor-preference ir`: `prefer_ir
  (--sensor-preference)`.
- `scripts/camera_report.sh --admin target/debug/soos-admin --config <scratch>`: full report, exit 0.
- `SOOS_HW_TESTS=1 cargo test -p soos-camera-v4l --test hardware_smoke_tests -- --ignored`: 2
  passed (enumeration, resolution and one streamed frame through the guarded open path).

## 7. Candid Review

Not run in this batch (the coordinator runs the independent Layer 2 review). Layer 1
(`./scripts/candid_review.sh`) result is in §8.

## 8. Verification Results

```bash
export CARGO_BUILD_JOBS=4
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -p soos-camera-v4l -p soos-admin-cli \
  -p soos-inference-ort -p soos-invariants -p soos-vision -p soos-daemon -- -D warnings
cargo test --locked --all-features -p soos-camera-v4l -p soos-admin-cli -p soos-inference-ort \
  -p soos-invariants -p soos-daemon -p soos-vision
./scripts/candid_review.sh
```

All green (pass counts in the branch report); `soos-invariants`: 294 passed.

## 9. Known Limitations / Follow-ups

- `soos-enroll` / `soos-gui` keep their own `read_daemon_camera_settings` (`toml::Value`,
  silent defaults) in `crates/enrollment-cli/src/service.rs`; moving both readers to one shared
  function would change enroll's handling of a wrongly typed key and was left out of this batch.
- The supervisor still looks up no by-id alias when `camera_device` is a `/dev/videoN` path (an IR
  node streaming a colour format under an ordinary card name can then be stamped `Rgb`); looking
  up the alias in the open path would only add IR stamps (stricter) and is a candidate follow-up.
- The caught `v4l` panic still runs the process panic hook (the daemon logs its location; the
  admin CLI prints Rust's default panic line on stderr) before the error is reported.
- `soos-admin camera list` on a real RGB+IR module remains an owner hardware check (#287).
